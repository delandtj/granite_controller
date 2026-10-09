//! The simulated controller: the real core over the fake board, plus the
//! side-effect handling the firmware's main loop does (persisting a
//! confirmed section, reverting an unconfirmed one, factory reset,
//! reboot, probe scan).

use std::time::Instant;

use granite_core::actuator::{Actuator, ActuatorEvent, apply_to_nodes};
use granite_core::api::{ApiCtx, ApiRequest, ApiResponse, AuthState, Dispatcher};
use granite_core::config::{Config, Section, Secrets};
use granite_core::dispatch::{DispatchCtx, SideEffect, dispatch};
use granite_core::msg::{Command, EventKind, Reply, event_from_actuator, event_from_rule};
use granite_core::node::Nodes;
use granite_core::observed::{Observed, ProbeObs};
use granite_core::rules::{RuleAction, RuleEngine};
use granite_core::{NODE_COUNT, NodeId};

use crate::fakes::{FakeSensors, FakeSwitches};
use crate::platform::{SimIdentity, SimNet, SimOta, SimPlatform, SimRng, SimStore};
use crate::scenario::Scenario;

/// How often the simulator ticks. The firmware ticks the actuator on a
/// 10 ms loop and the rules once a second; 20 Hz is plenty on the host
/// and keeps the press timing within a tick of the real thing.
pub const TICK_MS: u64 = 50;

/// The part of the simulator the core owns: the actuator over the fake
/// expander, the node tracker, the rule engine and the snapshot they work
/// from. [`Dispatcher`] is implemented here, which is how the HTTP API
/// reaches [`granite_core::dispatch::dispatch`].
pub struct SimCore {
    /// Device id, needed by `factory_reset`.
    pub device_id: String,
    /// The actuator, owning the fake U14.
    pub actuator: Actuator<FakeSwitches>,
    /// The fake sensors.
    pub sensors: FakeSensors,
    /// LED debounce and per-node state.
    pub nodes: Nodes,
    /// The rule engine.
    pub rules: RuleEngine,
    /// The working snapshot.
    pub observed: Observed,
    /// A copy of the live configuration, synced before every request.
    pub config: Config,
    /// Side effects the dispatcher asked for.
    pub effects: Vec<SideEffect>,
    /// Actuator and rule events, drained by the tick loop.
    pub events: Vec<EventKind>,
    /// Monotonic now.
    pub now_ms: u64,
    /// True once the boot policies have been applied.
    boot_policy_done: bool,
}

impl SimCore {
    /// Build the core from a scenario.
    pub fn new(scenario: &Scenario, config: &Config) -> Self {
        let mut core = SimCore {
            device_id: scenario.device_id.clone(),
            actuator: Actuator::new(FakeSwitches::new(scenario)),
            sensors: FakeSensors::new(scenario),
            nodes: Nodes::new(),
            rules: RuleEngine::with_rules(config.rules.rules.clone()),
            observed: Observed::new(),
            config: config.clone(),
            effects: Vec::new(),
            events: Vec::new(),
            now_ms: 0,
            boot_policy_done: false,
        };
        core.actuator.set_timings(config.nodes.timings);
        core.actuator.set_order(config.nodes.order);
        core.nodes.set_sense(config.nodes.sense_modes());
        core.refresh_probes();
        core
    }

    /// Re-read the configured probe names onto the ROM ids on the bus.
    fn refresh_probes(&mut self) {
        let roms = self.sensors.scan();
        let mut probes = Vec::new();
        for rom in roms {
            let name = self
                .config
                .nodes
                .probes
                .iter()
                .find(|p| granite_core::hal::rom_id_from_hex(&p.rom) == Some(rom))
                .map(|p| p.name.clone())
                .unwrap_or_default();
            probes.push(ProbeObs {
                rom,
                name,
                centi_c: None,
                ts_ms: 0,
            });
        }
        self.observed.probes = probes;
    }

