//! The simulated controller: the real core over the fake board, plus the
//! side-effect handling the firmware's main loop does (persisting a
//! confirmed section, reverting an unconfirmed one, factory reset,
//! reboot, probe scan).

use std::time::Instant;

use granite_core::api::{ApiCtx, ApiRequest, ApiResponse, AuthState, Dispatcher};
use granite_core::config::{Config, Secrets, Section};
use granite_core::dispatch::{DispatchCtx, SideEffect, dispatch};
use granite_core::jobs::JobLog;
use granite_core::msg::{
    Command, CommandKind, EventKind, Reply, event_from_client, event_from_rule,
};
use granite_core::node::BootPolicy;
use granite_core::observed::{Observed, ProbeObs};
use granite_core::plane::{PlaneClient, UpdatePolicy};
use granite_core::rules::RuleEngine;
use granite_core::{NodeId, Target};

use crate::fakes::FakeSensors;
use crate::planes::FakePlaneLink;
use crate::platform::{SimIdentity, SimNet, SimOta, SimPlatform, SimRng, SimStore};
use crate::scenario::Scenario;

/// How often the simulator ticks.
///
/// The firmware polls the plane client every 50-100 ms and the rules
/// once a second; 20 Hz on the host is the same cadence, and it matches
/// the planes' own 20 Hz cycle.
pub const TICK_MS: u64 = 50;

/// How long the boot policies wait after start before they are applied
/// \[ms\]: long enough for discovery and the first SUMMARY of every
/// plane.
pub const BOOT_POLICY_AFTER_MS: u64 = 3_000;

/// The part of the simulator the core owns: the plane client over the
/// fake plane link, the job log, the rule engine and the snapshot they
/// work from. [`Dispatcher`] is implemented here, which is how the HTTP
/// API reaches [`granite_core::dispatch::dispatch`].
pub struct SimCore {
    /// Device id, needed by `factory_reset`.
    pub device_id: String,
    /// The controller's side of the plane link (ADR 0004 decision 5).
    pub plane: PlaneClient,
    /// The plane link and the planes on it. A plane is a separate MCU:
    /// it runs whether or not the controller asks it anything.
    pub planes: FakePlaneLink,
    /// The jobs a command became, waiting for the plane's result.
    pub jobs: JobLog,
    /// The fake sensors of the controller board.
    pub sensors: FakeSensors,
    /// The rule engine.
    pub rules: RuleEngine,
    /// The working snapshot.
    pub observed: Observed,
    /// A copy of the live configuration, synced before every request.
    pub config: Config,
    /// Side effects the dispatcher asked for.
    pub effects: Vec<SideEffect>,
    /// Events, drained by the tick loop into the MQTT stream.
    pub events: Vec<EventKind>,
    /// Acks of finished jobs, newest last: the simulator's `.../ack/`
    /// topic.
    pub acks: Vec<Reply>,
    /// Monotonic now.
    pub now_ms: u64,
    /// True once the boot policies have been applied.
    boot_policy_done: bool,
    /// The last state of each node, so a change can be published.
    last_states: Vec<(NodeId, String, String)>,
}

