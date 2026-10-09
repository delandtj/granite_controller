//! `granite-sim`: run the Granite controller firmware logic on the host,
//! and the host-side tools that go with it.
//!
//!   granite-sim serve [--port 8443] [--scenario file.toml]
//!   granite-sim recover --key ~/.config/granite/fleet-recovery.key <host>
//!   granite-sim keygen
//!   granite-sim modbus-map > firmware/docs/modbus_map.md

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use clap::{Parser, Subcommand};
use granite_sim::http::ServeOptions;
use granite_sim::{Scenario, Sim, http, keys};

#[derive(Parser)]
#[command(
    name = "granite-sim",
    about = "Granite controller simulator and fleet recovery tools",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run the simulated controller and serve the setup page.
    Serve {
        /// Port to listen on.
        #[arg(long, default_value_t = 8443)]
        port: u16,
        /// Address to bind.
        #[arg(long, default_value = "127.0.0.1")]
        bind: String,
        /// Scenario file describing the fake hardware.
        #[arg(long)]
        scenario: Option<PathBuf>,
        /// Serve plain HTTP instead of HTTPS with a self-signed cert.
        #[arg(long)]
        no_tls: bool,
        /// Serve the page from this directory instead of the embedded
        /// copy, so edits show up on reload.
        #[arg(long)]
        assets: Option<PathBuf>,
    },
    /// Factory reset a controller with the fleet recovery key.
    Recover {
        /// Host name or address of the controller.
        host: String,
        /// Private key file.
        #[arg(long, default_value_os_t = keys::default_key_path())]
        key: PathBuf,
        /// Talk plain HTTP (a simulator started with --no-tls).
        #[arg(long)]
        no_tls: bool,
        /// Port, if it is not the default for the scheme.
        #[arg(long)]
        port: Option<u16>,
    },
    /// Write a new fleet recovery key pair.
    Keygen {
        /// Where the private key goes.
        #[arg(long, default_value_os_t = keys::default_key_path())]
        key: PathBuf,
    },
    /// Print the Modbus register map as Markdown.
    ModbusMap,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Serve {
            port,
            bind,
            scenario,
            no_tls,
            assets,
        } => serve(port, bind, scenario, !no_tls, assets),
        Commands::Recover {
            host,
            key,
            no_tls,
            port,
        } => recover(&host, &key, !no_tls, port),
        Commands::Keygen { key } => {
            let public_pem = keys::write_key_pair(&key)?;
            eprintln!("private key written to {} (mode 0600)", key.display());
            eprintln!("put the public key on the Security page of every board:");
            print!("{public_pem}");
            Ok(())
        }
        Commands::ModbusMap => {
            print!("{}", granite_core::modbus_map::render_markdown());
            Ok(())
        }
    }
}

fn serve(
    port: u16,
    bind: String,
    scenario_path: Option<PathBuf>,
    tls: bool,
    assets: Option<PathBuf>,
) -> anyhow::Result<()> {
    let scenario = match &scenario_path {
        Some(path) => Scenario::load(path)?,
        None => {
            let mut s = Scenario::default();
            s.normalise();
            s
        }
    };
    let certified = rcgen::generate_simple_self_signed(vec![
        String::from("localhost"),
        scenario.device_id.clone(),
        String::from("127.0.0.1"),
    ])?;
    let cert_pem = certified.cert.pem();
    let key_pem = certified.signing_key.serialize_pem();

    let sim = Arc::new(Mutex::new(Sim::new(scenario, &cert_pem, &key_pem)));
    {
        let sim = sim.clone();
        std::thread::Builder::new()
            .name(String::from("sim-tick"))
            .spawn(move || {
                loop {
                    std::thread::sleep(std::time::Duration::from_millis(
                        granite_sim::sim::TICK_MS,
                    ));
                    if let Ok(mut sim) = sim.lock() {
                        sim.tick();
                    }
                }
            })?;
    }
    {
        let sim = sim.lock().expect("lock");
        println!("device   {}", sim.identity.device_id);
        println!("mac      {}", sim.identity.mac);
        println!("cert     sha256 {}", granite_core::api::Identity::cert_sha256(&sim.identity));
        println!("recovery {} (shown once on the page)", sim.identity.recovery_token);
        if let Some(path) = &scenario_path {
            println!("scenario {}", path.display());
        }
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(http::serve(
        sim,
        ServeOptions {
            port,
            bind,
            tls,
            assets_dir: assets,
        },
    ))
}

fn recover(host: &str, key_path: &std::path::Path, tls: bool, port: Option<u16>) -> anyhow::Result<()> {
    let key = keys::load_signing_key(key_path)?;
    let scheme = if tls { "https" } else { "http" };
    let authority = match port {
        Some(p) => format!("{host}:{p}"),
        None => String::from(host),
    };
    let base = format!("{scheme}://{authority}");

    // The device certificate is self-signed by design (ADR component 8),
    // and the recovery request is authenticated by the fleet signature,
    // not by TLS. So the certificate is reported, not trusted.
    let client = reqwest::blocking::Client::builder()
        .danger_accept_invalid_certs(true)
        .timeout(std::time::Duration::from_secs(10))
        .build()?;

    let id: serde_json::Value = client.get(format!("{base}/id")).send()?.json()?;
    let device = id
        .get("device")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("/id did not report a device id"))?;
    let nonce = id
        .get("nonce")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("/id did not report a recovery nonce"))?;
    eprintln!(
        "device {device} fw {} cert sha256 {}",
        id.get("fw").and_then(|v| v.as_str()).unwrap_or("?"),
        id.get("cert_sha256").and_then(|v| v.as_str()).unwrap_or("?")
    );

    let sig = keys::sign_recovery(&key, device, nonce);
    let body = serde_json::json!({"device": device, "nonce": nonce, "sig": sig});
    let response = client
        .post(format!("{base}/recover"))
        .json(&body)
        .send()?;
    let status = response.status();
    let text = response.text().unwrap_or_default();
    if !status.is_success() {
        anyhow::bail!("recovery refused with {status}: {text}");
    }
    println!("{text}");
    eprintln!("{device} is doing a factory reset; it comes back in first-setup mode");
    Ok(())
}