    /// One pass: hardware, sense, actuator, rules.
    pub fn tick(&mut self, now_ms: u64) {
        self.now_ms = now_ms;

        // 1. The fake board moves first (scheduled LED changes, a long
        //    PWR hold cutting power).
        self.actuator.switches_mut().tick(now_ms);
        let leds = self.actuator.switches_mut().leds();

        // 2. Sense, with the debounce the firmware applies.
        self.nodes.update_sense(now_ms, leds);

        // 3. The actuator state machine.
        let events = self.actuator.tick(now_ms, &self.observed);
        apply_to_nodes(&mut self.nodes, &events, now_ms);
        for event in &events {
            if let Some(e) = event_from_actuator(event, now_ms) {
                self.events.push(e.kind);
            }
            if let ActuatorEvent::Fault { fault, .. } = event {
                self.events.push(EventKind::Fault {
                    fault: fault.to_string(),
                    node: None,
                    id: None,
                });
            }
        }

        // 4. The snapshot every reader works from.
        self.observed.refresh_nodes(&self.nodes);
        for i in 0..NODE_COUNT {
            self.observed.nodes[i].sense = self.config.nodes.nodes[i].sense;
        }
        let probes: Vec<(u64, usize)> = self
            .observed
            .probes
            .iter()
            .enumerate()
            .map(|(i, p)| (p.rom, i))
            .collect();
        for (rom, i) in probes {
            let centi = self.sensors.probe_centi_c(rom, now_ms);
            self.observed.probes[i].centi_c = centi;
            self.observed.probes[i].ts_ms = now_ms;
        }
        self.observed.board_temp =
            granite_core::observed::Stamped::new(Some(self.sensors.board_centi_c()), now_ms);
        self.observed.vin_mv =
            granite_core::observed::Stamped::new(Some(self.sensors.vin_mv(now_ms)), now_ms);
        self.observed.dry_in =
            granite_core::observed::Stamped::new(Some(self.sensors.dry_bits()), now_ms);
        self.observed.link_up =
            granite_core::observed::Stamped::new(self.sensors.link_up(), now_ms);
        self.observed.uptime_s =
            granite_core::observed::Stamped::new((now_ms / 1000) as u32, now_ms);

        // 5. Boot policies, once, after the settle time.
        if !self.boot_policy_done && now_ms >= u64::from(self.config.nodes.timings.t_settle_ms) {
            self.boot_policy_done = true;
            let policies = self.config.nodes.boot_policies();
            let (_, events) =
                self.actuator
                    .submit_boot_policy("boot", &policies, now_ms, &self.observed);
            for event in &events {
                if let Some(e) = event_from_actuator(event, now_ms) {
                    self.events.push(e.kind);
                }
            }
        }

        // 6. Rules, once a second.
        if now_ms.is_multiple_of(1_000) {
            let fired = self.rules.tick(now_ms, &self.observed);
            for f in fired {
                self.events.push(event_from_rule(&f, now_ms).kind);
                if let RuleAction::Act { kind } = f.action {
                    let cmd = Command::new(
                        format!("rule-{}", f.rule_id),
                        granite_core::msg::CommandKind::of_action(kind),
                    )
                    .with_target(f.target);
                    let reply = self.dispatch(&cmd);
                    if !reply.ok {
                        self.events.push(EventKind::ActionFailed {
                            id: cmd.id.clone(),
                            node: f.target.node(),
                            action: kind,
                            error: reply.error.unwrap_or_default(),
                        });
                    }
                }
            }
        }
    }

    /// Apply a configuration change to the parts of the core that cache
    /// it (timings, order, sense, probe names, rules).
    pub fn reconfigure(&mut self, config: &Config) {
        self.config = config.clone();
        self.actuator.set_timings(config.nodes.timings);
        self.actuator.set_order(config.nodes.order);
        self.nodes.set_sense(config.nodes.sense_modes());
        self.rules.set_rules(config.rules.rules.clone());
        self.refresh_probes();
    }

    /// Set a node's LED by hand, as a scenario step would.
    pub fn set_led(&mut self, node: NodeId, value: bool) {
        self.actuator.switches_mut().set_led(node, value);
    }
}

impl Dispatcher for SimCore {
    fn dispatch(&mut self, cmd: &Command) -> Reply {
        let mut ctx = DispatchCtx {
            now_ms: self.now_ms,
            device_id: &self.device_id,
            observed: &self.observed,
            actuator: &mut self.actuator,
            rules: &mut self.rules,
            config: &mut self.config,
            verify_sig: None,
        };
        let out = dispatch(cmd, &mut ctx);
        for event in &out.events {
            if let Some(e) = event_from_actuator(event, self.now_ms) {
                self.events.push(e.kind);
            }
        }
        self.effects.extend(out.effects);
        out.reply
    }
}

