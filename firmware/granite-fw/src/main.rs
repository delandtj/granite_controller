//! Granite controller firmware (ADR 0001).
//!
//! Boot order, which is the part of this file that must not be rearranged:
//!
//!   1. link the patches the ESP-IDF build expects,
//!   2. install the log ring in front of the ESP-IDF logger, so the first
//!      lines of the boot are already in the ring and on the console,
//!   3. **drive GPIO10 (U14/U15 reset) and GPIO4 (header expander reset)
//!      low before any other GPIO is touched.** While GPIO10 is low every
//!      photoMOS relay output is open, whatever the firmware does next.
//!      The pin has a pull-down, so this also survives a chip reset. No
//!      line of code may move anything above this one.
//!   4. platform layer: NVS, config, identity, OTA probation, network,
//!   5. shared state: the `Observed` snapshot, the command channel, the
//!      event bus, and the dispatcher thread that joins them,
//!   6. the console on USB Serial/JTAG,
//!   7. the hardware layer and the three servers, at the points marked
//!      `// INTEGRATION:`: HTTPS and the JSON API, the MQTT worker, the
//!      Modbus supervisor, and the config watcher that connects a saved
//!      `sec.modbus` to the running listener,
//!   8. one free-heap line once everything is up, then the heartbeat.
//!
//! Pin map: docs/controller.md, section "MCU".

#[allow(unused_imports)]
use granite_fw as _;

use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex, RwLock};
use std::thread;
use std::time::Duration;

use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::hal::gpio::PinDriver;
use esp_idf_svc::hal::peripherals::Peripherals;
use granite_core::actuator::{Actuator, apply_to_nodes};
use granite_core::config::Section;
use granite_core::dispatch::{DispatchCtx, SideEffect, dispatch};
use granite_core::hal::{
    BoardTemp, BusVoltage, DryInputs, LedPattern, NodeSense, NodeSwitches, Probes, StatusLed,
};
use granite_core::modbus_map::fault_bits;
use granite_core::msg::{Event, EventKind, event_from_actuator, event_from_rule};
use granite_core::node::Nodes;
use granite_core::observed::{Observed, ProbeObs, Stamped};
use granite_core::rules::RuleEngine;
use granite_fw::platform::{self, NullSwitches, Platform};
use granite_fw::{http, hw, modbus, mqtt};

/// Heartbeat interval of the idle loop.
const HEARTBEAT: Duration = Duration::from_secs(10);
/// Dispatcher tick: the actuator state machine and the rule engine both
/// need regular ticks, and a command may arrive between two of them.
const DISPATCH_TICK: Duration = Duration::from_millis(50);
/// Rule engine period (ADR component 5: every 1 s).
const RULES_PERIOD_MS: u64 = 1000;
/// Sense tick: U15, the TMP1075 and VIN are all 1 s in the ADR.
const SENSE_TICK: Duration = Duration::from_secs(1);
/// How often the config watcher looks for a `sec.modbus` change and
/// refreshes the broker state the status page reads.
const CONFIG_WATCH: Duration = Duration::from_secs(5);

/// Marks [`LED_BITS`] as carrying a real reading.
const LED_BITS_VALID: u16 = 0x100;

/// Raw node power-LED byte from U15, handed from the sense thread to the
/// dispatcher, which owns the 100 ms debounce and the `Busy` overlay.
/// Zero means "U15 has not answered"; [`LED_BITS_VALID`] distinguishes
/// that from a genuine "all eight nodes off".
static LED_BITS: AtomicU16 = AtomicU16::new(0);

/// The parts of the `nodes` section the sense thread works from.
///
/// They used to be read once at boot, which made `vin_trim`, the probe
/// names and `t_probe_s` reboot-only settings. [`apply_config`] now
/// pushes them here on every `nodes` save and the sense thread picks the
/// change up on its next tick (`generation` is what it compares).
#[derive(Debug, Clone, PartialEq, Eq)]
struct SenseCfg {
    /// Bumped on every real change.
    generation: u32,
    /// Configured probes as (ROM id, name). Empty means "use whatever the
    /// bus reported at boot", which is also the boot behaviour.
    probes: Vec<(u64, String)>,
    /// `t_probe_s` as milliseconds.
    probe_period_ms: u64,
    /// `nodes.vin_min_mv`.
    vin_min_mv: u32,
    /// `nodes.vin_max_mv`.
    vin_max_mv: u32,
}

/// Published by [`apply_config`], read by the sense thread.
static SENSE_CFG: Mutex<SenseCfg> = Mutex::new(SenseCfg {
    generation: 0,
    probes: Vec::new(),
    probe_period_ms: 10_000,
    vin_min_mv: 0,
    vin_max_mv: 0,
});

