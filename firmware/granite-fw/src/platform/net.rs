//! Network (ADR 0001 component 6): W5500 Ethernet, addressing,
//! commit-confirm, mDNS, SNTP, and a [`NetStatus`] every other task reads.
//!
//! The whole module is one thread that owns the Ethernet object. It brings
//! the link up, then loops once a second: refresh [`NetStatus`], watch the
//! commit-confirm deadline, run the static-config dead-man. Commands
//! ([`Net::apply`], [`Net::confirm`]) arrive on a channel, so nothing
//! outside this thread ever touches the netif and none of the ESP-IDF
//! handles have to be `Sync`.
//!
//! What lwIP does by itself and what this module drives:
//!
//! - **AutoIP** is lwIP's, not ours. `CONFIG_LWIP_AUTOIP=y` plus
//!   `CONFIG_LWIP_AUTOIP_TRIES=4` makes lwIP start IPv4 link-local
//!   alongside DHCP after four failed DISCOVERs, which with lwIP's
//!   doubling backoff (2 + 4 + 8 + 16 s) is the 30 s the ADR asks for, and
//!   DHCP keeps retrying underneath. This module only *reports* the state
//!   by recognising a 169.254/16 address.
//! - **SNTP** is `esp_netif_sntp_*` rather than esp-idf-svc's `EspSntp`,
//!   because only the former can take the server from DHCP option 42
//!   (`CONFIG_LWIP_DHCP_GET_NTP_SRV=y`). The clock is set to the firmware
//!   build time at boot so TLS date checks pass before the first sync.
//! - **Addressing** is raw `esp_netif_*`: esp-idf-svc has no way to switch
//!   a live netif between DHCP and a static address, which commit-confirm
//!   needs.
//!
//! With the `wifi-dev` feature the thread brings up the Wi-Fi station
//! interface **instead of** the W5500 ([`super::wifi_dev`]) and
//! everything below this line - status, hostname, mDNS, SNTP,
//! commit-confirm, the dead-man - runs on the `sta_default` netif
//! unchanged. That feature is a devboard tool and is off in any image
//! that goes on a board: ADR 0001 keeps the radio dark.

use std::ffi::CString;
use std::net::Ipv4Addr;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::Duration;

use esp_idf_svc::eth::{BlockingEth, EspEth, EthDriver, SpiEth, SpiEthChipset};
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::handle::RawHandle;
use esp_idf_svc::hal::gpio::{Gpio18, Gpio19, Gpio20, Gpio21, Gpio22, Gpio23};
use esp_idf_svc::hal::spi::{SPI2, SpiDriver, config::DriverConfig};
use esp_idf_svc::hal::units::Hertz;
use esp_idf_svc::ipv4;
use esp_idf_svc::netif::{EspNetif, NetifConfiguration};
use esp_idf_svc::ping::{Configuration as PingConfiguration, EspPing};
use esp_idf_svc::sys::{
    ESP_IPADDR_TYPE_V4, esp, esp_ip4_addr_t, esp_ip6_addr_t, esp_netif_create_ip6_linklocal,
    esp_netif_dhcpc_start, esp_netif_dhcpc_stop, esp_netif_dns_info_t,
    esp_netif_dns_type_t_ESP_NETIF_DNS_BACKUP, esp_netif_dns_type_t_ESP_NETIF_DNS_MAIN,
    esp_netif_get_ip6_linklocal, esp_netif_ip_info_t, esp_netif_set_dns_info,
    esp_netif_set_hostname, esp_netif_set_ip_info, esp_netif_t, esp_sntp_config_t,
    esp_netif_sntp_deinit, esp_netif_sntp_init, ip_event_t_IP_EVENT_ETH_GOT_IP, settimeofday,
    sntp_get_sync_status, sntp_sync_status_t_SNTP_SYNC_STATUS_COMPLETED, timeval,
};
use granite_core::config::{IpMode, NetCfg};

/// W5500 SPI clock. The datasheet allows 33 MHz; start conservative.
const SPI_BAUDRATE: Hertz = Hertz(20_000_000);
/// How long to wait for the driver to report the MAC/PHY started.
const START_TIMEOUT: Duration = Duration::from_secs(5);
/// How long to wait for the PHY to report link up during bring-up. After
/// this the thread keeps watching; a late cable is not an error.
const LINK_TIMEOUT: Duration = Duration::from_secs(10);
/// How long to wait for a first address once the link is up.
const IP_TIMEOUT: Duration = Duration::from_secs(35);
/// Monitor tick.
const TICK: Duration = Duration::from_secs(1);
/// How often the dead-man probes the gateway.
const DEADMAN_PROBE: Duration = Duration::from_secs(60);
/// mDNS service ports.
const HTTPS_PORT: u16 = 443;