/// The whole simulator. Every field is borrowed mutably and separately
/// when an `ApiCtx` is built, which is why they are not one struct.
pub struct Sim {
    /// Wall-clock start, for the monotonic clock.
    pub started: Instant,
    /// Monotonic now in milliseconds.
    pub now_ms: u64,
    /// The live configuration.
    pub config: Config,
    /// The published snapshot the API reads.
    pub observed: Observed,
    /// Sessions, lockout, recovery nonce.
    pub auth: AuthState,
    /// In-memory NVS.
    pub store: SimStore,
    /// System state and the log ring.
    pub platform: SimPlatform,
    /// Identity and recovery.
    pub identity: SimIdentity,
    /// Commit-confirm.
    pub net: SimNet,
    /// Firmware upload target.
    pub ota: SimOta,
    /// Randomness.
    pub rng: SimRng,
    /// The core over the fake board.
    pub core: SimCore,
    /// The scenario, kept for a reset.
    scenario: Scenario,
    /// Set when the dispatcher asked for a reboot.
    pub reboot_requested: bool,
}

impl Sim {
    /// Build a simulator for a scenario, serving `cert_pem`.
    pub fn new(scenario: Scenario, cert_pem: &str, key_pem: &str) -> Self {
        let config = Config::default();
        let mut sim = Sim {
            started: Instant::now(),
            now_ms: 0,
            observed: Observed::new(),
            auth: AuthState::new(),
            store: SimStore::default(),
            platform: SimPlatform::new(&scenario),
            identity: SimIdentity::new(&scenario, cert_pem, key_pem),
            net: SimNet::new(config.net.t_confirm_s),
            ota: SimOta::default(),
            rng: SimRng,
            core: SimCore::new(&scenario, &config),
            config,
            scenario,
            reboot_requested: false,
        };
        sim.platform.log_line(
            "info",
            format!(
                "simulator up: device {} mac {}",
                sim.identity.device_id, sim.identity.mac
            ),
        );
        sim.platform.log_line(
            "warn",
            "no admin password set; the API only accepts /id, /recover and the password route",
        );
        sim.tick();
        sim
    }

    /// Advance the simulation to the current wall clock.
    pub fn tick(&mut self) {
        self.now_ms = self.started.elapsed().as_millis() as u64;
        self.platform.now_ms = self.now_ms;
        self.net.now_ms = self.now_ms;
        self.core.tick(self.now_ms);
        self.observed = self.core.observed.clone();
        self.drain_events();
        self.apply_effects();
        self.resolve_staged();
    }

    fn drain_events(&mut self) {
        let events: Vec<EventKind> = std::mem::take(&mut self.core.events);
        for kind in events {
            self.platform.push_event(kind);
        }
    }

    /// Carry out what the dispatcher asked for, the way the firmware's
    /// main loop does.
    fn apply_effects(&mut self) {
        let effects: Vec<SideEffect> = std::mem::take(&mut self.core.effects);
        for effect in effects {
            match effect {
                SideEffect::ProbeScan => {
                    self.core.reconfigure(&self.config.clone());
                    self.platform.log_line("info", "1-wire bus rescanned");
                }
                SideEffect::SaveSection(section) => {
                    self.save_section(section);
                }
                SideEffect::StageSection(section) => {
                    if let Ok(json) = self.config.section_json(section) {
                        let _ = granite_core::api::NetControl::stage_and_apply(
                            &mut self.net,
                            section,
                            &json,
                        );
                    }
                }
                SideEffect::Ota { url, sha256 } => {
                    self.platform.log_line(
                        "warn",
                        format!("pull OTA is not simulated (url {url}, sha256 {sha256})"),
                    );
                }
                SideEffect::Reboot => {
                    self.reboot_requested = true;
                    self.platform
                        .log_line("warn", "reboot requested; the simulator restarts its state");
                    self.reboot();
                }
                SideEffect::FactoryReset => {
                    self.platform.log_line("warn", "factory reset");
                    self.factory_reset();
                }
            }
        }
        if self.platform.rollback_requested {
            self.platform.rollback_requested = false;
            self.platform.log_line("warn", "rollback to the previous slot");
            self.reboot();
        }
    }

    /// Persist a confirmed change, or put the previous configuration back
    /// when the confirm window ran out.
    fn resolve_staged(&mut self) {
        if self.net.confirmed {
            self.net.confirmed = false;
            let staged: Vec<(Section, String)> = self.net.staged.clone();
            for (section, json) in staged {
                let _ = granite_core::api::Store::save_section(&mut self.store, section, &json);
            }
            self.net.clear();
            self.platform.log_line("info", "configuration confirmed");
            self.core.reconfigure(&self.config.clone());
            return;
        }
        if self.net.reverted || self.net.expired() {
            if self.net.staged.is_empty() {
                self.net.reverted = false;
                return;
            }
            let why = if self.net.reverted {
                "reverted on request"
            } else {
                "confirm window expired, reverting"
            };
            self.net.reverted = false;
            if let Some(previous) = self.net.previous.clone() {
                self.config = previous;
            }
            self.net.clear();
            self.platform.log_line("warn", why);
            self.platform.push_event(EventKind::Config {
                section: None,
                change: String::from("reverted"),
                detail: Some(String::from(why)),
            });
            self.core.reconfigure(&self.config.clone());
        }
    }

