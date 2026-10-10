//! Wi-Fi STA for devboard work: the `wifi-dev` cargo feature.
//!
//! **This is not a path a controller board ever takes.** ADR 0001 is
//! explicit that radio is useless in the fluid and that Ethernet is the
//! only network path, so the shipped image contains none of this: the
//! feature is off by default and the module is not compiled without it.
//!
//! What it is for: a bare ESP32-C6 devboard has no W5500, so without a
//! network nothing north of the hardware layer can be exercised - no
//! HTTPS page, no MQTT session, no Modbus client, no OTA pull. With
//! `--features wifi-dev` the network thread brings up the station
//! interface **instead of** the W5500 and everything above it
//! ([`crate::platform::net`]'s status, hostname, mDNS, SNTP,
//! commit-confirm) runs on the `sta_default` netif unchanged.
//!
//! Credentials are build-time, not configuration: they are a property of
//! the bench, they must not end up in NVS next to a real board's config,
//! and a missing one has to break the build rather than produce an image
//! that silently cannot reach anything.
//!
//! ```sh
//! GRANITE_WIFI_SSID=bench GRANITE_WIFI_PASS=hunter2 \
//!     cargo build --release --features wifi-dev
//! ```
//!
//! Known limits of the dev path, all deliberate:
//!
//! - No NVS partition is handed to `esp_wifi`, so RF calibration is
//!   redone on every boot. The store already owns the one default NVS
//!   partition handle and `EspDefaultNvsPartition::take` refuses a
//!   second one.
//! - A dropped association is retried by the network thread every
//!   [`RETRY`]; there is no backoff and no roaming.
//! - WPA2-PSK or open only; no enterprise, no WPS.

use std::time::Duration;

use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::hal::modem::Modem;
use esp_idf_svc::handle::RawHandle;
use esp_idf_svc::wifi::{AuthMethod, BlockingWifi, ClientConfiguration, Configuration, EspWifi};
use granite_core::config::{IpMode, NetCfg};

/// Network to join. Build-time, from the environment.
///
/// A build with the feature on and this unset fails here, with this
/// message, instead of producing an image that cannot reach anything.
pub const SSID: &str = match option_env!("GRANITE_WIFI_SSID") {
    Some(ssid) => ssid,
    None => panic!(
        "the wifi-dev feature needs the network at build time: \
         GRANITE_WIFI_SSID=<ssid> GRANITE_WIFI_PASS=<pass> cargo build --release --features \
         wifi-dev (see firmware/README.md, \"Testing on a devboard\")"
    ),
};

/// Its PSK. Empty means an open network.
pub const PASS: &str = match option_env!("GRANITE_WIFI_PASS") {
    Some(pass) => pass,
    None => panic!(
        "the wifi-dev feature needs the network at build time: \
         GRANITE_WIFI_SSID=<ssid> GRANITE_WIFI_PASS=<pass> cargo build --release --features \
         wifi-dev (see firmware/README.md, \"Testing on a devboard\")"
    ),
};

/// How long to wait for an address once associated.
const IP_TIMEOUT: Duration = Duration::from_secs(20);

/// How often the network thread retries a lost association.
pub const RETRY: Duration = Duration::from_secs(10);

/// The station interface, as the network thread holds it.
pub type DevWifi = BlockingWifi<EspWifi<'static>>;

/// Bring the station interface up and wait for an address.
///
/// Returns once associated; a missing address is a warning, not an error,
/// because the monitor loop keeps reporting and DHCP keeps retrying, the
/// same as on the Ethernet path.
pub fn bring_up(
    modem: Modem<'static>,
    hostname: &str,
    cfg: &NetCfg,
    sysloop: EspSystemEventLoop,
) -> anyhow::Result<DevWifi> {
    if SSID.is_empty() {
        anyhow::bail!("GRANITE_WIFI_SSID is empty");
    }
    log::warn!(
        "wifi-dev: joining \"{SSID}\" instead of bringing up the W5500. This build is for a \
         devboard, not for a controller board."
    );

    let mut wifi = BlockingWifi::wrap(EspWifi::new(modem, sysloop.clone(), None)?, sysloop)?;
    wifi.set_configuration(&Configuration::Client(ClientConfiguration {
        ssid: SSID
            .try_into()
            .map_err(|_| anyhow::anyhow!("GRANITE_WIFI_SSID is longer than 32 bytes"))?,
        password: PASS
            .try_into()
            .map_err(|_| anyhow::anyhow!("GRANITE_WIFI_PASS is longer than 64 bytes"))?,
        auth_method: if PASS.is_empty() {
            AuthMethod::None
        } else {
            AuthMethod::WPA2Personal
        },
        ..ClientConfiguration::default()
    }))?;
    wifi.start()?;

    // Hostname before the association, so the DHCP request carries it and
    // the lease shows up under the device id like on a board.
    let handle = wifi.wifi().sta_netif().handle();
    if let Err(e) = super::net::set_hostname(handle, hostname) {
        log::warn!("wifi-dev: hostname could not be set: {e}");
    }

    wifi.connect()?;
    log::info!("wifi-dev: associated with \"{SSID}\", waiting for an address");

    // A static config is applied to this netif too, so commit-confirm can
    // be exercised on the bench.
    if cfg.ip_mode == IpMode::Static
        && let Err(e) = super::net::apply_addressing(handle, cfg, hostname)
    {
        log::error!("wifi-dev: the static address could not be applied: {e}");
    }

    if let Err(e) = wifi.ip_wait_while(|| wifi.is_up().map(|up| !up), Some(IP_TIMEOUT)) {
        log::warn!(
            "wifi-dev: no address within {} s: {e}",
            IP_TIMEOUT.as_secs()
        );
    }
    Ok(wifi)
}