type GraniteEth = BlockingEth<EspEth<'static, SpiEth<&'static SpiDriver<'static>>>>;

/// SPI and control pins of the W5500 (docs/controller.md, "Ethernet").
pub struct EthPins {
    /// SPI clock.
    pub sclk: Gpio19<'static>,
    /// Controller out, W5500 in.
    pub mosi: Gpio20<'static>,
    /// W5500 out, controller in.
    pub miso: Gpio21<'static>,
    /// Chip select, driven by the Ethernet driver.
    pub cs: Gpio18<'static>,
    /// W5500 interrupt.
    pub int: Gpio22<'static>,
    /// W5500 reset.
    pub rst: Gpio23<'static>,
}

/// The network hardware the thread is handed.
///
/// One struct rather than loose arguments because the `wifi-dev` build
/// needs the radio as well, and a call site that reads as a pin map is
/// the point of [`EthPins`]. The W5500 tokens are carried in both builds:
/// they are peripheral tokens, not drivers, and keeping them means
/// `main.rs` has one pin map for both.
pub struct NetHw {
    /// SPI2, the W5500's bus.
    pub spi: SPI2<'static>,
    /// The W5500's pins.
    pub pins: EthPins,
    /// The radio. Only in a `wifi-dev` build, which uses it instead of
    /// the W5500.
    #[cfg(feature = "wifi-dev")]
    pub modem: esp_idf_svc::hal::modem::Modem<'static>,
}

/// How the IPv4 address was obtained.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AddrMode {
    /// No address yet.
    #[default]
    None,
    /// From a DHCP lease.
    Dhcp,
    /// From the config.
    Static,
    /// IPv4 link-local, lwIP's AutoIP, while DHCP keeps retrying.
    AutoIp,
}

impl AddrMode {
    /// Lowercase wire name.
    pub const fn as_str(self) -> &'static str {
        match self {
            AddrMode::None => "none",
            AddrMode::Dhcp => "dhcp",
            AddrMode::Static => "static",
            AddrMode::AutoIp => "autoip",
        }
    }
}

/// What the rest of the firmware may know about the network.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct NetStatus {
    /// PHY link.
    pub link: bool,
    /// How the address was obtained.
    pub mode: AddrMode,
    /// IPv4 address, empty when there is none.
    pub ip: String,
    /// IPv4 netmask.
    pub netmask: String,
    /// Default gateway.
    pub gateway: String,
    /// IPv6 link-local address, empty until it is up.
    pub ipv6_linklocal: String,
    /// Hostname handed to DHCP and used for mDNS.
    pub hostname: String,
    /// mDNS is advertising.
    pub mdns: bool,
    /// SNTP has synced at least once.
    pub time_synced: bool,
    /// The static-config dead-man has fired and DHCP was asked for.
    pub deadman_fired: bool,
    /// A network change is applied but not yet confirmed.
    pub confirm_pending: bool,
    /// Seconds left to confirm.
    pub confirm_left_s: u32,
    /// Why Ethernet is not up, when it is not.
    pub error: Option<String>,
}

/// Last time a server accepted a connection, monotonic seconds since boot.
/// Seconds, not milliseconds: riscv32imac has no 64-bit atomics. The HTTP and
/// Modbus servers call [`note_connection`]; the dead-man reads it.
static LAST_CONN_S: AtomicU32 = AtomicU32::new(0);
/// Set once SNTP reports a sync.
static TIME_SYNCED: AtomicBool = AtomicBool::new(false);

/// Record that a server accepted a connection. Part of the static-config
/// dead-man: a reachable board is one someone can connect to.
pub fn note_connection() {
    LAST_CONN_S.store((super::now_ms() / 1000) as u32, Ordering::Relaxed);
}

/// True once the clock came from SNTP rather than the build stamp.
pub fn time_synced() -> bool {
    TIME_SYNCED.load(Ordering::Relaxed)
}

/// Commands the network thread accepts.
enum NetCmd {
    /// Apply a net config live and start the commit-confirm timer.
    Apply { cfg: Box<NetCfg>, confirm_s: u32 },
    /// The client reached us through the new config.
    Confirm,
    /// Drop the staged config and go back to the stored one without a
    /// reboot (used when a confirm window is cancelled explicitly).
    Revert,
}

/// Handle on the network thread.
pub struct Net {
    status: Arc<RwLock<NetStatus>>,
    tx: Sender<NetCmd>,
}

impl Net {
    /// Current status.
    pub fn status(&self) -> NetStatus {
        self.status
            .read()
            .map(|s| s.clone())
            .unwrap_or_else(|_| NetStatus::default())
    }

