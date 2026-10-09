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
//!   7. integration points for the hardware layer and the three servers,
//!      marked `// INTEGRATION:`,
//!   8. heartbeat.
//!
//! Pin map: docs/controller.md, section "MCU".

#[allow(unused_imports)]
use granite_fw as _;

use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, RwLock};
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
use granite_core::msg::{Event, EventKind, event_from_actuator, event_from_rule};
use granite_core::node::Nodes;
use granite_core::observed::{Observed, ProbeObs, Stamped};
use granite_core::rules::RuleEngine;
use granite_fw::platform::{self, NullSwitches, Platform};
use granite_fw::{http, hw};

/// Heartbeat interval of the idle loop.
const HEARTBEAT: Duration = Duration::from_secs(10);
/// Dispatcher tick: the actuator state machine and the rule engine both
/// need regular ticks, and a command may arrive between two of them.
const DISPATCH_TICK: Duration = Duration::from_millis(50);
/// Rule engine period (ADR component 5: every 1 s).
const RULES_PERIOD_MS: u64 = 1000;
/// Sense tick: U15, the TMP1075 and VIN are all 1 s in the ADR.
const SENSE_TICK: Duration = Duration::from_secs(1);

/// Marks [`LED_BITS`] as carrying a real reading.
const LED_BITS_VALID: u16 = 0x100;

/// Raw node power-LED byte from U15, handed from the sense thread to the
/// dispatcher, which owns the 100 ms debounce and the `Busy` overlay.
/// Zero means "U15 has not answered"; [`LED_BITS_VALID`] distinguishes
/// that from a genuine "all eight nodes off".
static LED_BITS: AtomicU16 = AtomicU16::new(0);

fn main() -> anyhow::Result<()> {
    // 1. Patches that the ESP-IDF build expects to find linked in.
    esp_idf_svc::sys::link_patches();

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

    // Kept alive: the ESP-IDF event loop outlives a failed Ethernet
    // bring-up and the network thread subscribes to it.
    let sysloop = EspSystemEventLoop::take()?;
    let eth_pins = platform::net::EthPins {
        sclk: pins.gpio19,
        mosi: pins.gpio20,
        miso: pins.gpio21,
        cs: pins.gpio18,
        int: pins.gpio22,
        rst: pins.gpio23,
    };

    // The hardware layer is in this build, so the OTA probation ladder has
    // to wait for "expanders initialised" (step 1). This has to be said
    // before platform::init, which is where the ladder starts.
    platform::ota::expect_expanders();

    let platform = platform::init(
        peripherals.spi2,
        eth_pins,
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

            start_sense(
                Arc::clone(&platform),
                sense,
                probes,
                board_temp,
                vin,
            );
            start_led(Arc::clone(&platform), led);

            thread::Builder::new()
                .name("dispatch".into())
                .stack_size(12288)
                .spawn(move || dispatcher(dispatcher_platform, commands_rx, relays))?;
        }
        Err(e) => {
            log::error!(
                "hardware layer did not come up ({e:#}); relays stay in reset and every press \
                 will be refused"
            );
            thread::Builder::new()
                .name("dispatch".into())
                .stack_size(12288)
                .spawn(move || dispatcher(dispatcher_platform, commands_rx, NullSwitches))?;
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

    // INTEGRATION: mqtt::start(Arc::clone(&platform)) subscribes to
    // platform.events, sends commands through platform.commands, and calls
    // platform::ota::mark_broker() on its first successful connection.
    //
    // INTEGRATION: modbus::start(Arc::clone(&platform)) if
    // config.sec.modbus.enabled and the allow-list is non-empty.
    //
    // None of the three is called yet: their modules are still stubs, and
    // calling into them before their entry points exist would not compile.

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
            cfg = shared.clone();
            drop(shared);
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
        }
    }
}

/// Push the timings, node order, sense modes and rule set from `cfg` into
/// the state machines this thread owns, and the sense modes into the
/// snapshot so a publisher can say "this LED is not trusted" without
/// reading the config.
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
                "config: {section} staged; confirm within {} s or this board reboots into the \
                 previous value",
                cfg.net.t_confirm_s
            );
            platform.publish(config_event(section, "staged", None));
            match section {
                // The net section is the one that can cut the session, so
                // it is applied live with the confirm timer running.
                Section::Net => platform.net.apply(&cfg.net, cfg.net.t_confirm_s),
                // TODO(ADR 0001 6): the broker and security sections are
                // staged but not applied live yet - that needs the MQTT
                // worker's reconnect path (auto_confirm_on_connect) and the
                // HTTP layer's certificate reload, both owned elsewhere.
                // Until then they are promoted by an explicit confirm.
                _ => log::warn!(
                    "config: {section} is staged but applying it live needs the owning task; \
                     confirm promotes it, a reboot discards it"
                ),
            }
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
            if roms.is_empty() {
                match probes.scan() {
                    Ok(found) => {
                        log::info!("probes: {} on the bus", found.len());
                        roms = found
                            .into_iter()
                            .map(|rom| (rom, granite_core::hal::rom_id_hex(rom)))
                            .collect();
                    }
                    Err(e) => log::warn!("probes: bus scan failed: {e}"),
                }
            }
            let probe_period_ms = u64::from(cfg.nodes.t_probe_s.max(1)) * 1000;
            let mut last_probe_ms = 0u64;

            loop {
                let now = platform::now_ms();
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
                thread::sleep(SENSE_TICK);
            }
        });
    if let Err(e) = spawned {
        log::error!("sense thread could not start: {e}");
    }
}

/// Drive the status LED from the ADR's pattern table (component 13).
fn start_led(platform: Arc<Platform>, mut led: hw::led::LedHandle) {
    let spawned = thread::Builder::new()
        .name("led-pattern".into())
        .stack_size(3072)
        .spawn(move || {
            let mut last = None;
            loop {
                let net = platform.net.status();
                let want = if platform::ota::ota_state() == platform::ota::OtaState::Pending {
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
