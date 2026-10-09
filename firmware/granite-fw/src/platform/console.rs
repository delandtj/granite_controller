//! Interactive console on USB Serial/JTAG (ADR 0001 component 12).
//!
//! UART0 carries the 1-wire probe bus, so the console lives on the USB-C
//! connector (`CONFIG_ESP_CONSOLE_USB_SERIAL_JTAG=y`). Physical access is
//! full access: the console is not a network path and it asks for no
//! password.
//!
//! Reading is done through the `usb_serial_jtag` driver rather than
//! `stdin`: the VFS path is non-blocking by default and its line-ending
//! translation differs per terminal, which makes a line editor unreliable.
//! `esp_vfs_usb_serial_jtag_use_driver` points the write side at the same
//! driver so log output and console replies cannot interleave mid-byte.
//!
//! The thread holds no locks between commands: everything it needs is
//! behind the shared handles in [`super::Platform`].

use std::sync::Arc;
use std::thread;
use std::time::Duration;

use esp_idf_svc::sys::{
    esp, esp_get_free_heap_size, esp_get_minimum_free_heap_size,
    esp_vfs_usb_serial_jtag_use_driver, usb_serial_jtag_driver_config_t,
    usb_serial_jtag_driver_install, usb_serial_jtag_is_driver_installed,
};

use super::Platform;

/// Receive buffer the driver keeps. One command line is far shorter.
const RX_BUFFER: u32 = 1024;
/// Transmit buffer. Log lines go through here once `use_driver` is on.
const TX_BUFFER: u32 = 2048;
/// Longest accepted line. Longer input is dropped with a message, so a
/// paste of a PEM does not run the heap out.
const MAX_LINE: usize = 512;
/// How long a read waits before the loop checks for a shutdown.
const READ_TICK: Duration = Duration::from_millis(200);
/// Console thread stack: the commands format a few strings, nothing deep.
const STACK: usize = 6144;

const BANNER: &str = "granite console; type `help`";

const HELP: &str = "\
commands:
  id                    device id, mac, firmware, cert fingerprint
  status                net, ota slot and state, uptime, free heap
  net                   network detail
  set-password <pw>     set the admin password (first setup or reset)
  recovery-token        print the per-device recovery token again
  fleet-key             fleet recovery key fingerprint
  factory-reset CONFIRM erase config and secrets, keep factory, reboot
  ota-mark-valid        confirm the running image
  reboot                restart through the planned-reboot path
  log [n]               last n lines of the log ring (default 40)
  help                  this list";

/// Start the console thread.
pub fn start(platform: Arc<Platform>) {
    if let Err(e) = install_driver() {
        log::error!("console: usb serial/jtag driver could not be installed ({e}); no console");
        return;
    }
    let spawned = thread::Builder::new()
        .name("console".into())
        .stack_size(STACK)
        .spawn(move || run(platform));
    if let Err(e) = spawned {
        log::error!("console thread could not start: {e}");
    }
}

fn install_driver() -> Result<(), esp_idf_svc::sys::EspError> {
    if unsafe { usb_serial_jtag_is_driver_installed() } {
        return Ok(());
    }
    let mut conf = usb_serial_jtag_driver_config_t {
        tx_buffer_size: TX_BUFFER,
        rx_buffer_size: RX_BUFFER,
    };
    unsafe { esp!(usb_serial_jtag_driver_install(&mut conf))? };
    // Point printf/stdout at the driver too, so log output and console
    // replies share one path.
    unsafe { esp_vfs_usb_serial_jtag_use_driver() };
    Ok(())
}

fn run(platform: Arc<Platform>) {
    say(BANNER);
    say("");
    prompt();

    let mut line = String::new();
    let mut buf = [0u8; 64];
    loop {
        let n = read(&mut buf);
        for &byte in &buf[..n] {
            match byte {
                b'\r' | b'\n' => {
                    println!();
                    let input = line.trim().to_string();
                    line.clear();
                    if !input.is_empty() {
                        handle(&platform, &input);
                    }
                    prompt();
                }
                0x08 | 0x7f => {
                    if line.pop().is_some() {
                        // Erase the character on the terminal too.
                        print!("\x08 \x08");
                        flush();
                    }
                }
                0x03 => {
                    line.clear();
                    say("");
                    prompt();
                }
                // Printable, and the line still has room. A longer paste
                // than MAX_LINE is dropped silently rather than allowed to
                // grow the heap.
                b if (b.is_ascii_graphic() || b == b' ') && line.len() < MAX_LINE => {
                    line.push(b as char);
                    print!("{}", b as char);
                    flush();
                }
                _ => {}
            }
        }
    }
}

fn read(buf: &mut [u8]) -> usize {
    let ticks = (READ_TICK.as_millis() as u32).div_ceil(10).max(1);
    let n = unsafe {
        esp_idf_svc::sys::usb_serial_jtag_read_bytes(
            buf.as_mut_ptr().cast(),
            buf.len() as u32,
            ticks,
        )
    };
    if n <= 0 { 0 } else { n as usize }
}

/// One console line. Only `\n`: the USB Serial/JTAG VFS turns it into
/// CRLF, and writing `\r\n` here would put two CRs on the wire.
fn say(text: &str) {
    println!("{text}");
    flush();
}

fn prompt() {
    print!("granite> ");
    flush();
}

fn flush() {
    use std::io::Write as _;
    let _ = std::io::stdout().flush();
}