    fn save_section(&mut self, section: Section) {
        if let Ok(json) = self.config.section_json(section) {
            let _ = granite_core::api::Store::save_section(&mut self.store, section, &json);
        }
    }

    /// Restart: the configuration survives, node power is untouched.
    fn reboot(&mut self) {
        let config = self.config.clone();
        self.auth.close_all();
        self.core = SimCore::new(&self.scenario, &config);
        // The LEDs keep the state the fake board had: a controller
        // restart must not move a node.
        for node in 1..=NODE_COUNT as NodeId {
            let lit = self.observed.node_led(node).unwrap_or(false);
            self.core.set_led(node, lit);
        }
        self.net.clear();
        self.platform.ota.pending_verify = false;
    }

    /// Erase everything but the factory namespace.
    fn factory_reset(&mut self) {
        self.config = Config::default();
        self.store.sections.clear();
        self.store.secrets = Secrets::default();
        self.auth.close_all();
        self.net.clear();
        // The factory namespace survives: recovery token, fleet key,
        // device certificate.
        self.identity.token_shown = false;
        self.core.reconfigure(&self.config.clone());
        self.platform.push_event(EventKind::Config {
            section: None,
            change: String::from("factory_reset"),
            detail: None,
        });
    }

    /// Is this credential good? The transport needs to know before it
    /// streams a firmware image into the sink.
    pub fn is_authenticated(&mut self, req: &ApiRequest) -> bool {
        let mut ctx = ApiCtx {
            now_ms: self.now_ms,
            config: &mut self.config,
            observed: &self.observed,
            auth: &mut self.auth,
            store: &mut self.store,
            platform: &mut self.platform,
            identity: &mut self.identity,
            net: &mut self.net,
            ota: &mut self.ota,
            rng: &mut self.rng,
            commands: &mut self.core,
        };
        ctx.password_set() && ctx.is_authenticated(req)
    }

    /// Push an image into the OTA sink in chunks, as the ESP-IDF handler
    /// does, and report what happened.
    pub fn stream_to_ota(&mut self, image: &[u8]) -> granite_core::api::UploadOutcome {
        use granite_core::api::OtaSink as _;
        let mut outcome = granite_core::api::UploadOutcome {
            bytes: image.len() as u64,
            error: None,
        };
        if let Err(e) = self.ota.begin(Some(image.len() as u64)) {
            outcome.error = Some(e);
            return outcome;
        }
        for chunk in image.chunks(4096) {
            if let Err(e) = self.ota.write(chunk) {
                outcome.error = Some(e);
                return outcome;
            }
        }
        outcome
    }

    /// Route one request through [`granite_core::api::handle`].
    pub fn handle(&mut self, req: ApiRequest) -> ApiResponse {
        self.tick();
        self.core.config = self.config.clone();
        let before_staged = self.net.staged.is_empty();
        let snapshot = self.config.clone();
        let response = {
            let mut ctx = ApiCtx {
                now_ms: self.now_ms,
                config: &mut self.config,
                observed: &self.observed,
                auth: &mut self.auth,
                store: &mut self.store,
                platform: &mut self.platform,
                identity: &mut self.identity,
                net: &mut self.net,
                ota: &mut self.ota,
                rng: &mut self.rng,
                commands: &mut self.core,
            };
            granite_core::api::handle(req, &mut ctx)
        };
        // A staged change needs the configuration as it was, so the
        // automatic revert has something to go back to.
        if before_staged && !self.net.staged.is_empty() {
            self.net.previous = Some(snapshot);
        }
        if let Some(done) = self.ota.finished.take() {
            if let Some(slot) = self
                .platform
                .ota
                .slots
                .iter_mut()
                .find(|s| s.label == done.slot)
            {
                slot.state = String::from("pending_verify");
                slot.version = String::from("uploaded");
            }
            self.platform.ota.rollback_available = true;
            self.platform.log_line(
                "info",
                format!("{} bytes written to {}", done.bytes, done.slot),
            );
        }
        self.core.reconfigure(&self.config.clone());
        self.apply_effects();
        self.resolve_staged();
        self.drain_events();
        response
    }
}
