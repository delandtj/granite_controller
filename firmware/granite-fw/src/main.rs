//! Granite controller firmware - bring-up skeleton.
//!
//! Stage 1 of firmware/docs/adr/0001-firmware-architecture.md: prove the
//! toolchain and the board bring-up path. In order, this binary
//!
//!   1. holds the relay/sense expander reset (GPIO10) low, before any
//!      other GPIO is touched, so no photoMOS relay can conduct,
//!   2. holds the header expander reset (GPIO4) low,
//!   3. blinks the status LED (GPIO1) at 1 Hz from its own thread,
//!   4. prints the device id and the eFuse Ethernet MAC on the console,
//!   5. brings up the W5500 on SPI2 and waits for link plus a DHCP lease,
//!   6. prints a heartbeat every 10 s.
//!
//! Pin map: docs/controller.md, section "MCU".

use std::thread;
use std::time::Duration;

use esp_idf_svc::eth::{BlockingEth, EspEth, EthDriver, SpiEth, SpiEthChipset};
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::hal::gpio::{Gpio18, Gpio19, Gpio20, Gpio21, Gpio22, Gpio23, PinDriver};
use esp_idf_svc::hal::peripherals::Peripherals;
use esp_idf_svc::hal::spi::{SPI2, SpiDriver, config::DriverConfig};
use esp_idf_svc::hal::units::Hertz;
use esp_idf_svc::ipv4;
use esp_idf_svc::netif::{EspNetif, NetifConfiguration};
use esp_idf_svc::sys::{EspError, esp, esp_mac_type_t_ESP_MAC_ETH, esp_read_mac};

/// W5500 SPI clock. The datasheet allows 33 MHz; start conservative.
const SPI_BAUDRATE: Hertz = Hertz(20_000_000);
/// Status LED half period (1 Hz blink).
const LED_HALF_PERIOD: Duration = Duration::from_millis(500);
/// How long to wait for the driver to report the MAC/PHY started.
const START_TIMEOUT: Duration = Duration::from_secs(5);
/// How long to wait for the PHY to report link up.
const LINK_TIMEOUT: Duration = Duration::from_secs(10);
/// How long to wait for a DHCP lease once the link is up.
const IP_TIMEOUT: Duration = Duration::from_secs(30);
/// Heartbeat interval of the idle loop.
const HEARTBEAT: Duration = Duration::from_secs(10);

type GraniteEth = BlockingEth<EspEth<'static, SpiEth<&'static SpiDriver<'static>>>>;

/// SPI and control pins of the W5500 (docs/controller.md, "Ethernet").
struct EthPins {
    sclk: Gpio19<'static>,
    mosi: Gpio20<'static>,
    miso: Gpio21<'static>,
    cs: Gpio18<'static>,
    int: Gpio22<'static>,
    rst: Gpio23<'static>,
}

fn main() -> anyhow::Result<()> {
    // Patches that the ESP-IDF build expects to find linked in.
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    let peripherals = Peripherals::take()?;
    let pins = peripherals.pins;

    // The very first GPIO action: hold U14/U15 in reset. While GPIO10 is
    // low every relay output is open, whatever the firmware does next.
    // The pin has a pull-down, so this also survives a chip reset.
    let mut exp_reset_int = PinDriver::output(pins.gpio10)?;
    exp_reset_int.set_low()?;

    // Header expander (J8) reset, also low for now.
    let mut exp_reset = PinDriver::output(pins.gpio4)?;
    exp_reset.set_low()?;

    let mut status_led = PinDriver::output(pins.gpio1)?;
    status_led.set_low()?;
    thread::Builder::new()
        .name("status-led".into())
        .stack_size(2048)
        .spawn(move || {
            loop {
                if let Err(err) = status_led.toggle() {
                    log::error!("status LED: {err}");
                    return;
                }
                thread::sleep(LED_HALF_PERIOD);
            }
        })?;

    let mac = eth_mac()?;
    let device_id = device_id(&mac);
    log::info!("device id {device_id}");
    log::info!(
        "ethernet mac {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        mac[0],
        mac[1],
        mac[2],
        mac[3],
        mac[4],
        mac[5]
    );

    // Kept alive in main: the ESP-IDF event loop outlives a failed Ethernet
    // bring-up, later stages subscribe to it.
    let sysloop = EspSystemEventLoop::take()?;
    let eth_pins = EthPins {
        sclk: pins.gpio19,
        mosi: pins.gpio20,
        miso: pins.gpio21,
        cs: pins.gpio18,
        int: pins.gpio22,
        rst: pins.gpio23,
    };

    // A missing or unresponsive W5500 must not panic or reboot: the USB
    // console stays the way in, and later stages bring the link up again.
    let _eth = match bring_up_eth(
        peripherals.spi2,
        eth_pins,
        &mac,
        &device_id,
        sysloop.clone(),
    ) {
        Ok(eth) => Some(eth),
        Err(err) => {
            log::error!("ethernet bring-up failed: {err:#}");
            None
        }
    };

    let mut ticks: u64 = 0;
    loop {
        log::info!(
            "heartbeat {ticks} ({} s uptime), {device_id}",
            ticks * HEARTBEAT.as_secs()
        );
        ticks += 1;
        thread::sleep(HEARTBEAT);
    }
}

/// eFuse Ethernet MAC. This is the address the user notes before a board
/// is sealed, so every identity in the firmware derives from it.
fn eth_mac() -> Result<[u8; 6], EspError> {
    let mut mac = [0u8; 6];
    esp!(unsafe { esp_read_mac(mac.as_mut_ptr(), esp_mac_type_t_ESP_MAC_ETH) })?;
    Ok(mac)
}

fn device_id(mac: &[u8; 6]) -> String {
    format!("granite-{:02x}{:02x}{:02x}", mac[3], mac[4], mac[5])
}

fn bring_up_eth(
    spi: SPI2<'static>,
    pins: EthPins,
    mac: &[u8; 6],
    hostname: &str,
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

    // DHCP on the default Ethernet netif, with the device id as the
    // hostname that is sent to the server.
    let mut netif_conf = NetifConfiguration::eth_default_client();
    netif_conf.ip_configuration = Some(ipv4::Configuration::Client(
        ipv4::ClientConfiguration::DHCP(ipv4::DHCPClientSettings {
            hostname: Some(hostname.try_into().map_err(|_| {
                anyhow::anyhow!("hostname {hostname} does not fit the netif hostname field")
            })?),
        }),
    ));

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
    log::info!("link up, waiting for a DHCP lease");
    eth.ip_wait_while(|| eth.is_up().map(|up| !up), Some(IP_TIMEOUT))?;

    let ip_info = eth.eth().netif().get_ip_info()?;
    log::info!(
        "ip {} netmask {} gw {:?}",
        ip_info.ip,
        ip_info.subnet.mask,
        ip_info.subnet.gateway
    );

    Ok(eth)
}