    /// The shared status, for a task that wants to poll it cheaply.
    pub fn status_handle(&self) -> Arc<RwLock<NetStatus>> {
        Arc::clone(&self.status)
    }

    /// Apply a staged net config live and start `t_confirm`.
    ///
    /// The caller must already have written the new section to the staging
    /// slot (`Store::stage_section`) and left the old one in `cfg`, so a
    /// reboot reverts. On timeout this module reboots.
    pub fn apply(&self, cfg: &NetCfg, confirm_s: u32) {
        let _ = self.tx.send(NetCmd::Apply {
            cfg: Box::new(cfg.clone()),
            confirm_s,
        });
    }

    /// Confirm the applied change. The caller promotes the staging slot.
    pub fn confirm(&self) {
        let _ = self.tx.send(NetCmd::Confirm);
    }

    /// Cancel a pending confirm window without rebooting.
    pub fn revert(&self) {
        let _ = self.tx.send(NetCmd::Revert);
    }
}

/// Start the network thread. Returns immediately: bring-up, which can take
/// half a minute on a DHCP network and forever without a cable, happens in
/// the thread.
///
/// A missing or dead W5500 is a logged error and a `NetStatus` with
/// `error` set, never a panic: the USB console must stay the way in.
#[allow(clippy::too_many_arguments)]
pub fn init(
    hw: NetHw,
    mac: [u8; 6],
    device_id: String,
    cfg: NetCfg,
    sysloop: EspSystemEventLoop,
) -> Net {
    let status = Arc::new(RwLock::new(NetStatus {
        hostname: hostname_of(&cfg, &device_id),
        ..NetStatus::default()
    }));
    let (tx, rx) = channel();
    let thread_status = Arc::clone(&status);
    let spawned = thread::Builder::new()
        .name("net".into())
        .stack_size(8192)
        .spawn(move || {
            run(hw, mac, device_id, cfg, sysloop, thread_status, rx);
        });
    if let Err(e) = spawned {
        log::error!("network thread could not start: {e}");
        if let Ok(mut s) = status.write() {
            s.error = Some(format!("thread: {e}"));
        }
    }
    Net { status, tx }
}

/// Hostname: the configured one, else the device id.
pub fn hostname_of(cfg: &NetCfg, device_id: &str) -> String {
    let h = cfg.hostname.trim();
    if h.is_empty() {
        device_id.to_string()
    } else {
        h.to_string()
    }
}

// ---------------------------------------------------------------------------
// The thread
// ---------------------------------------------------------------------------

struct Confirm {
    deadline_ms: u64,
}