impl SimCore {
    /// Build the core from a scenario.
    pub fn new(scenario: &Scenario, config: &Config) -> Self {
        let mut core = SimCore {
            device_id: scenario.device_id.clone(),
            // A fixed seed: the sequence numbers have to be
            // reproducible for the tests, and the firmware is the one
            // that passes something from its RNG.
            plane: PlaneClient::new(0x5EED_1234),
            planes: FakePlaneLink::from_scenario(scenario),
            jobs: JobLog::new(),
            sensors: FakeSensors::new(scenario),
            rules: RuleEngine::with_rules(config.rules.rules.clone()),
            observed: Observed::new(),
            config: config.clone(),
            effects: Vec::new(),
            events: Vec::new(),
            acks: Vec::new(),
            now_ms: 0,
            boot_policy_done: false,
            last_states: Vec::new(),
        };
        core.plane.set_policy(config.nodes.plane_update);
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

    /// One pass: the planes, the plane client, the sensors, the
    /// snapshot, the boot policies and the rules.
    pub fn tick(&mut self, now_ms: u64) {
        self.now_ms = now_ms;

        // 1. The planes move first: they are their own MCUs and run on
        //    their own clock.
        self.planes.tick(now_ms);

        // 2. The plane client does whatever is due: discovery, the poll
        //    schedule, EXP_INT, the commands in flight, the config push
        //    and an update (ADR 0004 decision 5).
        self.plane.poll(now_ms, &mut self.planes);

        // 3. The plane's events and the jobs it finished.
        for event in self.plane.drain_events() {
            if let Some(published) = event_from_client(&event, now_ms) {
                self.events.push(published.kind);
            }
        }
        for finished in self.plane.drain_jobs() {
            // A command that fanned out over `all` is acked once, when
            // the last of its jobs has reported.
            let done = self.jobs.finish(&finished, now_ms);
            if let Some(event) = done.event {
                self.events.push(event);
            }
            if let Some(reply) = done.reply {
                self.acks.push(reply);
            }
        }

        // 4. The snapshot every reader works from.
        self.observed.refresh_planes(&self.plane);
        self.jobs.apply(&mut self.observed);
        self.read_sensors(now_ms);
        self.note_state_changes();

        // 5. Boot policies, once, after discovery has had its chance.
        if !self.boot_policy_done && now_ms >= BOOT_POLICY_AFTER_MS {
            self.boot_policy_done = true;
            self.apply_boot_policies();
        }

        // 6. Rules, once a second.
        if now_ms.is_multiple_of(1_000) {
            let fired = self.rules.tick(now_ms, &self.observed);
            for f in fired {
                self.events.push(event_from_rule(&f, now_ms).kind);
                if let Some(cmd) = f.command() {
                    let reply = self.dispatch(&cmd);
                    if !reply.ok {
                        self.events.push(EventKind::ActionFailed {
                            id: cmd.id.clone(),
                            node: f.target.node(),
                            action: cmd.action,
                            error: reply.error.unwrap_or_default(),
                        });
                    }
                }
            }
        }
    }

    /// Read the controller's own sensors into the snapshot.
    fn read_sensors(&mut self, now_ms: u64) {
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
    }

    /// Publish a `node_state` event for every node whose axes moved.
    fn note_state_changes(&mut self) {
        let now: Vec<(NodeId, String, String)> = self
            .observed
            .nodes
            .iter()
            .map(|n| {
                (
                    n.node,
                    String::from(n.power.as_str()),
                    String::from(n.host.as_str()),
                )
            })
            .collect();
        for (node, power, host) in &now {
            let before = self.last_states.iter().find(|(id, ..)| id == node);
            let changed = match before {
                None => true,
                Some((_, p, h)) => p != power || h != host,
            };
            if changed {
                self.events.push(EventKind::NodeState {
                    node: *node,
                    power: power.clone(),
                    host: host.clone(),
                });
            }
        }
        self.last_states = now;
    }

    /// The boot policies of ADR 0004 decision 2, once per controller
    /// start: a node whose policy is not `leave` is commanded.
    fn apply_boot_policies(&mut self) {
        for (node, policy) in self.config.nodes.boot_policies() {
            let action = match policy {
                BootPolicy::Leave => continue,
                BootPolicy::On => CommandKind::On,
                BootPolicy::Off => CommandKind::Off,
            };
            let cmd = Command::new(format!("boot-{node}"), action).with_target(Target::Node(node));
            let reply = self.dispatch(&cmd);
            if !reply.ok {
                self.events.push(EventKind::ActionFailed {
                    id: cmd.id.clone(),
                    node: Some(node),
                    action,
                    error: reply.error.unwrap_or_default(),
                });
            }
        }
    }

    /// Apply a configuration change to the parts of the core that cache
    /// it (the update policy, the probe names, the rules). The per-node
    /// settings are read when a command is dispatched, so nothing has
    /// to be pushed for them.
    pub fn reconfigure(&mut self, config: &Config) {
        self.config = config.clone();
        self.plane.set_policy(config.nodes.plane_update);
        self.rules.set_rules(config.rules.rules.clone());
        self.refresh_probes();
    }

    /// The update policy the client is running under.
    pub fn update_policy(&self) -> UpdatePolicy {
        self.plane.policy()
    }
}

impl Dispatcher for SimCore {
    fn dispatch(&mut self, cmd: &Command) -> Reply {
        let mut ctx = DispatchCtx {
            now_ms: self.now_ms,
            device_id: &self.device_id,
            observed: &self.observed,
            plane: &mut self.plane,
            jobs: &mut self.jobs,
            rules: &mut self.rules,
            config: &mut self.config,
            verify_sig: None,
        };
        let out = dispatch(cmd, &mut ctx);
        if let Some(job) = out.job() {
            self.events.push(EventKind::Accepted {
                id: cmd.id.clone(),
                action: cmd.action,
                target: cmd.target,
                job,
            });
        }
        self.effects.extend(out.effects);
        out.reply
    }

    /// `?wait=1`: keep ticking until the plane reports this command's
    /// job, or until `timeout_ms` of wall clock has passed.
    ///
    /// The simulator's HTTP handler holds the whole `Sim` while it runs,
    /// so nothing else can advance the clock: this loop does it itself,
    /// sleeping a tick at a time so the fake board moves in real time
    /// the way it does for any other request.
    fn wait_for(&mut self, cmd: &Command, timeout_ms: u32) -> Option<Reply> {
        let deadline = Instant::now() + std::time::Duration::from_millis(u64::from(timeout_ms));
        loop {
            if let Some(reply) = self.finished_reply(&cmd.id) {
                return Some(reply);
            }
            if Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(std::time::Duration::from_millis(TICK_MS));
            let now = self.now_ms + TICK_MS;
            self.tick(now);
        }
    }
}

impl SimCore {
    /// The final ack for `id`, if the plane has reported its job.
    /// Scans and does not consume: the ack list is the simulator's
    /// `.../ack/` topic and still has to be published.
    pub fn finished_reply(&self, id: &str) -> Option<Reply> {
        self.acks.iter().rev().find(|r| r.id == id).cloned()
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
    /// The core over the fake board and the planes.
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
        // The core ticks the planes itself: they run on their own clock
        // and the controller sees whatever they have got to by now.
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
            self.platform
                .log_line("warn", "rollback to the previous slot");
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
    ///
    /// The planes are not rebuilt either. ADR 0004 ("the controller
    /// reboots, does any node change state?") answers no: the planes
    /// hold their own state and sequences, and the controller
    /// rediscovers them.
    fn reboot(&mut self) {
        let config = self.config.clone();
        self.auth.close_all();
        // The planes are not rebuilt: ADR 0004 ("the controller
        // reboots, does any node change state?") answers no. They keep
        // running, the fresh client rediscovers them, and whatever a
        // node was doing it keeps doing.
        let planes = std::mem::replace(&mut self.core.planes, FakePlaneLink::new());
        self.core = SimCore::new(&self.scenario, &config);
        self.core.planes = planes;
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

    /// The plane link and the planes on it.
    pub fn planes(&self) -> &FakePlaneLink {
        &self.core.planes
    }

    /// The plane link, mutably: what a scenario fault injection uses.
    pub fn planes_mut(&mut self) -> &mut FakePlaneLink {
        &mut self.core.planes
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
