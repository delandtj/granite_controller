//! The dispatcher of ADR 0004 decision 6: every transport funnels
//! through it, it checks the target, submits a plane job and answers
//! `accepted`.
//!
//! The job itself never reaches a plane here - there is no link in this
//! test - so the plane client finishes it as `absent` straight away.
//! What this file pins down is the vocabulary, the target checks and the
//! bookkeeping; the end-to-end path (command, plane job, ack) runs in
//! granite-sim against the fake planes.

use granite_core::config::Config;
use granite_core::dispatch::{DispatchCtx, SideEffect, dispatch};
use granite_core::jobs::JobLog;
use granite_core::modbus_map::{cmd_word, command_from_coil, command_from_holding};
use granite_core::msg::{Args, Command, CommandKind, Reply};
use granite_core::node::{Escalation, NodeSettings};
use granite_core::observed::{NodeObs, Observed, PlaneObs, UnitObs};
use granite_core::plane::model::{HostState, PowerState};
use granite_core::plane::{Plane, PlaneClient, UpdatePolicy};
use granite_core::rules::RuleEngine;
use granite_core::{NodeId, Target};

/// A controller with one plane on it, as the snapshot sees it.
struct Harness {
    observed: Observed,
    plane: PlaneClient,
    jobs: JobLog,
    rules: RuleEngine,
    config: Config,
    now_ms: u64,
}

impl Harness {
    fn new() -> Self {
        let mut h = Harness {
            observed: Observed::new(),
            plane: PlaneClient::new(1),
            jobs: JobLog::new(),
            rules: RuleEngine::with_rules(Config::default().rules.rules),
            config: Config::default(),
            now_ms: 1_000,
        };
        h.add_plane(0);
        h.add_node(1, true);
        h.add_node(2, true);
        h
    }

    /// A plane that answers, with unit 0 on the chain.
    fn add_plane(&mut self, number: u8) {
        let mut plane = PlaneObs::of_plane(&Plane::new(0x30 + number));
        plane.info_ms = 10;
        plane.ts_ms = 20;
        plane.units_present = 0b0001;
        plane.units = (0..4)
            .map(|unit| UnitObs {
                unit,
                present: unit == 0,
                ..UnitObs::default()
            })
            .collect();
        self.observed.planes.push(plane);
    }

    /// A node with a module in its slot.
    fn add_node(&mut self, node: NodeId, present: bool) {
        let (address, slot) = granite_core::plane::node_location(node).expect("a node id");
        let mut obs = NodeObs::unknown(node, address, slot);
        obs.present = present;
        obs.power = if present {
            PowerState::On
        } else {
            PowerState::Absent
        };
        obs.host = HostState::On;
        obs.ts_ms = 20;
        self.observed.nodes.push(obs);
        self.observed.nodes.sort_by_key(|n| n.node);
    }

    fn run(&mut self, cmd: &Command) -> (Reply, Vec<SideEffect>) {
        let mut ctx = DispatchCtx {
            now_ms: self.now_ms,
            device_id: "granite-test",
            observed: &self.observed,
            plane: &mut self.plane,
            jobs: &mut self.jobs,
            rules: &mut self.rules,
            config: &mut self.config,
            verify_sig: None,
        };
        let out = dispatch(cmd, &mut ctx);
        (out.reply, out.effects)
    }

    fn verb(&mut self, action: CommandKind, target: Target) -> Reply {
        let cmd = Command::new("c-1", action).with_target(target);
        self.run(&cmd).0
    }
}

#[test]
fn a_node_verb_becomes_a_plane_job_and_answers_accepted() {
    let mut h = Harness::new();
    for action in [
        CommandKind::On,
        CommandKind::Off,
        CommandKind::ForceOff,
        CommandKind::Reset,
        CommandKind::Cycle,
        CommandKind::Press,
        CommandKind::PowerOn,
        CommandKind::PowerOff,
        CommandKind::ClearLatch,
    ] {
        let reply = h.verb(action, Target::Node(1));
        assert!(reply.ok, "{action} was refused: {:?}", reply.error);
        assert_eq!(reply.result.as_deref(), Some("accepted"), "{action}");
        assert!(reply.job.is_some(), "{action} carries its job id");
    }
    // on_all is a group and takes every present node.
    let reply = h.verb(CommandKind::OnAll, Target::All);
    assert!(reply.ok);
    assert!(reply.job.is_some());
}

#[test]
fn the_plane_and_unit_verbs_need_their_own_targets() {
    let mut h = Harness::new();
    let reboot = h.verb(CommandKind::PlaneReboot, Target::Plane(0));
    assert!(reboot.ok, "{:?}", reboot.error);

    let unit = h.verb(CommandKind::UnitReset, Target::Unit { plane: 0, unit: 0 });
    assert!(unit.ok, "{:?}", unit.error);

    // Unit 1 is not on the chain.
    let absent = h.verb(CommandKind::UnitReset, Target::Unit { plane: 0, unit: 1 });
    assert!(!absent.ok);
    assert!(absent.error.unwrap().contains("not present"));

    // A node verb with a plane target is a refusal, not a guess.
    let wrong = h.verb(CommandKind::On, Target::Plane(0));
    assert!(!wrong.ok);
    assert!(wrong.error.unwrap().contains("single node target"));

    // And a plane verb for a plane that does not answer.
    let gone = h.verb(CommandKind::PlaneReboot, Target::Plane(3));
    assert!(!gone.ok);
}