#[allow(clippy::too_many_arguments)]
fn run(
    hw: NetHw,
    mac: [u8; 6],
    device_id: String,
    mut cfg: NetCfg,
    sysloop: EspSystemEventLoop,
    status: Arc<RwLock<NetStatus>>,
    rx: Receiver<NetCmd>,
) {
    // The clock starts at the build time so TLS not-before checks pass and
    // timestamps are at least plausible until SNTP lands (ADR component 6).
    set_clock_to_build_time();

    let hostname = hostname_of(&cfg, &device_id);
    #[allow(unused_mut)]
    let mut link = match bring_up_link(hw, &mac, &hostname, &cfg, sysloop) {
        Ok(link) => link,
        Err(e) => {
            log::error!("{LINK_KIND} bring-up failed: {e:#}");
            if let Ok(mut s) = status.write() {
                s.error = Some(format!("{e:#}"));
            }
            Link::Down
        }
    };

    let netif_handle = link
        .netif()
        .map(|netif| netif.handle())
        .unwrap_or(core::ptr::null_mut());
    let netif_index = link.netif().map(|netif| netif.get_index());

    let mut sntp_started = false;
    let mut ip6_done = false;
    let mut mdns = None;
    let mut confirm: Option<Confirm> = None;
    let mut last_probe_ms = 0u64;
    let mut last_alive_ms = super::now_ms();
    let mut deadman_fired = false;
    #[cfg(feature = "wifi-dev")]
    let mut last_retry_ms = super::now_ms();

    loop {
        // -- commands ----------------------------------------------------
        match rx.recv_timeout(TICK) {
            Ok(NetCmd::Apply {
                cfg: new_cfg,
                confirm_s,
            }) => {
                cfg = *new_cfg;
                let hostname = hostname_of(&cfg, &device_id);
                if !netif_handle.is_null()
                    && let Err(e) = apply_addressing(netif_handle, &cfg, &hostname)
                {
                    log::error!("applying the staged net config failed: {e}");
                }
                let window = if confirm_s == 0 { 300 } else { confirm_s };
                confirm = Some(Confirm {
                    deadline_ms: super::now_ms() + u64::from(window) * 1000,
                });
                log::warn!(
                    "net config applied, {window} s to confirm through the new address or this \
                     board reboots into the previous one"
                );
                // The hostname and the mDNS name may have changed.
                mdns = None;
                ip6_done = false;
            }
            Ok(NetCmd::Confirm) => {
                if confirm.take().is_some() {
                    log::warn!("net config confirmed");
                } else {
                    log::info!("net confirm with nothing pending, ignored");
                }
            }
            Ok(NetCmd::Revert) => {
                if confirm.take().is_some() {
                    log::warn!("net config confirm window cancelled; rebooting into the stored config");
                    super::planned_reboot("net_revert");
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                // Nobody can talk to us any more, but the link still has to
                // be watched; keep ticking.
                thread::sleep(TICK);
            }
        }

        // -- the dev radio, if this is a wifi-dev build ------------------
        // A dropped association otherwise needs a reboot, which on a
        // bench board in the middle of a test is the wrong answer.
        #[cfg(feature = "wifi-dev")]
        if !link.connected()
            && super::now_ms().saturating_sub(last_retry_ms)
                >= super::wifi_dev::RETRY.as_millis() as u64
        {
            last_retry_ms = super::now_ms();
            link.retry();
        }

        // -- status refresh ----------------------------------------------
        let link_up = link.connected();
        let ip_info = link.netif().and_then(|netif| netif.get_ip_info().ok());
        let ip = ip_info.map(|i| i.ip).unwrap_or(Ipv4Addr::UNSPECIFIED);
        let has_ip = ip != ipv4::Ipv4Addr::new(0, 0, 0, 0);
        let mode = if !has_ip {
            AddrMode::None
        } else if is_link_local(ip) {
            AddrMode::AutoIp
        } else if cfg.ip_mode == IpMode::Static && !deadman_fired {
            AddrMode::Static
        } else {
            AddrMode::Dhcp
        };

        // IPv6 link-local once the interface is up (ADR: always on).
        if link_up && has_ip && !ip6_done && !netif_handle.is_null() {
            match unsafe { esp!(esp_netif_create_ip6_linklocal(netif_handle)) } {
                Ok(()) => ip6_done = true,
                Err(e) => log::debug!("ipv6 link-local not ready yet: {e}"),
            }
        }

        // SNTP once there is an address to reach a server from.
        if link_up && has_ip && !sntp_started {
            match start_sntp(&cfg) {
                Ok(()) => sntp_started = true,
                Err(e) => log::warn!("sntp could not start: {e}"),
            }
        }
        if sntp_started
            && !TIME_SYNCED.load(Ordering::Relaxed)
            && unsafe { sntp_get_sync_status() } == sntp_sync_status_t_SNTP_SYNC_STATUS_COMPLETED
        {
            TIME_SYNCED.store(true, Ordering::Relaxed);
            log::info!("clock synced from sntp");
        }

        // mDNS once there is an address to answer on.
        if link_up && has_ip && cfg.mdns && mdns.is_none() {
            match start_mdns(&hostname_of(&cfg, &device_id), &device_id) {
                Ok(m) => mdns = Some(m),
                Err(e) => log::warn!("mdns could not start: {e}"),
            }
        }

        if let Ok(mut s) = status.write() {
            s.link = link_up;
            s.mode = mode;
            s.ip = if has_ip { ip.to_string() } else { String::new() };
            s.netmask = String::new();
            s.gateway = String::new();
            if let Some(netif) = link.netif()
                && let Ok(info) = netif.get_ip_info()
            {
                s.netmask = info.subnet.mask.to_string();
                s.gateway = info.subnet.gateway.to_string();
            }
            s.hostname = hostname_of(&cfg, &device_id);
            s.mdns = mdns.is_some();
            s.time_synced = TIME_SYNCED.load(Ordering::Relaxed);
            s.deadman_fired = deadman_fired;
            s.ipv6_linklocal = if netif_handle.is_null() {
                String::new()
            } else {
                ip6_linklocal(netif_handle)
            };
            match confirm.as_ref() {
                Some(c) => {
                    s.confirm_pending = true;
                    s.confirm_left_s =
                        (c.deadline_ms.saturating_sub(super::now_ms()) / 1000) as u32;
                }
                None => {
                    s.confirm_pending = false;
                    s.confirm_left_s = 0;
                }
            }
        }

        // -- commit-confirm deadline -------------------------------------
        if let Some(c) = confirm.as_ref()
            && super::now_ms() >= c.deadline_ms
        {
            log::error!(
                "net config was not confirmed in time; rebooting into the stored configuration"
            );
            super::planned_reboot("net_confirm_timeout");
        }

        // -- static-config dead-man --------------------------------------
        // ADR: static address, no gateway ARP reply and no accepted TCP
        // connection for t_deadman (default 1 h, 0 = off) -> start DHCP
        // alongside. An ICMP echo to the gateway stands in for the ARP
        // reply: it is strictly stronger evidence and it is reachable from
        // Rust without an lwIP-internal call.
        if cfg.t_deadman_s > 0
            && cfg.ip_mode == IpMode::Static
            && link_up
            && has_ip
            && !deadman_fired
        {
            let now = super::now_ms();
            let last_conn_ms = u64::from(LAST_CONN_S.load(Ordering::Relaxed)) * 1000;
            if last_conn_ms > 0
                && now.saturating_sub(last_conn_ms) < DEADMAN_PROBE.as_millis() as u64
            {
                last_alive_ms = now;
            } else if now.saturating_sub(last_probe_ms) >= DEADMAN_PROBE.as_millis() as u64 {
                last_probe_ms = now;
                if let (Some(index), Some(gw)) = (netif_index, gateway_of(&cfg)) {
                    let mut ping = EspPing::new(index);
                    let conf = PingConfiguration {
                        count: 2,
                        timeout: Duration::from_secs(1),
                        ..PingConfiguration::default()
                    };
                    match ping.ping(gw, &conf) {
                        Ok(summary) if summary.received > 0 => last_alive_ms = now,
                        Ok(_) => log::warn!("dead-man: gateway {gw} did not answer"),
                        Err(e) => log::warn!("dead-man: gateway probe failed: {e}"),
                    }
                }
            }
            if now.saturating_sub(last_alive_ms) >= u64::from(cfg.t_deadman_s) * 1000 {
                deadman_fired = true;
                log::error!(
                    "dead-man: {} s without a gateway answer or an accepted connection on a \
                     static address; asking for DHCP",
                    cfg.t_deadman_s
                );
                // TODO(ADR 0001 6): the ADR wants DHCP started "alongside"
                // without dropping the static address. esp_netif has no
                // such mode - esp_netif_dhcpc_start clears the address
                // first - so this starts plain DHCP and reports
                // deadman_fired in the status. Keeping both needs a second
                // netif on the same driver, or an lwIP-level second
                // address; neither is wired up yet.
                if !netif_handle.is_null()
                    && let Err(e) = unsafe { esp!(esp_netif_dhcpc_start(netif_handle)) }
                {
                    log::error!("dead-man: dhcp could not start: {e}");
                }
            }
        }
    }
}

fn is_link_local(ip: ipv4::Ipv4Addr) -> bool {
    let o = ip.octets();
    o[0] == 169 && o[1] == 254
}

fn gateway_of(cfg: &NetCfg) -> Option<ipv4::Ipv4Addr> {
    cfg.gateway.trim().parse::<Ipv4Addr>().ok()
}

fn ip6_linklocal(handle: *mut esp_netif_t) -> String {
    let mut addr: esp_ip6_addr_t = unsafe { core::mem::zeroed() };
    if unsafe { esp_netif_get_ip6_linklocal(handle, &mut addr) } != 0 {
        return String::new();
    }
    let mut octets = [0u8; 16];
    for (i, word) in addr.addr.iter().enumerate() {
        octets[i * 4..i * 4 + 4].copy_from_slice(&word.to_le_bytes());
    }
    std::net::Ipv6Addr::from(octets).to_string()
}

// ---------------------------------------------------------------------------
// Bring-up
// ---------------------------------------------------------------------------

/// What this build calls a network, for the log line that says it failed.
#[cfg(not(feature = "wifi-dev"))]
const LINK_KIND: &str = "ethernet";
#[cfg(feature = "wifi-dev")]
const LINK_KIND: &str = "wifi-dev";

/// The interface the thread owns.
///
/// Everything above the bring-up works through [`Link::connected`] and
/// [`Link::netif`], so the W5500 and the devboard radio are the same
/// thing to the monitor loop, and `Down` (no W5500, no AP) is a state
/// that keeps the loop running rather than a reason to stop.
enum Link {
    /// The W5500. Still compiled in a `wifi-dev` build, just never
    /// constructed there.
    #[cfg_attr(feature = "wifi-dev", allow(dead_code))]
    Eth(GraniteEth),
    /// The devboard radio (`wifi-dev`).
    #[cfg(feature = "wifi-dev")]
    Wifi(Box<super::wifi_dev::DevWifi>),
    /// Nothing came up.
    Down,
}

impl Link {
    /// Link (or association) up.
    fn connected(&self) -> bool {
        match self {
            Link::Eth(eth) => eth.is_connected().unwrap_or(false),
            #[cfg(feature = "wifi-dev")]
            Link::Wifi(wifi) => wifi.is_connected().unwrap_or(false),
            Link::Down => false,
        }
    }

    /// The netif addressing, mDNS, SNTP and IPv6 work on.
    fn netif(&self) -> Option<&EspNetif> {
        match self {
            Link::Eth(eth) => Some(eth.eth().netif()),
            #[cfg(feature = "wifi-dev")]
            Link::Wifi(wifi) => Some(wifi.wifi().sta_netif()),
            Link::Down => None,
        }
    }

    /// Re-associate a dropped devboard link. Never called for Ethernet:
    /// the W5500 reconnects in hardware.
    #[cfg(feature = "wifi-dev")]
    fn retry(&mut self) {
        if let Link::Wifi(wifi) = self
            && let Err(e) = wifi.connect()
        {
            log::warn!("wifi-dev: re-association failed: {e}");
        }
    }
}

/// Bring up whatever this build calls a network.
#[cfg(not(feature = "wifi-dev"))]
fn bring_up_link(
    hw: NetHw,
    mac: &[u8; 6],
    hostname: &str,
    cfg: &NetCfg,
    sysloop: EspSystemEventLoop,
) -> anyhow::Result<Link> {
    bring_up(hw.spi, hw.pins, mac, hostname, cfg, sysloop).map(Link::Eth)
}

/// The devboard variant: the radio instead of the W5500, whose SPI bus
/// and pins are left untouched (there is no W5500 on a devboard, and on a
/// board this build has no business running).
#[cfg(feature = "wifi-dev")]
fn bring_up_link(
    hw: NetHw,
    _mac: &[u8; 6],
    hostname: &str,
    cfg: &NetCfg,
    sysloop: EspSystemEventLoop,
) -> anyhow::Result<Link> {
    // The W5500's bus and pins are taken and dropped: peripheral tokens,
    // no driver, nothing to release.
    let NetHw {
        spi: _spi,
        pins: _pins,
        modem,
    } = hw;
    super::wifi_dev::bring_up(modem, hostname, cfg, sysloop)
        .map(|wifi| Link::Wifi(Box::new(wifi)))
}

/// The W5500. Compiled in both builds (the [`Link::Eth`] variant needs
/// it), called only when `wifi-dev` is off.
#[cfg_attr(feature = "wifi-dev", allow(dead_code))]
fn bring_up(
    spi: SPI2<'static>,
    pins: EthPins,
    mac: &[u8; 6],
    hostname: &str,
    cfg: &NetCfg,
    sysloop: EspSystemEventLoop,
) -> anyhow::Result<GraniteEth> {
    // CS is driven by the Ethernet driver, not by the SPI bus driver.
    //
    // The bus driver is leaked on purpose. SPI2 belongs to the W5500 for the
    // lifetime of the firmware, and when the W5500 does not answer, ESP-IDF
    // leaves the Ethernet MAC's SPI device attached to the bus; dropping the
    // driver then makes esp_spi_bus_free fail and esp-idf-hal's Drop panics
    // on it. A missing W5500 must stay a logged error, not a reboot loop.
    let spi: &'static SpiDriver<'static> = Box::leak(Box::new(SpiDriver::new(
        spi,
        pins.sclk,
        pins.mosi,
        Some(pins.miso),
        &DriverConfig::new(),
    )?));

    let driver = EthDriver::new_spi(
        spi,
        pins.int,
        Some(pins.cs),
        Some(pins.rst),
        SpiEthChipset::W5500,
        SPI_BAUDRATE,
        Some(mac),
        None,
        sysloop.clone(),
    )?;

    if cfg.vlan.is_some() {
        // TODO(ADR 0001 6): 802.1Q needs the esp_eth VLAN tagging hook or a
        // second netif; the W5500 in MAC-raw mode passes tags through, so
        // the work is in lwIP, not here. Config is accepted and ignored.
        log::warn!("net.vlan is set but VLAN tagging is not implemented yet; running untagged");
    }

    let mut netif_conf = NetifConfiguration::eth_default_client();
    netif_conf.ip_configuration = Some(ip_configuration(cfg, hostname)?);

    let eth = EspEth::wrap_all(driver, EspNetif::new_with_conf(&netif_conf)?)?;
    let mut eth = BlockingEth::wrap(eth, sysloop)?;

    // Start without BlockingEth::start: that one waits for the started
    // event forever, and a missing W5500 must time out instead.
    eth.eth_mut().start()?;
    eth.eth_wait_while(
        || eth.is_started().map(|started| !started),
        Some(START_TIMEOUT),
    )?;
    log::info!("ethernet started, waiting for link");
    eth.eth_wait_while(
        || eth.is_connected().map(|connected| !connected),
        Some(LINK_TIMEOUT),
    )?;
    log::info!("link up, waiting for an address");

    // Not fatal: AutoIP or a late DHCP server still land in the monitor
    // loop, which keeps reporting.
    if let Err(e) = eth.ip_wait_while(|| eth.is_up().map(|up| !up), Some(IP_TIMEOUT)) {
        log::warn!("no address within {} s: {e}", IP_TIMEOUT.as_secs());
    }

    let netif = eth.eth().netif();
    if let Err(e) = set_hostname(netif.handle(), hostname) {
        log::warn!("hostname could not be set: {e}");
    }
    if let Ok(ip_info) = netif.get_ip_info() {
        log::info!(
            "ip {} netmask {} gw {}",
            ip_info.ip,
            ip_info.subnet.mask,
            ip_info.subnet.gateway
        );
    }

    Ok(eth)
}

/// The netif's initial IP configuration from `net`.
fn ip_configuration(cfg: &NetCfg, hostname: &str) -> anyhow::Result<ipv4::Configuration> {
    match cfg.ip_mode {
        IpMode::Dhcp => Ok(ipv4::Configuration::Client(
            ipv4::ClientConfiguration::DHCP(ipv4::DHCPClientSettings {
                hostname: Some(hostname.try_into().map_err(|_| {
                    anyhow::anyhow!("hostname {hostname} does not fit the netif hostname field")
                })?),
            }),
        )),
        IpMode::Static => {
            let (ip, mask) = parse_cidr(&cfg.address)
                .ok_or_else(|| anyhow::anyhow!("net.address {:?} is not a CIDR", cfg.address))?;
            let gateway: Ipv4Addr = cfg
                .gateway
                .trim()
                .parse()
                .map_err(|_| anyhow::anyhow!("net.gateway {:?} is not an address", cfg.gateway))?;
            let dns = cfg.dns.first().and_then(|d| d.trim().parse().ok());
            let secondary_dns = cfg.dns.get(1).and_then(|d| d.trim().parse().ok());
            Ok(ipv4::Configuration::Client(
                ipv4::ClientConfiguration::Fixed(ipv4::ClientSettings {
                    ip,
                    subnet: ipv4::Subnet {
                        gateway,
                        mask: ipv4::Mask(mask),
                    },
                    dns,
                    secondary_dns,
                }),
            ))
        }
    }
}

/// `192.168.1.10/24` -> address plus prefix length.
pub fn parse_cidr(s: &str) -> Option<(Ipv4Addr, u8)> {
    let (addr, prefix) = s.trim().split_once('/')?;
    let addr: Ipv4Addr = addr.trim().parse().ok()?;
    let prefix: u8 = prefix.trim().parse().ok()?;
    if prefix > 32 {
        return None;
    }
    Some((addr, prefix))
}

fn mask_of(prefix: u8) -> Ipv4Addr {
    if prefix == 0 {
        return Ipv4Addr::UNSPECIFIED;
    }
    Ipv4Addr::from(u32::MAX << (32 - u32::from(prefix)))
}

fn to_esp_ip4(addr: Ipv4Addr) -> esp_ip4_addr_t {
    esp_ip4_addr_t {
        addr: u32::from_le_bytes(addr.octets()),
    }
}

pub(super) fn set_hostname(handle: *mut esp_netif_t, hostname: &str) -> anyhow::Result<()> {
    let c = CString::new(hostname)?;
    unsafe { esp!(esp_netif_set_hostname(handle, c.as_ptr()))? };
    Ok(())
}

/// Switch a live netif between DHCP and a static address.
///
/// esp-idf-svc can only set this at netif construction, and commit-confirm
/// has to change it on a running interface, so this goes to `esp_netif_*`
/// directly.
pub(super) fn apply_addressing(
    handle: *mut esp_netif_t,
    cfg: &NetCfg,
    hostname: &str,
) -> anyhow::Result<()> {
    // The hostname is sent in the DHCP request, so it has to be set while
    // the client is stopped.
    unsafe {
        // Not an error if it was not running.
        let _ = esp_netif_dhcpc_stop(handle);
    }
    set_hostname(handle, hostname)?;

    match cfg.ip_mode {
        IpMode::Dhcp => {
            unsafe { esp!(esp_netif_dhcpc_start(handle))? };
            log::info!("net: dhcp started, hostname {hostname}");
        }
        IpMode::Static => {
            let (ip, prefix) = parse_cidr(&cfg.address)
                .ok_or_else(|| anyhow::anyhow!("net.address {:?} is not a CIDR", cfg.address))?;
            let gateway: Ipv4Addr = cfg
                .gateway
                .trim()
                .parse()
                .map_err(|_| anyhow::anyhow!("net.gateway {:?} is not an address", cfg.gateway))?;
            let info = esp_netif_ip_info_t {
                ip: to_esp_ip4(ip),
                netmask: to_esp_ip4(mask_of(prefix)),
                gw: to_esp_ip4(gateway),
            };
            unsafe { esp!(esp_netif_set_ip_info(handle, &info))? };
            for (i, kind) in [
                esp_netif_dns_type_t_ESP_NETIF_DNS_MAIN,
                esp_netif_dns_type_t_ESP_NETIF_DNS_BACKUP,
            ]
            .into_iter()
            .enumerate()
            {
                let Some(server) = cfg.dns.get(i).and_then(|d| d.trim().parse::<Ipv4Addr>().ok())
                else {
                    continue;
                };
                let mut dns: esp_netif_dns_info_t = unsafe { core::mem::zeroed() };
                dns.ip.u_addr.ip4 = to_esp_ip4(server);
                dns.ip.type_ = ESP_IPADDR_TYPE_V4 as u8;
                unsafe { esp!(esp_netif_set_dns_info(handle, kind, &mut dns))? };
            }
            log::info!("net: static {ip}/{prefix} gw {gateway}, hostname {hostname}");
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// SNTP
// ---------------------------------------------------------------------------

/// Set the system clock to the firmware build time.
///
/// Until SNTP syncs this is what makes TLS not-before checks pass and
/// timestamps monotonic (ADR component 6). Never moves the clock backwards.
pub fn set_clock_to_build_time() {
    let build = super::identity::BUILD_UNIX;
    if build == 0 {
        return;
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if now >= build {
        return;
    }
    let tv = timeval {
        tv_sec: build as _,
        tv_usec: 0,
    };
    if unsafe { settimeofday(&tv, core::ptr::null()) } == 0 {
        log::info!("clock set to the firmware build time ({build} unix) until sntp syncs");
    } else {
        log::warn!("clock could not be set to the build time");
    }
}

/// Start SNTP. With `net.sntp` empty the server comes from DHCP option 42
/// (`CONFIG_LWIP_DHCP_GET_NTP_SRV=y`), otherwise from the config.
fn start_sntp(cfg: &NetCfg) -> anyhow::Result<()> {
    let server = cfg.sntp.trim();
    let mut conf: esp_sntp_config_t = unsafe { core::mem::zeroed() };
    conf.smooth_sync = false;
    conf.wait_for_sync = false;
    conf.start = true;
    conf.renew_servers_after_new_IP = true;
    conf.ip_event_to_renew = ip_event_t_IP_EVENT_ETH_GOT_IP;
    conf.index_of_first_server = 0;

    // Kept alive for the lifetime of the process: esp_netif_sntp only
    // stores the pointer.
    let held: Option<&'static CString>;
    if server.is_empty() {
        conf.server_from_dhcp = true;
        conf.num_of_servers = 0;
        held = None;
        log::info!("sntp: server from dhcp option 42");
    } else {
        let c: &'static CString = Box::leak(Box::new(CString::new(server)?));
        conf.server_from_dhcp = false;
        conf.num_of_servers = 1;
        conf.servers[0] = c.as_ptr();
        held = Some(c);
        log::info!("sntp: server {server}");
    }
    let _ = held;

    // A second init would fail; deinit first so a net config change can
    // move the server.
    unsafe { esp_netif_sntp_deinit() };
    unsafe { esp!(esp_netif_sntp_init(&conf))? };
    Ok(())
}

// ---------------------------------------------------------------------------
// mDNS
// ---------------------------------------------------------------------------

/// Advertise `_https._tcp` and `_granite._tcp` with `id=<device id>`.
///
/// mDNS is an external ESP-IDF component since v5; it is pulled in by the
/// `[[package.metadata.esp-idf-sys.extra_components]]` entry in
/// `Cargo.toml`.
fn start_mdns(hostname: &str, device_id: &str) -> anyhow::Result<esp_idf_svc::mdns::EspMdns> {
    let mut mdns = esp_idf_svc::mdns::EspMdns::take()?;
    mdns.set_hostname(hostname)?;
    mdns.set_instance_name(device_id)?;
    let txt = [("id", device_id)];
    mdns.add_service(Some(device_id), "_https", "_tcp", HTTPS_PORT, &txt)?;
    mdns.add_service(Some(device_id), "_granite", "_tcp", HTTPS_PORT, &txt)?;
    log::info!("mdns: {hostname}.local advertising _https._tcp and _granite._tcp on {HTTPS_PORT}");
    Ok(mdns)
}