fn main() -> anyhow::Result<()> {
    // 1. Patches that the ESP-IDF build expects to find linked in.
    esp_idf_svc::sys::link_patches();
    platform::ota::rtc_watchdog_at_boot();
    // Bench test of the hardware probation deadline: an image on
    // probation hangs before the probation thread exists.
    #[cfg(feature = "boot-hang-test")]
    if platform::ota::is_pending_verify() {
        esp_idf_svc::log::EspLogger::initialize_default();
        log::warn!("boot-hang-test: hanging on probation; the RTC watchdog has to reset the chip");
        loop {
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
    }

    // 2. The log ring replaces EspLogger::initialize_default: it forwards
    //    to the ESP-IDF logger and keeps the last 16 KB for `log tail`.
    if !platform::logring::init() {
        esp_idf_svc::log::EspLogger::initialize_default();
        log::warn!("log ring could not be installed; falling back to the plain ESP-IDF logger");
    }

    let peripherals = Peripherals::take()?;
    let pins = peripherals.pins;

    // 3. THE FIRST GPIO ACTION: hold U14/U15 in reset. While GPIO10 is
    //    low every relay output is open, whatever the firmware does next.
    //    The pin has a pull-down, so this also survives a chip reset.
    //    Nothing may be inserted above this.
    let mut exp_reset_int = PinDriver::output(pins.gpio10)?;
    exp_reset_int.set_low()?;

    // Header expander (J8) reset, also low for now.
    let mut exp_reset = PinDriver::output(pins.gpio4)?;
    exp_reset.set_low()?;

    // 4 and 5. Shared state first, because the platform layer wants the
    //          command sender and the snapshot handle.
    let observed = Arc::new(RwLock::new(Observed::new()));
    let (commands, commands_rx) = granite_fw::http::CommandChannel::new();
    // Modbus input register 13 (granite_core::modbus_map::fault_bits).
    // Whoever notices a fault sets its bit; the Modbus server only reads.
    let faults = Arc::new(AtomicU16::new(0));

    // Kept alive: the ESP-IDF event loop outlives a failed Ethernet
    // bring-up and the network thread subscribes to it.
    let sysloop = EspSystemEventLoop::take()?;
    // The network hardware. With `--features wifi-dev` the network thread
    // brings up the radio instead of the W5500 (platform::wifi_dev): a
    // devboard has no W5500, and without a network nothing above the
    // hardware layer can be exercised. The default image has no radio
    // code in it at all.
    let net_hw = platform::net::NetHw {
        spi: peripherals.spi2,
        pins: platform::net::EthPins {
            sclk: pins.gpio19,
            mosi: pins.gpio20,
            miso: pins.gpio21,
            cs: pins.gpio18,
            int: pins.gpio22,
            rst: pins.gpio23,
        },
        #[cfg(feature = "wifi-dev")]
        modem: peripherals.modem,
    };

    // The hardware layer is in this build, so the OTA probation ladder has
    // to wait for "expanders initialised" (step 1). This has to be said
    // before platform::init, which is where the ladder starts.
    platform::ota::expect_expanders();

    let platform = platform::init(
        net_hw,
        sysloop.clone(),
        Arc::clone(&commands),
        Arc::clone(&observed),
    )?;

    log::info!(
        "granite-fw {} on {}, boot reason {}",
        platform::FW_VERSION,
        platform.device_id(),
        platform::boot_reason()
    );
    log::info!("image signing: {}", platform::signing_summary());

    // INTEGRATION: hw::init. It takes GPIO10 over (still low), configures
    // both MCP23017s, starts the sense task, the LED and the two I2C
    // buses. GPIO4 (the J8 header expander reset) is not part of its pin
    // map, so it stays held low here.
    //
    // A hardware layer that does not come up is logged, not fatal: the USB
    // console and the network have to stay reachable on a board whose I2C
    // bus is dead, and the dispatcher then runs on
    // platform::NullSwitches, which refuses every press honestly.
    let vin_trim = platform.config().nodes.vin_trim;
    let hw = hw::init(hw::HwInit {
        exp_reset: exp_reset_int,
        exp_int: pins.gpio0,
        status_led: pins.gpio1,
        int_sda: pins.gpio2,
        int_scl: pins.gpio3,
        ext_sda: pins.gpio6,
        ext_scl: pins.gpio7,
        vin: pins.gpio5,
        onewire: pins.gpio16,
        uart_rx: pins.gpio17,
        adc1: peripherals.adc1,
        vin_trim,
    });
    let _held_header_reset = exp_reset;

    // 6. The console. Physical access is full access; it is the way in
    //    when the network is not.
    platform::console::start(Arc::clone(&platform));

    // Modbus fault bit 5: a config section that could not be read fell
    // back to its defaults at boot. platform::init logged and published
    // that, but it does not keep the list, so this is a second read of
    // the same sections (a few milliseconds of NVS, once). The bit stays
    // set until the section is written again, which the dispatcher sees.
    let fallback_sections: Vec<Section> = match platform.store.lock() {
        Ok(mut store) => store
            .load_config()
            .1
            .into_iter()
            .map(|fb| fb.section)
            .collect(),
        Err(_) => Vec::new(),
    };
    if !fallback_sections.is_empty() {
        faults.fetch_or(fault_bits::CONFIG_FALLBACK, Ordering::Relaxed);
    }

    // 5b. The dispatcher: the single consumer of the command channel. It
    //     owns the relay driver, because the actuator state machine is
    //     single-threaded and this is the only thread that ticks it.
    let dispatcher_platform = Arc::clone(&platform);
    match hw {
        Ok(hw) => {
            // Every planned reboot and the OTA rollback path open the
            // relays before restarting. force_reset_low is the lock-free
            // path the press deadline and the panic hook also use: one
            // GPIO register write, no I2C, safe from any context, so a
            // wedged bus cannot stop a reboot from releasing the relays.
            platform::set_release_hook(hw::expanders::force_reset_low);
            // Probation step 1: the expanders answered.
            platform::ota::mark_expanders();

            let relays = hw.relays;
            let led = hw.led;
            let sense = hw.sense;
            let probes = hw.probes;
            let board_temp = hw.board_temp;
            let vin = hw.vin;
            // The rest of the hardware layer comes up on a board with no
            // MCP23017 (a devboard): only this says whether a relay can
            // actually close, and the boot policy depends on it.
            let expanders_up = hw.expanders_up;
            if !expanders_up {
                faults.fetch_or(fault_bits::EXPANDER, Ordering::Relaxed);
            }

            start_sense(
                Arc::clone(&platform),
                sense,
                probes,
                board_temp,
                vin,
                Arc::clone(&faults),
            );
            // The devboard RGB mirror (`rgb-led`) shares the one pattern
            // handle, so it can never disagree with the GPIO1 LED.
            #[cfg(feature = "rgb-led")]
            let rgb = led.clone();
            start_led(Arc::clone(&platform), led, Arc::clone(&faults));
            #[cfg(feature = "rgb-led")]
            if let Err(e) = hw::rgb::start(rgb, mqtt::connected) {
                log::error!("rgb mirror did not start: {e:#}");
            }

            let dispatch_faults = Arc::clone(&faults);
            thread::Builder::new()
                .name("dispatch".into())
                .stack_size(12288)
                .spawn(move || {
                    dispatcher(
                        dispatcher_platform,
                        commands_rx,
                        relays,
                        expanders_up,
                        dispatch_faults,
                        fallback_sections,
                    )
                })?;
        }
        Err(e) => {
            log::error!(
                "hardware layer did not come up ({e:#}); relays stay in reset and every press \
                 will be refused"
            );
            faults.fetch_or(fault_bits::EXPANDER | fault_bits::SENSE, Ordering::Relaxed);
            let dispatch_faults = Arc::clone(&faults);
            thread::Builder::new()
                .name("dispatch".into())
                .stack_size(12288)
                .spawn(move || {
                    dispatcher(
                        dispatcher_platform,
                        commands_rx,
                        NullSwitches,
                        false,
                        dispatch_faults,
                        fallback_sections,
                    )
                })?;
        }
    }

    // The HTTPS setup page and /api/v1, on the device certificate the
    // platform layer generated or loaded. Kept alive for the life of the
    // firmware: dropping the handle stops the servers.
    let _http = match http::start(platform::api_impl::http_ctx(&platform)) {
        Ok(handle) => {
            log::info!(
                "https on {} with cert sha256 {}",
                platform::api_impl::HTTPS_PORT,
                platform.identity.cert_sha256
            );
            Some(handle)
        }
        Err(e) => {
            // A board that cannot serve the page is still reachable over
            // the console and MQTT, and must not reboot-loop.
            log::error!("http server did not start: {e:#}");
            None
        }
    };

    // INTEGRATION: the MQTT client (ADR component 7). The worker thread is
    // started whatever the configuration says: it idles on the config when
    // there is no broker to talk to and connects as soon as the section
    // says it may, so enabling MQTT or changing the broker needs no reboot
    // (ADR component 6, commit-confirm). The handle is kept: dropping it
    // stops the worker.
    //
    // The probation ladder waits for a broker only when one is
    // configured: an image on probation with MQTT off confirms on an
    // authenticated HTTPS request instead. platform::init already armed
    // that flag (it reads the config first, and the ladder has to be
    // watching before the link comes up); the call is idempotent, and
    // repeating it here keeps it next to the worker it is about.
    let cfg_at_boot = platform.config();
    if cfg_at_boot.mqtt.enabled {
        platform::ota::expect_broker();
    }
    let _mqtt = match mqtt::start(mqtt::MqttCtx {
        config: Box::new({
            let p = Arc::clone(&platform);
            move || p.config()
        }),
        secrets: Box::new({
            let p = Arc::clone(&platform);
            move || {
                p.store
                    .lock()
                    .map(|mut s| s.load_secrets())
                    .unwrap_or_default()
            }
        }),
        observed: Arc::clone(&platform.observed),
        events: platform.events.subscribe_replay(),
        // The worker applies `sys.log_level` itself, so it subscribes to
        // everything the ring sees and filters live.
        logs: platform::logring::subscribe(log::Level::Trace),
        commands: Box::new({
            let channel = Arc::clone(&platform.commands);
            move |cmd, reply| channel.send(cmd, reply)
        }),
        host: Box::new({
            let p = Arc::clone(&platform);
            move || mqtt::HostStatus {
                ip: p.net.status().ip,
                uptime_s: (platform::now_ms() / 1000) as u32,
                boot_reason: platform::boot_reason(),
                ota_state: platform::ota::ota_state().to_string(),
            }
        }),
        device_id: platform.device_id().to_string(),
        fw_version: platform::FW_VERSION.to_string(),
        // Probation step 3a.
        on_connect: Some(Box::new(platform::ota::mark_broker)),
        // mqtt.auto_confirm_on_connect: a session on the staged section is
        // the confirm (ADR component 6). Does nothing when the section is
        // not staged.
        confirm_staged: Some(Box::new({
            let p = Arc::clone(&platform);
            move || {
                platform::api_impl::auto_confirm_mqtt(&p);
            }
        })),
    }) {
        Ok(handle) => {
            if handle.enabled_at_start() {
                log::info!(
                    "mqtt worker started for {}:{}",
                    cfg_at_boot.mqtt.host,
                    cfg_at_boot.mqtt.port
                );
            }
            Some(handle)
        }
        Err(e) => {
            // A broker that cannot be used must not stop the board from
            // serving its page: the configuration is the thing to fix.
            log::error!("mqtt worker did not start: {e:#}");
            None
        }
    };

    // INTEGRATION: the Modbus TCP server (ADR component 9). Started
    // unconditionally: what `start` spawns is the supervisor, and it binds
    // the listener only while `sec.modbus` says it may (enabled plus a
    // usable allow-list). It logs the reason either way.
    let modbus = match modbus::start(modbus::ModbusCtx {
        observed: Arc::clone(&platform.observed),
        commands: Arc::clone(&platform.commands),
        config: cfg_at_boot.sec.modbus.clone(),
        fw: modbus::fw_version(),
        faults: Arc::clone(&faults),
    }) {
        Ok(handle) => Some(handle),
        Err(e) => {
            log::error!("modbus supervisor did not start: {e:#}");
            None
        }
    };

    // The config watcher: the one thing that turns a saved `sec` section
    // into a running (or stopped) Modbus listener, and the one thing that
    // keeps the status page's MQTT box current.
    start_config_watch(Arc::clone(&platform), modbus.clone(), Arc::clone(&faults));

    // Everything is up, so this is the heap number that matters: it is
    // measured once, after the servers have allocated their buffers and
    // before any traffic.
    log::info!(
        "boot complete: free heap {} bytes, minimum free since boot {} bytes, largest free block \
         {} bytes",
        unsafe { esp_idf_svc::sys::esp_get_free_heap_size() },
        unsafe { esp_idf_svc::sys::esp_get_minimum_free_heap_size() },
        unsafe {
            esp_idf_svc::sys::heap_caps_get_largest_free_block(
                esp_idf_svc::sys::MALLOC_CAP_DEFAULT,
            )
        }
    );

    let mut ticks: u64 = 0;
    loop {
        let net = platform.net.status();
        log::info!(
            "heartbeat {ticks} ({} s uptime), {}, link {}, ip {}, ota {}",
            ticks * HEARTBEAT.as_secs(),
            platform.device_id(),
            if net.link { "up" } else { "down" },
            if net.ip.is_empty() { "-" } else { &net.ip },
            platform::ota::ota_state()
        );
        ticks += 1;
        thread::sleep(HEARTBEAT);
    }
}

/// The one dispatcher (ADR, "Tasks and data flow").
///
/// Owns the actuator queue, the node tracker and the rule engine, because
/// all three are single-threaded state machines that only this thread
/// ticks. Commands arrive from MQTT, HTTP and Modbus through one channel,
/// go through `granite_core::dispatch`, and come back as a reply plus
/// events plus side effects this thread carries out.
/// ### Why this thread keeps its own `Config`
///
/// An HTTP request holds the write guard on `platform.config` for its
/// whole lifetime (`granite_core::api::ApiCtx` takes `&mut Config`) and
/// may, inside that, send a command down this channel and wait for the
/// ack. If this thread then blocked on the same lock, the request and the
/// dispatcher would wait for each other. So: a local working copy,
/// `try_write` to publish a change this thread made, `try_read` to pick
/// up a change the HTTP task made, and never a blocking wait.
fn dispatcher<S: NodeSwitches>(
    platform: Arc<Platform>,
    commands: Receiver<granite_fw::http::CommandRequest>,
    switches: S,
    expanders_up: bool,
    faults: Arc<AtomicU16>,
    fallback_sections: Vec<Section>,
) {
    // `switches` is the hardware layer's relay driver when it came up, and
    // platform::NullSwitches (which refuses every press) when it did not.
    let mut actuator = Actuator::new(switches);
    let mut nodes = Nodes::new();
    let mut rules = RuleEngine::new();

    let mut cfg = platform.config();
    apply_config(&platform, &cfg, &mut actuator, &mut nodes, &mut rules);

    let mut last_rules_ms = 0u64;
    // Set when this thread changed `cfg` and the shared copy has not
    // caught up yet.
    let mut push_pending = false;
    // Boot policy (ADR component 3): once, t_settle after this thread
    // started, and never when the hardware layer did not come up.
    let boot_ms = platform::now_ms();
    let mut boot_policy_done = false;
    // Modbus fault bit 0 at runtime: set on an actuator fault, cleared
    // once the hardware layer's 1 s readback has been clean again.
    let mut expander_fault_seq: Option<u32> = None;
    // Sections that fell back to their defaults at boot (fault bit 5).
    let mut fallback_pending = fallback_sections;

    loop {
        let now = platform::now_ms();
        let mut events = Vec::new();

        match commands.recv_timeout(DISPATCH_TICK) {
            Ok((cmd, reply)) => {
                let snapshot = platform
                    .observed
                    .read()
                    .map(|o| o.clone())
                    .unwrap_or_else(|_| Observed::new());
                let before = cfg.clone();
                let device_id = platform.identity.device_id.clone();
                let mut ctx = DispatchCtx {
                    now_ms: now,
                    device_id: &device_id,
                    observed: &snapshot,
                    actuator: &mut actuator,
                    rules: &mut rules,
                    config: &mut cfg,
                    verify_sig: None,
                };
                let out = dispatch(&cmd, &mut ctx);

                let _ = reply.send(out.reply.clone());
                events.extend(out.events);
                for effect in out.effects {
                    // A section that fell back at boot is trustworthy
                    // again the moment it is written (fault bit 5).
                    if let SideEffect::SaveSection(section) = &effect {
                        fallback_pending.retain(|s| s != section);
                        if fallback_pending.is_empty() {
                            faults.fetch_and(!fault_bits::CONFIG_FALLBACK, Ordering::Relaxed);
                        }
                    }
                    apply_effect(&platform, &cfg, effect);
                }
                if cfg != before {
                    push_pending = true;
                    apply_config(&platform, &cfg, &mut actuator, &mut nodes, &mut rules);
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                log::error!("dispatcher: every command sender is gone; stopping");
                return;
            }
        }

        // Publish a local config change, or pick up someone else's. Never
        // blocking: see the note on this function.
        if push_pending {
            if let Ok(mut shared) = platform.config.try_write() {
                *shared = cfg.clone();
                push_pending = false;
            }
        } else if let Ok(shared) = platform.config.try_read()
            && *shared != cfg
        {
            let next = shared.clone();
            drop(shared);
            // The HTTP API writes a section itself (api::put_section), so
            // this is where a fallback section stops being a fallback on
            // that path (fault bit 5).
            if !fallback_pending.is_empty() {
                fallback_pending.retain(|s| cfg.section_json(*s).ok() == next.section_json(*s).ok());
                if fallback_pending.is_empty() {
                    faults.fetch_and(!fault_bits::CONFIG_FALLBACK, Ordering::Relaxed);
                }
            }
            cfg = next;
            apply_config(&platform, &cfg, &mut actuator, &mut nodes, &mut rules);
        }

        // Fold the raw LED byte the sense thread left behind into the node
        // tracker, which debounces it (100 ms, ADR component 3).
        let raw = LED_BITS.load(Ordering::Relaxed);
        nodes.update_sense(
            now,
            if raw & LED_BITS_VALID != 0 {
                Some((raw & 0xff) as u8)
            } else {
                None
            },
        );

        // Tick the actuator state machine.
        let snapshot = platform
            .observed
            .read()
            .map(|o| o.clone())
            .unwrap_or_else(|_| Observed::new());
        events.extend(actuator.tick(now, &snapshot));
        apply_to_nodes(&mut nodes, &events, now);

        // Refresh the parts of the snapshot this thread owns. The sense
        // task owns the rest.
        if let Ok(mut observed) = platform.observed.write() {
            observed.refresh_nodes(&nodes);
            observed.uptime_s = Stamped::new((now / 1000) as u32, now);
            let net = platform.net.status();
            observed.link_up = Stamped::new(net.link, now);
            if net.link && !net.ip.is_empty() {
                platform::ota::mark_link();
            }
        }

        // The boot policy, once (ADR component 3): t_settle after this
        // thread started, so the LED debounce has a trustworthy value,
        // then `on`/`off` on the nodes that ask for it, staggered, in
        // nodes.order. Never on a board whose expanders did not answer:
        // every press there would be refused anyway, and a silent
        // "policy applied" would be a lie.
        if !boot_policy_done && now >= boot_ms + u64::from(cfg.nodes.timings.t_settle_ms) {
            boot_policy_done = true;
            if expanders_up {
                let policies = cfg.nodes.boot_policies();
                let snapshot = platform
                    .observed
                    .read()
                    .map(|o| o.clone())
                    .unwrap_or_else(|_| Observed::new());
                let (submission, mut evs) =
                    actuator.submit_boot_policy("boot", &policies, now, &snapshot);
                log::info!(
                    "boot policy after {} ms settle: {}, {:?}",
                    cfg.nodes.timings.t_settle_ms,
                    match submission {
                        granite_core::actuator::Submission::Accepted { jobs } =>
                            format!("{jobs} press job(s) queued"),
                        granite_core::actuator::Submission::Done(ref result) =>
                            format!("nothing queued ({})", result.as_str()),
                    },
                    policies
                );
                apply_to_nodes(&mut nodes, &evs, now);
                events.append(&mut evs);
            } else {
                log::warn!(
                    "boot policy not applied: the expanders did not answer, so no relay can \
                     close and no node is touched"
                );
            }
        }

        // Rules once a second, against the snapshot, broker or no broker.
        if now.saturating_sub(last_rules_ms) >= RULES_PERIOD_MS {
            last_rules_ms = now;
            let snapshot = platform
                .observed
                .read()
                .map(|o| o.clone())
                .unwrap_or_else(|_| Observed::new());
            for fired in rules.tick(now, &snapshot) {
                log::warn!(
                    "rule {} ({}) fired on {} with value {}",
                    fired.rule_id,
                    fired.name,
                    fired.target,
                    fired.value
                );
                platform.publish(event_from_rule(&fired, now));
                if let granite_core::rules::RuleAction::Act { kind } = fired.action {
                    let req = granite_core::actuator::ActionRequest::new(
                        format!("rule{}", fired.rule_id),
                        kind,
                        fired.target,
                    );
                    let (_, mut evs) = actuator.submit(&req, now, &snapshot);
                    events.append(&mut evs);
                }
            }
        }

        for ev in &events {
            if let Some(event) = event_from_actuator(ev, now) {
                platform.publish(event);
            }
            // Modbus fault bit 0 at runtime: a blown press deadline or an
            // expander that did not behave. The bit is cleared again by
            // the first clean 1 s readback after it (hw::expanders), not
            // by the next press: a press can succeed on a chip that is
            // still misconfigured.
            if let granite_core::actuator::ActuatorEvent::Fault { fault, node, .. } = ev {
                log::error!("actuator fault: {fault} (node {node:?})");
                faults.fetch_or(fault_bits::EXPANDER, Ordering::Relaxed);
                expander_fault_seq = Some(hw::expanders::verify_ok_seq());
            }
        }
        if let Some(seq) = expander_fault_seq
            && hw::expanders::verify_ok_seq() != seq
        {
            expander_fault_seq = None;
            faults.fetch_and(!fault_bits::EXPANDER, Ordering::Relaxed);
            log::info!("expander readback is clean again; fault bit cleared");
        }
    }
}

/// Push the timings, node order, sense modes and rule set from `cfg` into
/// the state machines this thread owns, and the sense modes into the
/// snapshot so a publisher can say "this LED is not trusted" without
/// reading the config.
///
/// Also pushes what the two other threads cache: the VIN trim into the
/// ADC reader and the probe list, probe period and VIN window into
/// [`SENSE_CFG`]. Those four used to be read once at boot, so editing
/// them needed a reboot; they now apply on the next sense tick.
fn apply_config<S: NodeSwitches>(
    platform: &Arc<Platform>,
    cfg: &granite_core::config::Config,
    actuator: &mut Actuator<S>,
    nodes: &mut Nodes,
    rules: &mut RuleEngine,
) {
    actuator.set_timings(cfg.nodes.timings);
    actuator.set_order(cfg.nodes.order);
    nodes.set_sense(cfg.nodes.sense_modes());
    rules.set_rules(cfg.rules.rules.clone());
    if let Ok(mut observed) = platform.observed.write() {
        for (i, mode) in cfg.nodes.sense_modes().into_iter().enumerate() {
            observed.nodes[i].sense = mode;
        }
    }
    hw::vin::set_trim(cfg.nodes.vin_trim);
    push_sense_cfg(cfg);
}

/// Publish the sense thread's share of the `nodes` section. The
/// generation only moves when something really changed, so the sense
/// thread does not rebuild its probe list on every save.
fn push_sense_cfg(cfg: &granite_core::config::Config) {
    let probes: Vec<(u64, String)> = cfg
        .nodes
        .probes
        .iter()
        .filter_map(|p| granite_core::hal::rom_id_from_hex(&p.rom).map(|rom| (rom, p.name.clone())))
        .collect();
    let Ok(mut live) = SENSE_CFG.lock() else {
        log::error!("sense config: the handoff lock is poisoned; keeping the old values");
        return;
    };
    let next = SenseCfg {
        generation: live.generation.wrapping_add(1),
        probes,
        probe_period_ms: u64::from(cfg.nodes.t_probe_s.max(1)) * 1000,
        vin_min_mv: cfg.nodes.vin_min_mv,
        vin_max_mv: cfg.nodes.vin_max_mv,
    };
    if next.probes == live.probes
        && next.probe_period_ms == live.probe_period_ms
        && next.vin_min_mv == live.vin_min_mv
        && next.vin_max_mv == live.vin_max_mv
    {
        return;
    }
    *live = next;
}

/// Carry out one dispatcher side effect. `cfg` is the dispatcher's working
/// copy, which already has the change the effect is about.
fn apply_effect(
    platform: &Arc<Platform>,
    cfg: &granite_core::config::Config,
    effect: SideEffect,
) {
    match effect {
        SideEffect::SaveSection(section) => {
            match platform.store.lock() {
                Ok(mut store) => match store.save_section(cfg, section) {
                    Ok(()) => {
                        log::info!("config: {section} saved");
                        platform.publish(config_event(section, "set", None));
                    }
                    Err(e) => log::error!("config: {section} could not be saved: {e}"),
                },
                Err(_) => log::error!("config: the store is locked; {section} not saved"),
            }
        }
        SideEffect::StageSection(section) => {
            let staged = match platform.store.lock() {
                Ok(mut store) => store.stage_section(cfg, section).is_ok(),
                Err(_) => false,
            };
            if !staged {
                log::error!("config: {section} could not be staged");
                return;
            }
            log::warn!(
                "config: {section} staged; confirm within {} s or the stored value comes back",
                cfg.net.t_confirm_s
            );
            platform.publish(config_event(section, "staged", None));
            match section {
                // The net section is the one that can cut the session, so
                // it is applied live with the confirm timer running.
                Section::Net => platform.net.apply(&cfg.net, cfg.net.t_confirm_s),
                // mqtt and sec are applied live by their owners: the MQTT
                // worker rebuilds its client when the live section stops
                // matching it (mqtt::supervise) and the config watcher
                // pushes sec.modbus into the Modbus supervisor. This
                // thread has the staged value in its working copy and
                // pushes it into the live config a few lines further on,
                // so both see it within their refresh period. A new device
                // certificate is the exception: the HTTPS server cannot
                // swap its TLS config, so that one waits for a reboot.
                _ => log::warn!(
                    "config: {section} is staged and applied live; confirm promotes it, the \
                     deadline puts the stored value back (a new device certificate needs a \
                     reboot either way)"
                ),
            }
            // One deadline for every staged section, enforced by the
            // config-watch thread (ADR component 6).
            platform::api_impl::note_staged(section, cfg.net.t_confirm_s);
        }
        SideEffect::Reboot => platform::planned_reboot("command"),
        SideEffect::FactoryReset => match platform.store.lock() {
            Ok(mut store) => store.factory_reset(),
            Err(_) => platform::planned_reboot("factory_reset_locked"),
        },
        SideEffect::Ota { url, sha256 } => {
            let platform = Arc::clone(platform);
            let spawned = thread::Builder::new()
                .name("ota-pull".into())
                .stack_size(10240)
                .spawn(move || {
                    let ca = platform.config().mqtt.ca_pem;
                    match platform::ota::pull(&url, &sha256, &ca) {
                        Ok(written) => {
                            platform.publish(Event::new(
                                platform::now_ms(),
                                EventKind::Ota {
                                    phase: String::from("verified"),
                                    progress: Some(100),
                                    detail: Some(format!(
                                        "{} bytes into {}",
                                        written.len, written.slot
                                    )),
                                },
                            ));
                            platform::ota::reboot_into_new_image();
                        }
                        Err(e) => {
                            log::error!("ota pull failed: {e}");
                            platform.publish(Event::new(
                                platform::now_ms(),
                                EventKind::Ota {
                                    phase: String::from("failed"),
                                    progress: None,
                                    detail: Some(e.to_string()),
                                },
                            ));
                        }
                    }
                });
            if let Err(e) = spawned {
                log::error!("ota: pull thread could not start: {e}");
            }
        }
        SideEffect::ProbeScan => {
            // TODO(ADR 0001 4): a probe scan is the hardware layer's
            // 1-wire bus walk; it lands when src/hw exposes it.
            log::warn!("probe_scan: the 1-wire bus is owned by the hardware layer, not wired yet");
        }
    }
}

/// The sense task (ADR component 4): U15 through its handle, the TMP1075,
/// VIN and the DS18B20 bus, all landing in the one [`Observed`] snapshot.
///
/// This lives in `main.rs` because it is integration, not hardware: every
/// call below is the hardware layer's public API and every write goes to
/// the shared snapshot the rule engine and the publishers read.
fn start_sense(
    platform: Arc<Platform>,
    mut sense: hw::sense::SenseHandle,
    mut probes: hw::probes::OneWireProbes,
    mut board_temp: hw::tmp1075::Tmp1075,
    mut vin: hw::vin::Vin,
    faults: Arc<AtomicU16>,
) {
    let spawned = thread::Builder::new()
        .name("sense".into())
        .stack_size(6144)
        .spawn(move || {
            // Probe list: the configured ROM ids if there are any, else
            // whatever the bus reports. A missing probe is reported, not
            // an error (ADR component 4).
            let cfg = platform.config();
            let mut roms: Vec<(u64, String)> = cfg
                .nodes
                .probes
                .iter()
                .filter_map(|p| {
                    granite_core::hal::rom_id_from_hex(&p.rom).map(|rom| (rom, p.name.clone()))
                })
                .collect();
            // True while the list came from the configuration: only a
            // probe an operator asked for can go missing (fault bit 2).
            let mut configured = !roms.is_empty();
            // What the bus reported at boot, kept so a configuration that
            // loses its probe list falls back to it without a rescan.
            let mut scanned: Vec<u64> = Vec::new();
            if roms.is_empty() {
                match probes.scan() {
                    Ok(found) => {
                        log::info!("probes: {} on the bus", found.len());
                        scanned = found;
                        roms = scanned
                            .iter()
                            .map(|rom| (*rom, granite_core::hal::rom_id_hex(*rom)))
                            .collect();
                    }
                    Err(e) => log::warn!("probes: bus scan failed: {e}"),
                }
            }
            let mut probe_period_ms = u64::from(cfg.nodes.t_probe_s.max(1)) * 1000;
            let mut vin_min_mv = cfg.nodes.vin_min_mv;
            let mut vin_max_mv = cfg.nodes.vin_max_mv;
            // Where apply_config leaves a change to any of the four.
            let mut cfg_generation = SENSE_CFG.lock().map(|c| c.generation).unwrap_or(0);
            let mut last_probe_ms = 0u64;
            let mut vin_warned = false;
            drop(cfg);

            loop {
                let now = platform::now_ms();

                // Pick up a `nodes` save: probe names and ROM ids, the
                // probe period and the VIN window all apply here, no
                // reboot (the trim is picked up by hw::vin itself).
                if let Ok(live) = SENSE_CFG.lock()
                    && live.generation != cfg_generation
                {
                    cfg_generation = live.generation;
                    probe_period_ms = live.probe_period_ms;
                    vin_min_mv = live.vin_min_mv;
                    vin_max_mv = live.vin_max_mv;
                    configured = !live.probes.is_empty();
                    roms = if configured {
                        live.probes.clone()
                    } else {
                        scanned
                            .iter()
                            .map(|rom| (*rom, granite_core::hal::rom_id_hex(*rom)))
                            .collect()
                    };
                    drop(live);
                    log::info!(
                        "sense: {} probe(s) every {} ms, vin window {}..{} mV",
                        roms.len(),
                        probe_period_ms,
                        vin_min_mv,
                        vin_max_mv
                    );
                }

                let leds = sense.read_leds().ok();
                let dry = sense.read_dry().ok();
                let temp = board_temp.read_centi_c().ok();
                let mv = vin.read_mv().ok();

                let read_probes = now.saturating_sub(last_probe_ms) >= probe_period_ms;
                let probe_obs: Option<Vec<ProbeObs>> = if read_probes {
                    last_probe_ms = now;
                    Some(
                        roms.iter()
                            .map(|(rom, name)| ProbeObs {
                                rom: *rom,
                                name: name.clone(),
                                centi_c: probes.read(*rom).ok(),
                                ts_ms: now,
                            })
                            .collect(),
                    )
                } else {
                    None
                };

                // Modbus fault bit 2: a probe the configuration names did
                // not answer this cycle. A list that came from the bus
                // scan cannot go missing, so it never sets the bit.
                if let Some(list) = &probe_obs {
                    let missing = configured && list.iter().any(|p| p.centi_c.is_none());
                    if missing {
                        faults.fetch_or(fault_bits::PROBE_MISSING, Ordering::Relaxed);
                    } else {
                        faults.fetch_and(!fault_bits::PROBE_MISSING, Ordering::Relaxed);
                    }
                }
                // Modbus fault bit 3: the 19 V bus is outside the window
                // from the `nodes` section. A VIN the ADC could not read
                // is not "out of window"; that is bit 1's territory.
                let vin_bad = mv.is_some_and(|mv| {
                    granite_core::config::vin_out_of_window(mv, vin_min_mv, vin_max_mv)
                });
                if vin_bad {
                    faults.fetch_or(fault_bits::VIN, Ordering::Relaxed);
                    if !vin_warned {
                        vin_warned = true;
                        log::warn!(
                            "vin {} mV is outside {}..{} mV",
                            mv.unwrap_or(0),
                            vin_min_mv,
                            vin_max_mv
                        );
                    }
                } else {
                    faults.fetch_and(!fault_bits::VIN, Ordering::Relaxed);
                    vin_warned = false;
                }

                if let Ok(mut observed) = platform.observed.write() {
                    // The LED bits are folded into the node tracker by the
                    // dispatcher; the raw byte goes here so it can.
                    observed.dry_in = Stamped::new(dry, now);
                    observed.board_temp = Stamped::new(temp, now);
                    observed.vin_mv = Stamped::new(mv, now);
                    if let Some(probes) = probe_obs {
                        observed.probes = probes;
                    }
                }
                // The node LED byte goes to the dispatcher, not into the
                // snapshot: the debounce and the Busy overlay live in the
                // node tracker, which only that thread owns.
                LED_BITS.store(
                    leds.map(|b| u16::from(b) | LED_BITS_VALID).unwrap_or(0),
                    Ordering::Relaxed,
                );
                // Modbus fault bit 1: U15 did not answer, so no node LED
                // on the register map can be trusted.
                if leds.is_some() {
                    faults.fetch_and(!fault_bits::SENSE, Ordering::Relaxed);
                } else {
                    faults.fetch_or(fault_bits::SENSE, Ordering::Relaxed);
                }
                thread::sleep(SENSE_TICK);
            }
        });
    if let Err(e) = spawned {
        log::error!("sense thread could not start: {e}");
    }
}

/// The config watcher: the one place where a saved configuration becomes
/// a running (or stopped) server, plus the status page's MQTT box.
///
/// Two jobs, each too small for a thread of its own:
///
/// - `sec.modbus` into [`modbus::ModbusHandle::set_config`], so enabling
///   the server or editing its allow-list takes effect within
///   [`CONFIG_WATCH`] plus [`modbus::POLL`] and never needs a reboot.
///   Hooking the HTTP layer's save path would have been one call site,
///   but it would only catch the HTTP one: the same section is saved by a
///   `config_set` over MQTT and promoted by a confirm. Polling the live
///   config catches all of them, which is how `mqtt.rs` re-reads its own
///   section too (`mqtt::CONFIG_REFRESH`).
/// - the broker state the status page reads: the MQTT worker owns the
///   session and publishes the bit through [`mqtt::connected`], but it
///   never sees [`Platform::mqtt`], which is what `/api/v1/status`
///   answers from.
///
/// It also keeps the two Modbus fault bits that belong to no single task
/// (`mqtt_down`, `ota_pending`) current.
///
/// `try_config`, never `config()`: an HTTP request holds the config write
/// guard for its whole life (see [`dispatcher`]).
fn start_config_watch(
    platform: Arc<Platform>,
    modbus: Option<modbus::ModbusHandle>,
    faults: Arc<AtomicU16>,
) {
    let spawned = thread::Builder::new()
        .name("config-watch".into())
        // 8 KB, not 4: this thread now also parses a stored config section
        // back into the live configuration when a staged one is reverted,
        // and serde_json wants room for that.
        .stack_size(8192)
        .spawn(move || {
            let mut last: Option<granite_core::config::ModbusCfg> = None;
            loop {
                if let Some(cfg) = platform.try_config() {
                    if last.as_ref() != Some(&cfg.sec.modbus) {
                        if let Some(handle) = modbus.as_ref() {
                            handle.set_config(&cfg.sec.modbus);
                        }
                        last = Some(cfg.sec.modbus.clone());
                    }

                    let connected = mqtt::connected();
                    let mut status = platform.mqtt_status();
                    status.enabled = cfg.mqtt.enabled;
                    status.connected = connected;
                    status.broker = if cfg.mqtt.host.is_empty() {
                        String::new()
                    } else {
                        format!("{}:{}", cfg.mqtt.host, cfg.mqtt.port)
                    };
                    platform.set_mqtt_status(status);

                    let mut set = 0u16;
                    let mut clear = 0u16;
                    if cfg.mqtt.enabled && !connected {
                        set |= fault_bits::MQTT_DOWN;
                    } else {
                        clear |= fault_bits::MQTT_DOWN;
                    }
                    if platform::ota::ota_state() == platform::ota::OtaState::Pending {
                        set |= fault_bits::OTA_PENDING;
                    } else {
                        clear |= fault_bits::OTA_PENDING;
                    }
                    faults.fetch_or(set, Ordering::Relaxed);
                    faults.fetch_and(!clear, Ordering::Relaxed);
                }

                // The one staged-section deadline (ADR component 6). `net`
                // reverts by rebooting into the stored configuration -
                // the live interface was already changed, so nothing else
                // is sure. `mqtt` and `sec` only need their stored section
                // back in the live config: the MQTT worker rebuilds its
                // client and the Modbus supervisor rebinds, both within
                // one refresh, and no node loses power for a broker typo.
                let due = platform::api_impl::take_due_staged(platform::now_ms());
                if !due.is_empty() {
                    log::error!(
                        "config: {} was not confirmed in time; putting the stored value back",
                        section_list(&due)
                    );
                    platform::api_impl::revert_sections(&platform, &due);
                }
                // Restores queued by a revert, here because this thread
                // holds no guard: the API cannot take the config lock it
                // is already holding (see platform::api_impl).
                let restore = platform::api_impl::take_restore();
                if !restore.is_empty() && !restore_live_sections(&platform, &restore) {
                    // The config guard was busy; try again next pass.
                    platform::api_impl::queue_restore(&restore);
                }

                thread::sleep(CONFIG_WATCH);
            }
        });
    if let Err(e) = spawned {
        log::error!("config watch thread could not start: {e}");
    }
}

/// Section keys as one readable list for a log line.
fn section_list(sections: &[Section]) -> String {
    sections
        .iter()
        .map(|s| s.key())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Put the *stored* value of these sections back into the live
/// configuration, which is how a `mqtt` or `sec` revert works: no reboot,
/// the owning task notices on its next refresh (ADR component 6).
///
/// False when the config guard was busy, so the caller can retry. Only the
/// config-watch thread calls this: an HTTP request holds that guard for its
/// whole lifetime.
fn restore_live_sections(platform: &Arc<Platform>, sections: &[Section]) -> bool {
    // NVS first, so the config guard is held for the write and nothing
    // else.
    let mut stored: Vec<(Section, String)> = Vec::new();
    match platform.store.lock() {
        Ok(mut store) => {
            for section in sections {
                let json = store
                    .get_str(granite_core::config::CONFIG_NAMESPACE, section.key())
                    .unwrap_or(None)
                    // Nothing stored means the default is what was
                    // running, and every section is `#[serde(default)]`.
                    .unwrap_or_else(|| String::from("{}"));
                stored.push((*section, json));
            }
        }
        Err(_) => return false,
    }

    let Ok(mut live) = platform.config.try_write() else {
        return false;
    };
    for (section, json) in &stored {
        match live.set_section_json(*section, json) {
            Ok(()) => log::warn!("config: {section} restored from the stored value"),
            Err(e) => log::error!("config: {section} could not be restored: {e}"),
        }
    }
    live.normalise();
    drop(live);
    for (section, _) in &stored {
        platform.publish(config_event(
            *section,
            "reverted",
            Some(String::from("not confirmed in time")),
        ));
    }
    true
}

/// The fault pattern, in a build that has an LED able to show one.
///
/// ADR 0001 fixes four meanings for the plain GPIO1 LED and "a fault is
/// present" is not one of them, so the pattern is only ever selected in
/// an image that also carries the RGB mirror, which paints it red. The
/// signal itself is nothing new: the `faults` word is the one the sense
/// thread and the config watcher already keep for Modbus input register
/// 13.
#[cfg(feature = "rgb-led")]
fn fault_pattern(faults: &AtomicU16) -> Option<LedPattern> {
    hw::rgb::for_faults(faults)
}

#[cfg(not(feature = "rgb-led"))]
fn fault_pattern(_faults: &AtomicU16) -> Option<LedPattern> {
    None
}

/// Drive the status LED from the ADR's pattern table (component 13).
fn start_led(platform: Arc<Platform>, mut led: hw::led::LedHandle, faults: Arc<AtomicU16>) {
    let spawned = thread::Builder::new()
        .name("led-pattern".into())
        .stack_size(3072)
        .spawn(move || {
            let mut last = None;
            loop {
                let net = platform.net.status();
                let want = if let Some(fault) = fault_pattern(&faults) {
                    fault
                } else if platform::ota::ota_state() == platform::ota::OtaState::Pending {
                    LedPattern::OtaPending
                } else if !net.link {
                    LedPattern::NoLink
                } else {
                    LedPattern::Heartbeat
                };
                // A press takes the LED solid, and the actuator owns that;
                // only change the pattern when it is not pressing.
                if last != Some(want) && led.pattern() != LedPattern::Press {
                    if let Err(e) = led.set(want) {
                        log::error!("status led: {e}");
                    }
                    last = Some(want);
                }
                thread::sleep(Duration::from_millis(500));
            }
        });
    if let Err(e) = spawned {
        log::error!("led pattern thread could not start: {e}");
    }
}

fn config_event(section: Section, change: &str, detail: Option<String>) -> Event {
    Event::new(
        platform::now_ms(),
        EventKind::Config {
            section: Some(section),
            change: change.to_string(),
            detail,
        },
    )
}