#[test]
fn an_absent_node_fails_at_once() {
    let mut h = Harness::new();
    h.add_node(5, false);

    let absent = h.verb(CommandKind::On, Target::Node(5));
    assert!(!absent.ok, "a node whose slot is empty is refused");
    assert!(absent.job.is_none(), "nothing was submitted");

    // Node 9 is on plane 1, which does not answer at all.
    let no_plane = h.verb(CommandKind::On, Target::Node(9));
    assert!(!no_plane.ok);
    assert!(no_plane.error.unwrap().contains("plane 1"));

    // A node id that is not one.
    let nonsense = h.verb(CommandKind::On, Target::Node(200));
    assert!(!nonsense.ok);
}

#[test]
fn a_job_is_remembered_so_the_ack_finds_its_command() {
    let mut h = Harness::new();
    let reply = h.verb(CommandKind::Off, Target::Node(2));
    let job = reply.job.expect("a job id");
    // The client has no link, so it finished the job as absent at once;
    // the log still knows whose job it was until the firmware drains it.
    let pending = h.jobs.pending(job).expect("the job is in the log");
    assert_eq!(pending.command_id, "c-1");
    assert_eq!(pending.verb, CommandKind::Off);
    assert_eq!(pending.nodes, vec![2]);
    assert_eq!(h.jobs.job_of("c-1"), Some(job));

    let finished = h.plane.drain_jobs();
    assert_eq!(finished.len(), 1, "the job ended without a plane");
    let done = h.jobs.finish(&finished[0], h.now_ms + 10);
    assert_eq!(done.reply.id, "c-1");
    assert!(!done.reply.ok, "an absent plane is not a success");
    assert!(h.jobs.last(2).and_then(|l| l.result.clone()).is_some());
}

#[test]
fn the_node_settings_shape_the_verb() {
    let mut h = Harness::new();
    h.config.nodes.nodes.push(NodeSettings {
        node: 1,
        led_sense: false,
        t_soft_off_s: 30,
        escalation: Escalation::None,
        ..NodeSettings::for_node(1)
    });
    // Nothing is observable from here but the acceptance: what the
    // settings produce on the wire is checked in granite-core's command
    // tests and end to end in granite-sim.
    let reply = h.verb(CommandKind::Off, Target::Node(1));
    assert!(reply.ok);
    assert_eq!(h.config.nodes.settings(1).t_soft_off_s, 30);
    assert_eq!(h.config.nodes.settings(1).escalation, Escalation::None);
    assert!(!h.config.nodes.settings(1).led_sense);
    // A node with no entry of its own gets the defaults.
    assert_eq!(
        h.config.nodes.settings(7).escalation,
        Escalation::HoldThenCut
    );
    assert_eq!(h.config.nodes.name_of(7), "node7");
}

#[test]
fn the_non_plane_commands_still_work() {
    let mut h = Harness::new();

    let scan = h.run(&Command::new("c-scan", CommandKind::ProbeScan));
    assert!(scan.0.ok);
    assert_eq!(scan.1, vec![SideEffect::ProbeScan]);

    let reboot = h.run(&Command::new("c-rb", CommandKind::Reboot));
    assert_eq!(reboot.1, vec![SideEffect::Reboot]);

    let bad_factory = h.run(&Command::new("c-fr", CommandKind::FactoryReset));
    assert!(!bad_factory.0.ok);
    let factory = h.run(
        &Command::new("c-fr2", CommandKind::FactoryReset).with_args(Args {
            confirm: Some(String::from("granite-test")),
            ..Args::default()
        }),
    );
    assert_eq!(factory.1, vec![SideEffect::FactoryReset]);

    let get = h.run(&Command::new("c-get", CommandKind::ConfigGet));
    assert!(get.0.ok && get.0.data.is_some());

    let ack = h.run(
        &Command::new("c-ack", CommandKind::RuleAck).with_args(Args {
            rule: Some(1),
            ..Args::default()
        }),
    );
    assert!(ack.0.ok, "rule 1 is one of the defaults");

    let version = Command {
        v: 99,
        ..Command::new("c-v", CommandKind::On)
    };
    assert!(!h.run(&version).0.ok, "an unknown payload version");
}

#[test]
fn the_plane_update_policy_lives_in_the_nodes_section() {
    let mut h = Harness::new();
    assert_eq!(h.config.nodes.plane_update, UpdatePolicy::Manual);
    h.config.nodes.plane_update = UpdatePolicy::Auto;
    let json = h
        .config
        .section_json(granite_core::config::Section::Nodes)
        .expect("the nodes section");
    assert!(json.contains("\"plane_update\":\"auto\""), "{json}");
}

#[test]
fn modbus_writes_build_the_same_commands() {
    let on = command_from_holding(100, cmd_word::ON, "modbus-1")
        .expect("node 1's command word")
        .expect("a command");
    assert_eq!(on.action, CommandKind::On);
    assert_eq!(on.target, Target::Node(1));

    let coil = command_from_coil(100 + 8, true, "modbus-2")
        .expect("node 9's coil")
        .expect("a command");
    assert_eq!(coil.action, CommandKind::On);
    assert_eq!(coil.target, Target::Node(9));

    let mut h = Harness::new();
    let reply = h.run(&on).0;
    assert!(
        reply.ok,
        "a Modbus-born command goes through the dispatcher"
    );
}