fn handle(platform: &Platform, input: &str) {
    let mut parts = input.split_whitespace();
    let Some(cmd) = parts.next() else {
        return;
    };
    let rest = input[cmd.len()..].trim();

    match cmd {
        "help" | "?" => say(HELP),
        "id" => cmd_id(platform),
        "status" => cmd_status(platform),
        "net" => cmd_net(platform),
        "recovery-token" => cmd_recovery_token(platform),
        "fleet-key" => cmd_fleet_key(platform),
        "set-password" => cmd_set_password(platform, rest),
        "factory-reset" => cmd_factory_reset(platform, rest),
        "ota-mark-valid" => match super::ota::mark_valid() {
            Ok(()) => say("ok: running image marked valid"),
            Err(e) => say(&format!("err: {e}")),
        },
        "reboot" => {
            say("ok: rebooting");
            thread::sleep(Duration::from_millis(200));
            super::planned_reboot("console");
        }
        "log" => {
            let n: usize = rest.parse().unwrap_or(40);
            for line in super::logring::tail(n) {
                say(&line);
            }
            say(&format!("-- {} lines kept", super::logring::len()));
        }
        other => say(&format!("err: unknown command `{other}`; try `help`")),
    }
}

fn cmd_id(platform: &Platform) {
    let id = &platform.identity;
    say(&format!("device    {}", id.device_id));
    say(&format!(
        "mac       {}",
        super::identity::mac_string(&id.mac)
    ));
    say(&format!("fw        {}", super::FW_VERSION));
    say(&format!("built     {}", super::identity::BUILD_X509_TIME));
    say(&format!("cert      sha256 {}", id.cert_sha256));
}

fn cmd_status(platform: &Platform) {
    let net = platform.net.status();
    say(&format!(
        "net       link {} {} {} gw {}",
        if net.link { "up" } else { "down" },
        net.mode.as_str(),
        if net.ip.is_empty() { "-" } else { &net.ip },
        if net.gateway.is_empty() {
            "-"
        } else {
            &net.gateway
        }
    ));
    if let Some(err) = net.error.as_ref() {
        say(&format!("net error {err}"));
    }
    match super::ota::info() {
        Ok(info) => say(&format!(
            "ota       running {} ({}), next {}, state {}",
            info.running,
            info.state,
            info.next,
            super::ota::ota_state()
        )),
        Err(e) => say(&format!("ota       unreadable: {e}")),
    }
    say(&format!("signing   {}", super::signing_summary()));
    let uptime = super::now_ms() / 1000;
    say(&format!(
        "uptime    {} d {:02}:{:02}:{:02}",
        uptime / 86_400,
        (uptime % 86_400) / 3600,
        (uptime % 3600) / 60,
        uptime % 60
    ));
    let free = unsafe { esp_get_free_heap_size() };
    let low = unsafe { esp_get_minimum_free_heap_size() };
    say(&format!("heap      {free} free, {low} low water"));
    say(&format!("boot      {}", super::boot_reason()));
    say(&format!("time      {}", super::wall_clock_string()));
}

fn cmd_net(platform: &Platform) {
    let s = platform.net.status();
    say(&format!("link      {}", if s.link { "up" } else { "down" }));
    say(&format!("mode      {}", s.mode.as_str()));
    say(&format!("ip        {}", or_dash(&s.ip)));
    say(&format!("netmask   {}", or_dash(&s.netmask)));
    say(&format!("gateway   {}", or_dash(&s.gateway)));
    say(&format!("ipv6-ll   {}", or_dash(&s.ipv6_linklocal)));
    say(&format!("hostname  {}", or_dash(&s.hostname)));
    say(&format!("mdns      {}", if s.mdns { "on" } else { "off" }));
    say(&format!(
        "time      {}",
        if s.time_synced { "sntp" } else { "build stamp" }
    ));
    if s.deadman_fired {
        say("deadman   fired: dhcp was started as a fallback");
    }
    if s.confirm_pending {
        say(&format!(
            "confirm   pending, {} s left before a revert reboot",
            s.confirm_left_s
        ));
    }
}

fn or_dash(s: &str) -> &str {
    if s.is_empty() { "-" } else { s }
}

fn cmd_recovery_token(platform: &Platform) {
    say(&format!("recovery  {}", platform.identity.recovery_token));
    say("keep it with the MAC; POST /recover with it does a factory reset");
}

fn cmd_fleet_key(platform: &Platform) {
    match platform.identity.fleet_key_fingerprint() {
        Some(fp) => say(&format!("fleet-key sha256 {fp}")),
        None => say("fleet-key none configured"),
    }
}

fn cmd_set_password(platform: &Platform, rest: &str) {
    let pw = rest.trim();
    if pw.len() < 8 {
        say("err: the admin password must be at least 8 characters");
        return;
    }
    match platform.set_admin_password(pw) {
        Ok(()) => say("ok: admin password set"),
        Err(e) => say(&format!("err: {e}")),
    }
}

fn cmd_factory_reset(platform: &Platform, rest: &str) {
    if rest.trim() != "CONFIRM" {
        say("err: say `factory-reset CONFIRM`; this erases config and secrets");
        return;
    }
    say("ok: erasing everything but the factory namespace, then rebooting");
    thread::sleep(Duration::from_millis(300));
    match platform.store.lock() {
        Ok(mut store) => store.factory_reset(),
        Err(_) => {
            say("err: the store is locked; rebooting without erasing");
            super::planned_reboot("factory_reset_locked")
        }
    }
}
