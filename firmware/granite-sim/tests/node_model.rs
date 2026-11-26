//! The node model of controller ADR 0004 end to end: a command from
//! each surface becomes a plane job, the plane runs it, and the ack
//! comes back on the job's own id.
//!
//! The whole stack is real here - dispatcher, plane client, wire format,
//! plane firmware core - and only the I2C lines and the board sensors
//! are fake. The clock is fake too, so a soft off that waits two
//! minutes takes a millisecond of test time.

use granite_core::api::Dispatcher;
use granite_core::config::Config;
use granite_core::modbus_map::{
    FwVersion, Image, NODE_CMD_BASE, PLANE_CMD_BASE, ResultCode, cmd_word, plane_cmd_word,
};
use granite_core::modbus_server::{MBAP_HEADER, handle_frame};
use granite_core::msg::{Command, CommandKind, EventKind, Reply};
use granite_core::node::{BootPolicy, Escalation, NodeSettings};
use granite_core::plane::model::{HostState, PowerState};
use granite_core::{NodeId, Target};
use granite_sim::Scenario;
use granite_sim::scenario::{
    FaultKind, PlaneFault, PlaneRespond, PlaneScenario, PlaneSlotScenario,
};
use granite_sim::sim::{SimCore, TICK_MS};

/// A simulated controller with one plane and a fake clock.
struct Rig {
    core: SimCore,
    now_ms: u64,
}

impl Rig {
    /// One plane at 0x30 with unit 0 and unit 1 on the chain: nodes 1-4.
    fn new() -> Self {
        Rig::with_scenario(scenario(), Config::default())
    }

    fn with_scenario(scenario: Scenario, config: Config) -> Self {
        let mut rig = Rig {
            core: SimCore::new(&scenario, &config),
            now_ms: 0,
        };
        // Let discovery, the first SUMMARY and the boot policies happen.
        rig.run(5_000);
        rig
    }

    /// Advance the simulation by `ms`.
    fn run(&mut self, ms: u64) {
        let until = self.now_ms + ms;
        while self.now_ms < until {
            self.now_ms += TICK_MS;
            self.core.tick(self.now_ms);
        }
    }

    /// Run until `f` holds, at most `ms`.
    fn run_until<F: Fn(&SimCore) -> bool>(&mut self, ms: u64, f: F) -> bool {
        let until = self.now_ms + ms;
        while self.now_ms < until {
            if f(&self.core) {
                return true;
            }
            self.now_ms += TICK_MS;
            self.core.tick(self.now_ms);
        }
        f(&self.core)
    }

    /// Send one command the way MQTT does: the JSON payload of the
    /// `.../cmd` topic.
    fn mqtt(&mut self, payload: &str) -> Reply {
        let cmd = Command::from_json(payload).expect("a command payload");
        self.core.dispatch(&cmd)
    }

    /// Send one command the way the HTTP API does.
    fn verb(&mut self, id: &str, action: CommandKind, target: Target) -> Reply {
        let cmd = Command::new(id, action).with_target(target);
        self.core.dispatch(&cmd)
    }

    /// Write one Modbus holding register, the way a PLC does.
    fn modbus_write(&mut self, addr: u16, value: u16) -> (Vec<u8>, Option<ResultCode>) {
        let mut pdu = vec![6u8];
        pdu.extend_from_slice(&addr.to_be_bytes());
        pdu.extend_from_slice(&value.to_be_bytes());
        let mut frame = vec![0x00, 0x01, 0x00, 0x00];
        frame.extend_from_slice(&((pdu.len() + 1) as u16).to_be_bytes());
        frame.push(1);
        frame.extend_from_slice(&pdu);

        // The image borrows a snapshot, the dispatcher needs the core:
        // the server on the board holds the snapshot behind its own
        // lock, so a copy here is the same arrangement.
        let snapshot = self.core.observed.clone();
        let image = Image::new(&snapshot, FwVersion::default(), 0, ResultCode::Idle);
        let mut seq = 0u32;
        let out = handle_frame(1, &frame, &image, &mut seq, &mut |cmd| {
            self.core.dispatch(cmd)
        });
        assert!(!out.broken, "the frame should be well formed");
        (out.response[MBAP_HEADER + 1..].to_vec(), out.result)
    }

    /// Wait for the ack of a command id and give it back.
    fn ack(&mut self, id: &str, ms: u64) -> Reply {
        let want = String::from(id);
        let got = self.run_until(ms, |core| core.finished_reply(&want).is_some());
        assert!(got, "no ack for {id} within {ms} ms");
        self.core.finished_reply(id).expect("checked")
    }

    fn power(&self, node: NodeId) -> PowerState {
        self.core.observed.node_power(node)
    }

    fn host(&self, node: NodeId) -> HostState {
        self.core.observed.node_host(node)
    }

    /// Every `node_state` event the simulator published.
    fn state_events(&self) -> Vec<(NodeId, String, String)> {
        self.core
            .events
            .iter()
            .filter_map(|e| match e {
                EventKind::NodeState { node, power, host } => {
                    Some((*node, power.clone(), host.clone()))
                }
                _ => None,
            })
            .collect()
    }
}

/// One plane at 0x30, units 0 and 1, every host dark.
fn scenario() -> Scenario {
    let mut scenario = Scenario {
        planes: vec![PlaneScenario {
            address: 0x30,
            units: vec![0, 1],
            // A short blink window, so waiting for a host LED is quick.
            blink_window_ms: Some([100, 500]),
            slots: (0..4)
                .map(|slot| PlaneSlotScenario {
                    slot,
                    module: true,
                    host_on: false,
                    ..PlaneSlotScenario::default()
                })
                .collect(),
            faults: Vec::new(),
            ..Default::default()
        }],
        ..Scenario::default()
    };
    scenario.normalise();
    scenario
}

#[test]
fn the_plane_shows_up_as_nodes_one_to_four() {
    let rig = Rig::new();
    let nodes: Vec<NodeId> = rig.core.observed.present_nodes();
    assert_eq!(nodes, vec![1, 2, 3, 4], "two units, four modules");
    assert_eq!(rig.core.observed.planes.len(), 1);
    let plane = &rig.core.observed.planes[0];
    assert_eq!(plane.address, 0x30);
    assert_eq!(plane.units_present, 0b0011);
    assert!(plane.units[0].present && !plane.units[2].present);
    // Both axes are known: the rail is up and the hosts are dark.
    assert_eq!(rig.power(1), PowerState::On);
    assert_eq!(rig.host(1), HostState::Off);
    // And every node reported its state once.
    let states = rig.state_events();
    assert!(
        states
            .iter()
            .any(|(node, power, _)| *node == 1 && power == "on"),
        "{states:?}"
    );
}

#[test]
fn an_mqtt_command_becomes_a_plane_job_and_is_acked() {
    let mut rig = Rig::new();
    let reply = rig.mqtt(r#"{"v":1,"id":"c-42","action":"on","target":2}"#);
    assert!(reply.ok, "{:?}", reply.error);
    assert_eq!(reply.result.as_deref(), Some("accepted"));
    let job = reply.job.expect("a job id");

    let ack = rig.ack("c-42", 60_000);
    assert!(ack.ok, "{:?}", ack.error);
    assert_eq!(ack.result.as_deref(), Some("done"));
    assert_eq!(ack.job, Some(job), "the ack carries the same job");
    assert_eq!(rig.host(2), HostState::On, "the host answered its button");

    // The node's last job is on the snapshot, which is what a dashboard
    // polls.
    let last = rig
        .core
        .observed
        .node(2)
        .and_then(|n| n.last_job.clone())
        .expect("a last job");
    assert_eq!(last.id, "c-42");
    assert_eq!(last.verb, "on");
    assert_eq!(last.result.as_deref(), Some("done"));
    assert!(last.finished_ms.is_some());
}

/// Bug: `on_all` cut the rail of every present node, running ones
/// included, and staggered them back on. It is a staggered `on` (ADR
/// 0004 decision 4): a node that runs is not touched, and a node whose
/// rail is up but whose host is dark keeps its rail and gets its press.
#[test]
fn on_all_does_not_cut_a_node_that_runs() {
    let mut rig = Rig::new();
    // Bring node 1 up: rail on, host on.
    let reply = rig.verb("c-on1", CommandKind::On, Target::Node(1));
    assert!(reply.ok, "{:?}", reply.error);
    rig.ack("c-on1", 60_000);
    assert_eq!(rig.host(1), HostState::On);
    // Node 2's rail off, so the stagger has something to release.
    let reply = rig.verb("c-off2", CommandKind::PowerOff, Target::Node(2));
    assert!(reply.ok, "{:?}", reply.error);
    rig.ack("c-off2", 30_000);
    assert!(rig.run_until(5_000, |core| {
        core.observed.node_power(2) == PowerState::Off
    }));

    // A watcher that would catch node 1 losing its rail for a moment.
    let reply = rig.verb("c-onall", CommandKind::OnAll, Target::All);
    assert!(reply.ok, "{:?}", reply.error);
    let mut node1_dropped = false;
    let ack = {
        let want = String::from("c-onall");
        let until = rig.now_ms + 120_000;
        while rig.now_ms < until && rig.core.finished_reply(&want).is_none() {
            rig.run(TICK_MS);
            if rig.core.observed.node_power(1) == PowerState::Off {
                node1_dropped = true;
            }
        }
        rig.core.finished_reply(&want).expect("an ack for on_all")
    };
    assert!(ack.ok, "{:?}", ack.error);
    assert!(
        !node1_dropped,
        "on_all cut the rail of a node that was running"
    );
    assert_eq!(rig.host(1), HostState::On, "and it kept running");
    // The ones that were off came up.
    assert!(rig.run_until(60_000, |core| {
        (2..=4).all(|node| core.observed.node_host(node) == HostState::On)
    }));

    // With everything running there is nothing left to do.
    let idle = rig.verb("c-idle", CommandKind::OnAll, Target::All);
    assert!(idle.ok, "{:?}", idle.error);
    assert_eq!(idle.result.as_deref(), Some("done"));
    assert!(idle.job.is_none(), "no plane job was submitted");
}

#[test]
fn a_modbus_write_takes_the_same_path() {
    let mut rig = Rig::new();
    // Node 3's command word: power_off, the rail only.
    let (echo, result) = rig.modbus_write(NODE_CMD_BASE + 2, cmd_word::POWER_OFF);
    assert_eq!(echo[0], 6, "the write is echoed");
    assert_eq!(
        result,
        Some(ResultCode::Accepted),
        "a queued job reads back as accepted"
    );
    assert!(rig.run_until(10_000, |core| {
        core.observed.node_power(3) == PowerState::Off
    }));

    // And a plane command word reboots the plane.
    let boots = rig.core.observed.planes[0].boot_count;
    let (_, result) = rig.modbus_write(PLANE_CMD_BASE, plane_cmd_word::REBOOT);
    assert_eq!(result, Some(ResultCode::Accepted));
    assert!(
        rig.run_until(30_000, |core| {
            core.observed.plane(0).is_some_and(|p| p.boot_count > boots)
        }),
        "the plane never rebooted"
    );
}

#[test]
fn an_absent_node_fails_at_once() {
    let mut rig = Rig::new();
    // Nodes 5-8 are on units that are not on the chain.
    let reply = rig.verb("c-absent", CommandKind::On, Target::Node(5));
    assert!(!reply.ok, "an absent node is refused");
    assert!(reply.job.is_none(), "nothing was submitted");
    assert_eq!(rig.core.plane.jobs_running(), 0);

    // Node 9 is on plane 1, which is not there at all.
    let reply = rig.verb("c-noplane", CommandKind::On, Target::Node(9));
    assert!(!reply.ok);
    assert!(reply.error.unwrap().contains("plane 1"));

    // A unit that is not on the chain, and a plane that is not there.
    let reply = rig.verb(
        "c-nounit",
        CommandKind::UnitReset,
        Target::Unit { plane: 0, unit: 3 },
    );
    assert!(!reply.ok);
    let reply = rig.verb("c-noplane2", CommandKind::PlaneReboot, Target::Plane(4));
    assert!(!reply.ok);

    // The plane is still perfectly usable.
    let reply = rig.verb("c-ok", CommandKind::PowerOff, Target::Node(1));
    assert!(reply.ok, "{:?}", reply.error);
}

/// Bug: `all` was refused by every verb but `on_all`, so the shipped
/// safety rules switched nothing off. A node verb on `all` is now one
/// plane job per present node, acked once.
#[test]
fn a_node_verb_on_all_reaches_every_present_node() {
    let mut rig = Rig::new();
    assert_eq!(rig.core.observed.present_nodes(), vec![1, 2, 3, 4]);

    let reply = rig.verb("c-all", CommandKind::PowerOff, Target::All);
    assert!(reply.ok, "{:?}", reply.error);
    assert_eq!(reply.result.as_deref(), Some("accepted"));
    let ack = rig.ack("c-all", 30_000);
    assert!(ack.ok, "{:?}", ack.error);
    assert_eq!(ack.job, reply.job, "one ack, on the group's first job");
    assert!(
        rig.run_until(10_000, |core| {
            (1..=4).all(|node| core.observed.node_power(node) == PowerState::Off)
        }),
        "not every node went off: {:?}",
        (1..=4).map(|n| rig.power(n)).collect::<Vec<_>>()
    );
    // Every node carries its own job result.
    for node in 1..=4 {
        let last = rig
            .core
            .observed
            .node(node)
            .and_then(|n| n.last_job.clone())
            .expect("a last job");
        assert_eq!(last.id, "c-all");
        assert_eq!(last.verb, "power_off");
        assert_eq!(last.result.as_deref(), Some("done"), "node {node}");
    }
    // And exactly one ack went out for the command.
    let acks = rig.core.acks.iter().filter(|r| r.id == "c-all").count();
    assert_eq!(acks, 1, "a group is acked once");
}

#[test]
fn force_off_ends_with_the_node_off_even_when_the_host_ignores_it() {
    let mut scenario = scenario();
    scenario.planes[0].slots[0].host_on = true;
    scenario.planes[0].slots[0].respond = PlaneRespond::IgnoresHold;
    let mut rig = Rig::with_scenario(scenario, Config::default());
    assert!(rig.run_until(10_000, |core| {
        core.observed.node_host(1) == HostState::On
    }));

    let reply = rig.verb("c-force", CommandKind::ForceOff, Target::Node(1));
    assert!(reply.ok, "{:?}", reply.error);
    let ack = rig.ack("c-force", 120_000);
    assert!(ack.ok, "{:?}", ack.error);
    // The host survived the hold, so the plane cut the rail: the node
    // is off either way (ADR 0004 decision 4).
    assert_eq!(rig.power(1), PowerState::Off);
}

#[test]
fn the_node_settings_reach_the_plane() {
    let mut config = Config::default();
    config.nodes.nodes.push(NodeSettings {
        node: 1,
        name: String::from("head"),
        // A soft off that gives up almost at once, and no escalation.
        t_soft_off_s: 1,
        escalation: Escalation::None,
        ..NodeSettings::for_node(1)
    });
    let mut scenario = scenario();
    scenario.planes[0].slots[0].host_on = true;
    scenario.planes[0].slots[0].respond = PlaneRespond::IgnoresShortPress;
    let mut rig = Rig::with_scenario(scenario, config);
    assert!(rig.run_until(10_000, |core| {
        core.observed.node_host(1) == HostState::On
    }));

    let reply = rig.verb("c-off", CommandKind::Off, Target::Node(1));
    assert!(reply.ok, "{:?}", reply.error);
    let ack = rig.ack("c-off", 60_000);
    // Escalation `none` and a host that ignores the press: the verb
    // fails rather than cutting the rail, and the node stays up.
    assert!(!ack.ok, "the verb should not claim success");
    assert_eq!(rig.power(1), PowerState::On, "the rail was left alone");
    assert_eq!(rig.core.observed.node(1).unwrap().host, HostState::On);
}

/// Bug: the plane job gave up after a fixed 600 s (and the MQTT ack
/// after 300 s), while a node's `t_soft_off_s` may be 3600. A soft off
/// on a node that waits 15 minutes for its host now runs to its end and
/// is acked with the job id it was accepted with.
#[test]
fn a_long_soft_off_is_not_timed_out() {
    let mut config = Config::default();
    config.nodes.nodes.push(NodeSettings {
        node: 1,
        // Well past the 600 s the client used to allow a job.
        t_soft_off_s: 900,
        ..NodeSettings::for_node(1)
    });
    let mut scenario = scenario();
    scenario.planes[0].slots[0].host_on = true;
    // The host ignores the short press, so the sequence waits the whole
    // window and then escalates to the hold.
    scenario.planes[0].slots[0].respond = PlaneRespond::IgnoresShortPress;
    let mut rig = Rig::with_scenario(scenario, config);
    assert!(rig.run_until(10_000, |core| {
        core.observed.node_host(1) == HostState::On
    }));

    let reply = rig.verb("c-slow", CommandKind::Off, Target::Node(1));
    assert!(reply.ok, "{:?}", reply.error);
    let job = reply.job.expect("a job id");
    // 900 s of waiting plus the escalation, on the fake clock.
    let ack = rig.ack("c-slow", 1_000_000);
    assert!(ack.ok, "the job was given up on: {:?}", ack.error);
    assert_eq!(ack.result.as_deref(), Some("done"));
    assert_eq!(ack.job, Some(job), "the completion ack carries its job");
    assert_eq!(rig.host(1), HostState::Off, "the host was shut down");
}

/// A plane that comes up having lost its intended state (a cold frame)
/// is a plane boot, so the policies run.
#[test]
fn a_boot_policy_is_applied_on_a_plane_boot() {
    let mut config = Config::default();
    config.nodes.nodes.push(NodeSettings {
        node: 2,
        boot_policy: BootPolicy::On,
        ..NodeSettings::for_node(2)
    });
    let mut rig = Rig::with_scenario(scenario(), config);
    assert!(
        rig.run_until(60_000, |core| {
            core.observed.node_host(2) == HostState::On
        }),
        "the boot policy never ran"
    );
    // And nothing happened to the nodes that said `leave`.
    assert_eq!(rig.host(1), HostState::Off);
    assert_eq!(rig.power(1), PowerState::On);
}

/// Bug: the boot policies ran once per controller start, so a firmware
/// update or an API `reboot` pressed PWR on every dark node and shut
/// down every node whose policy was `off`. ADR 0004 ("the controller
/// reboots: does any node change state? No") wants them on a plane boot
/// only.
#[test]
fn a_controller_restart_runs_no_boot_policy() {
    let mut config = Config::default();
    // Node 2 wants to come up, node 3 wants to be shut down.
    config.nodes.nodes.push(NodeSettings {
        node: 2,
        boot_policy: BootPolicy::On,
        ..NodeSettings::for_node(2)
    });
    config.nodes.nodes.push(NodeSettings {
        node: 3,
        boot_policy: BootPolicy::Off,
        ..NodeSettings::for_node(3)
    });
    let mut scenario = scenario();
    // Node 3's host is already running when the frame starts.
    scenario.planes[0].slots[2].host_on = true;
    let mut rig = Rig::with_scenario(scenario.clone(), config.clone());

    // The frame came up cold, so that *is* a plane boot: node 2 is
    // started and node 3 is shut down.
    assert!(
        rig.run_until(90_000, |core| {
            core.observed.node_host(2) == HostState::On
                && core.observed.node_host(3) == HostState::Off
        }),
        "the plane boot never ran the policies: {:?} {:?}",
        rig.host(2),
        rig.host(3)
    );
    // Bring node 3 back by hand, the way an operator would.
    let reply = rig.verb("c-on3", CommandKind::On, Target::Node(3));
    assert!(reply.ok, "{:?}", reply.error);
    rig.ack("c-on3", 60_000);
    assert_eq!(rig.host(3), HostState::On);
    // Let the plane get past "it just booted".
    rig.run(90_000);

    // Now restart the controller alone: the planes keep running.
    let planes = std::mem::replace(&mut rig.core.planes, granite_sim::FakePlaneLink::new());
    let mut core = SimCore::new(&scenario, &config);
    core.planes = planes;
    rig.core = core;
    rig.run(30_000);

    assert_eq!(
        rig.host(3),
        HostState::On,
        "a controller restart shut down a node whose policy says off"
    );
    assert_eq!(rig.host(2), HostState::On, "and node 2 kept running");
    // No boot command was even submitted.
    assert!(
        rig.core.jobs.last(3).is_none(),
        "the restart submitted a job for node 3"
    );
}

#[test]
fn an_intended_off_survives_a_plane_power_loss() {
    let mut rig = Rig::new();

    // Command node 2's rail off and wait for the job.
    let reply = rig.verb("c-off2", CommandKind::PowerOff, Target::Node(2));
    assert!(reply.ok, "{:?}", reply.error);
    let ack = rig.ack("c-off2", 30_000);
    assert!(ack.ok, "{:?}", ack.error);
    // The ack is the plane's result; the snapshot follows at the next
    // SUMMARY read.
    assert!(rig.run_until(5_000, |core| {
        core.observed.node_power(2) == PowerState::Off
    }));
    assert_eq!(
        rig.core.plane.intended_off(),
        &[2],
        "the controller remembers what it put off"
    );

    // The plane loses its own power: its backup registers go with it,
    // every slot comes back on and STATUS says the intended state is
    // lost (ADR 0004, "What an Expert Would Ask").
    rig.core.planes.inject(
        0x30,
        PlaneFault {
            kind: FaultKind::PowerLoss,
            ..PlaneFault::default()
        },
    );
    // First the slot comes back on with the plane, which is the whole
    // problem: nothing on the plane remembers it was off.
    assert!(
        rig.run_until(30_000, |core| {
            core.observed.node_power(2) == PowerState::On
        }),
        "the slot never came back on after the power loss"
    );
    // Then the controller notices and puts it back.
    assert!(
        rig.run_until(60_000, |core| core.events.iter().any(|e| matches!(
            e,
            EventKind::Plane { what, .. } if what == "intended_off_reapplied"
        ))),
        "the controller never re-applied the OFF it remembered"
    );
    assert!(
        rig.run_until(30_000, |core| {
            core.observed.node_power(2) == PowerState::Off
        }),
        "node 2 never went back off: {:?}",
        rig.core.observed.node(2).map(|n| n.power)
    );
    assert_eq!(rig.power(1), PowerState::On, "the others were left alone");
}

#[test]
fn a_controller_restart_moves_no_node() {
    let mut rig = Rig::new();
    let reply = rig.verb("c-on", CommandKind::On, Target::Node(1));
    assert!(reply.ok);
    rig.ack("c-on", 60_000);
    assert_eq!(rig.host(1), HostState::On);

    // Restart the controller: a fresh client and a fresh job log, the
    // same planes. The planes hold their state and their sequences.
    let intended = rig.core.plane.intended_off().to_vec();
    let planes = std::mem::replace(&mut rig.core.planes, granite_sim::FakePlaneLink::new());
    let config = rig.core.config.clone();
    let mut core = SimCore::new(&scenario(), &config);
    core.planes = planes;
    core.plane.load_intended_off(&intended);
    rig.core = core;
    rig.run(5_000);

    assert_eq!(rig.power(1), PowerState::On);
    assert_eq!(rig.host(1), HostState::On, "the host kept running");
    assert_eq!(rig.core.observed.present_nodes(), vec![1, 2, 3, 4]);
}

// --- nodes without a front panel --------------------------------------

/// The configuration of a frame where node 2 has no front-panel cable.
fn no_front_panel_config(node: NodeId, boot_policy: BootPolicy) -> Config {
    let mut config = Config::default();
    config.nodes.nodes.push(NodeSettings {
        front_panel: false,
        boot_policy,
        ..NodeSettings::for_node(node)
    });
    config
}

/// How long the last press the fake host behind a slot felt was \[ms\].
fn last_press_ms(rig: &mut Rig, slot: u8) -> u32 {
    rig.core
        .planes
        .plane_mut(0x30)
        .expect("the plane at 0x30")
        .slot_mut(slot)
        .expect("a slot")
        .node
        .last_press_ms()
}

/// ADR 0004, "Nodes without a front panel", end to end: the verbs are
/// the rail, `off` is reported as a hard cut, `reset` is refused, the
/// host axis stays `unknown`, and the flag is published.
#[test]
fn a_node_without_a_front_panel_is_switched_by_its_rail() {
    use granite_core::modbus_map::{NODE_FRONT_PANEL_BIT_BASE, RegKind};
    use granite_core::msg::State;
    use granite_core::plane::HARD_CUT_TEXT;

    let mut rig = Rig::with_scenario(scenario(), no_front_panel_config(2, BootPolicy::Leave));

    // The host axis says nothing about node 2, and the flag is in the
    // snapshot, the state payload and the Modbus map.
    assert_eq!(rig.power(2), PowerState::On);
    assert_eq!(rig.host(2), HostState::Unknown);
    assert_eq!(rig.host(1), HostState::Off, "node 1 is unchanged");
    assert!(!rig.core.observed.node(2).expect("node 2").front_panel);
    let state = State::from_observed(&rig.core.observed, &rig.core.config.nodes, rig.now_ms);
    let node2 = state
        .nodes
        .iter()
        .find(|n| n.node == 2)
        .expect("node 2 in the state payload");
    assert!(!node2.front_panel);
    assert_eq!(node2.host, "unknown");
    assert!(
        state
            .nodes
            .iter()
            .find(|n| n.node == 1)
            .expect("node 1")
            .front_panel
    );
    let snapshot = rig.core.observed.clone();
    let image = Image::new(&snapshot, FwVersion::default(), 0, ResultCode::Idle);
    assert_eq!(
        image.bit(RegKind::DiscreteInput, NODE_FRONT_PANEL_BIT_BASE + 1),
        Some(false)
    );
    assert_eq!(
        image.bit(RegKind::DiscreteInput, NODE_FRONT_PANEL_BIT_BASE),
        Some(true)
    );

    // off: the rail goes down and the ack says what it was.
    let reply = rig.verb("c-off", CommandKind::Off, Target::Node(2));
    assert!(reply.ok, "{:?}", reply.error);
    let ack = rig.ack("c-off", 30_000);
    assert!(ack.ok, "{:?}", ack.error);
    assert!(ack.hard_cut, "the ack flags the hard cut");
    assert_eq!(ack.result.as_deref(), Some(HARD_CUT_TEXT));
    assert!(rig.run_until(10_000, |core| {
        core.observed.node_power(2) == PowerState::Off
    }));
    assert_eq!(
        rig.core.plane.intended_off(),
        &[2],
        "a hard cut is a rail-off intent"
    );
    // The `action_done` event carries the same flag, which is what the
    // surfaces with no room for the text have.
    assert!(
        rig.core.events.iter().any(|e| matches!(
            e,
            EventKind::ActionDone { id, hard_cut: true, .. } if id == "c-off"
        )),
        "no action_done with hard_cut for c-off"
    );

    // reset and press reach nothing, so they are refused at dispatch.
    for (id, action) in [
        ("c-rst", CommandKind::Reset),
        ("c-press", CommandKind::Press),
    ] {
        let reply = rig.verb(id, action, Target::Node(2));
        assert!(!reply.ok, "{} was accepted", action.as_str());
        let error = reply.error.unwrap_or_default();
        assert!(error.contains("no front panel"), "{error}");
    }

    // on: the rail comes back, and the ack is a plain `done`.
    let reply = rig.verb("c-on", CommandKind::On, Target::Node(2));
    assert!(reply.ok, "{:?}", reply.error);
    let ack = rig.ack("c-on", 30_000);
    assert!(ack.ok, "{:?}", ack.error);
    assert!(!ack.hard_cut);
    assert_eq!(ack.result.as_deref(), Some("done"));
    assert!(rig.run_until(10_000, |core| {
        core.observed.node_power(2) == PowerState::On
    }));
    assert!(rig.core.plane.intended_off().is_empty());

    // cycle: the rail goes down and comes back as one job.
    let reply = rig.verb("c-cycle", CommandKind::Cycle, Target::Node(2));
    assert!(reply.ok, "{:?}", reply.error);
    assert!(
        rig.run_until(10_000, |core| {
            core.observed.node_power(2) == PowerState::Off
        }),
        "the cycle never took the rail down"
    );
    let ack = rig.ack("c-cycle", 30_000);
    assert!(ack.ok, "{:?}", ack.error);
    assert!(ack.hard_cut, "a cycle is a cut too");
    assert!(rig.run_until(10_000, |core| {
        core.observed.node_power(2) == PowerState::On
    }));

    // Nothing ever pressed node 2's PWR, and node 1 still answers its
    // own button the way it always did.
    assert_eq!(last_press_ms(&mut rig, 1), 0, "node 2 was pressed");
    let reply = rig.verb("c-on1", CommandKind::On, Target::Node(1));
    assert!(reply.ok, "{:?}", reply.error);
    rig.ack("c-on1", 60_000);
    assert_eq!(rig.host(1), HostState::On);
    assert!(last_press_ms(&mut rig, 0) > 0, "node 1 took a press");
}

/// `boot_policy = on` with no front panel means "this node powers up at
/// every plane boot": the rail, not a button.
#[test]
fn a_boot_policy_on_a_node_without_a_front_panel_is_its_rail() {
    let mut rig = Rig::with_scenario(scenario(), no_front_panel_config(2, BootPolicy::On));

    // Take the rail down by hand first.
    let reply = rig.verb("c-off2", CommandKind::PowerOff, Target::Node(2));
    assert!(reply.ok, "{:?}", reply.error);
    rig.ack("c-off2", 30_000);
    assert!(rig.run_until(10_000, |core| {
        core.observed.node_power(2) == PowerState::Off
    }));

    // The plane reboots on command: the boot count moves, which is what
    // the policies run on, and an OFF slot stays off across it (ADR
    // 0002 decision 3) - so the policy is what brings node 2 back.
    let reply = rig.verb("c-rb", CommandKind::PlaneReboot, Target::Plane(0));
    assert!(reply.ok, "{:?}", reply.error);
    assert!(
        rig.run_until(120_000, |core| {
            core.observed.node_power(2) == PowerState::On
        }),
        "the boot policy never brought the rail up"
    );
    assert_eq!(last_press_ms(&mut rig, 1), 0, "and it pressed nothing");
}

// --- the rail force_off ends with ---------------------------------------
//
// ADR 0004, "`force_off` always ends with the rail down".

/// A frame where every node's front-panel cable has fallen out: no
/// press is felt, no LED ever lights, and every slot keeps drawing a
/// running host's watts. The bench case of 2026-11-25.
fn unplug_every_front_panel(rig: &mut Rig) {
    let plane = rig
        .core
        .planes
        .plane_mut(0x30)
        .expect("the plane at 0x30");
    for slot in 0..4u8 {
        plane
            .slot_mut(slot)
            .expect("a slot")
            .node
            .min_press_ms = u32::MAX;
        plane.set_load(slot, 1_000);
        plane.set_standby(slot, 1_000);
    }
}

/// The documented action of the leak and over-temperature rules:
/// `force_off` on `all`, through the dispatcher with `force` set. Every
/// rail ends down even though no host answers its button.
#[test]
fn a_leak_rule_takes_every_rail_down() {
    use granite_core::rules::{Op, Rule, RuleAction, Source};

    let mut config = Config::default();
    config.rules.rules.push(Rule {
        id: 1,
        enabled: true,
        name: "leak float closed".into(),
        source: Source::DryIn { n: 1 },
        op: Op::Eq,
        threshold: 1,
        action: RuleAction::Act {
            kind: CommandKind::ForceOff,
        },
        target: Target::All,
        ..Rule::default()
    });
    let mut rig = Rig::with_scenario(scenario(), config);
    unplug_every_front_panel(&mut rig);

    // Bring the hosts up so there is something to take down.
    let reply = rig.verb("c-on", CommandKind::OnAll, Target::All);
    assert!(reply.ok, "{:?}", reply.error);
    rig.run(20_000);
    for node in 1..=4 {
        assert_eq!(rig.power(node), PowerState::On, "node {node} never came up");
    }

    // The float closes.
    rig.core.sensors.set_dry(1, true);
    assert!(
        rig.run_until(60_000, |core| (1..=4)
            .all(|node| core.observed.node_power(node) == PowerState::Off)),
        "rails: {:?}",
        (1..=4).map(|n| rig.power(n)).collect::<Vec<_>>()
    );

    // And the controller keeps them off over a plane that loses its
    // intended state.
    let mut intended = rig.core.plane.intended_off().to_vec();
    intended.sort_unstable();
    assert_eq!(intended, vec![1, 2, 3, 4]);
    assert!(
        rig.core.events.iter().any(|e| matches!(
            e,
            EventKind::ActionDone { rail_cut: true, .. }
        )),
        "no action_done said the controller cut a rail"
    );
}

/// The two explicit verbs over the API and over Modbus (ADR 0004, "Two
/// explicit verbs").
#[test]
fn the_explicit_force_verbs_reach_the_plane() {
    let mut rig = Rig::with_scenario(scenario(), no_front_panel_config(2, BootPolicy::Leave));
    use granite_core::plane::HARD_CUT_TEXT;

    // Node 1 has a panel: the hold alone leaves its rail up.
    let reply = rig.verb("c-on1", CommandKind::On, Target::Node(1));
    assert!(reply.ok, "{:?}", reply.error);
    rig.ack("c-on1", 60_000);
    assert_eq!(rig.host(1), HostState::On);

    let reply = rig.verb("c-btn", CommandKind::ForceButtonOff, Target::Node(1));
    assert!(reply.ok, "{:?}", reply.error);
    let ack = rig.ack("c-btn", 30_000);
    assert!(ack.ok, "{:?}", ack.error);
    assert!(!ack.rail_cut, "force_button_off may not touch the rail");
    assert!(rig.run_until(10_000, |core| core.observed.node_host(1)
        == HostState::Off));
    assert_eq!(rig.power(1), PowerState::On, "the rail stayed up");
    assert!(rig.core.plane.intended_off().is_empty());

    // Node 2 has no panel, so the hold reaches nothing and is refused.
    let reply = rig.verb("c-btn2", CommandKind::ForceButtonOff, Target::Node(2));
    assert!(!reply.ok, "force_button_off was accepted on node 2");
    let error = reply.error.unwrap_or_default();
    assert!(error.contains("no front panel"), "{error}");

    // The rail verb takes node 1 down with no shutdown at all, and says
    // so the way the other cuts do.
    let reply = rig.verb("c-pwr", CommandKind::ForcePowerOff, Target::Node(1));
    assert!(reply.ok, "{:?}", reply.error);
    let ack = rig.ack("c-pwr", 30_000);
    assert!(ack.ok, "{:?}", ack.error);
    assert!(ack.hard_cut, "no shutdown was even tried");
    assert_eq!(ack.result.as_deref(), Some(HARD_CUT_TEXT));
    assert!(rig.run_until(10_000, |core| {
        core.observed.node_power(1) == PowerState::Off
    }));
    assert_eq!(rig.core.plane.intended_off(), &[1]);

    // And both have a Modbus command word (the map version did not
    // move: they are the next free values).
    let (_, result) = rig.modbus_write(NODE_CMD_BASE + 2, cmd_word::FORCE_POWER_OFF);
    assert_eq!(result, Some(ResultCode::Accepted));
    assert!(rig.run_until(30_000, |core| {
        core.observed.node_power(3) == PowerState::Off
    }));
    let (_, result) = rig.modbus_write(NODE_CMD_BASE + 1, cmd_word::FORCE_BUTTON_OFF);
    assert_eq!(
        result,
        Some(ResultCode::Refused),
        "node 2 has no front panel"
    );
}

/// `plane_log` answers from a read taken for the command, so a line the
/// plane wrote after the client's last read is in the *first* reply
/// (ADR 0004, "As built"). Before that it answered from the cache, and
/// the lines an operator asked for arrived one command late.
#[test]
fn plane_log_answers_from_a_fresh_read_and_not_from_the_cache() {
    let mut rig = Rig::new();
    // The plane's boot event armed a read, so the client holds the ring
    // as it was; the rate limit then keeps it from reading again for
    // `log::READ_PERIOD_MS`.
    rig.run(10_000);
    let before = rig.verb("c-log1", CommandKind::PlaneLog, Target::Plane(0));
    assert!(before.ok, "{:?}", before.error);
    let data = before.data.expect("the lines");
    assert_eq!(data["stale"], false, "the plane answered the read");
    let boot = data["lines"]
        .as_array()
        .expect("an array")
        .iter()
        .any(|l| l["text"] == "boot 1");
    assert!(boot, "the ring holds the plane's boot line: {data}");

    // A new line on the plane, which the client has not read: the
    // latch's own line, written by the plane's next poll cycle.
    rig.core
        .planes
        .plane_mut(0x30)
        .expect("the plane at 0x30")
        .latch_ov(1);
    rig.run(200);

    let reply = rig.verb("c-log2", CommandKind::PlaneLog, Target::Plane(0));
    assert!(reply.ok, "{:?}", reply.error);
    let data = reply.data.expect("the lines");
    assert_eq!(data["stale"], false);
    let lines: Vec<String> = data["lines"]
        .as_array()
        .expect("an array")
        .iter()
        .map(|l| l["text"].as_str().unwrap_or_default().to_string())
        .collect();
    assert!(
        lines.iter().any(|t| t == "ov latched 1"),
        "the line the plane wrote after the last read is in the first reply: {lines:?}"
    );

    // A plane that does not answer the read: the cache, marked stale.
    rig.core
        .planes
        .plane_mut(0x30)
        .expect("the plane at 0x30")
        .drop_transactions(8);
    let reply = rig.verb("c-log3", CommandKind::PlaneLog, Target::Plane(0));
    assert!(reply.ok, "{:?}", reply.error);
    let data = reply.data.expect("the lines");
    assert_eq!(data["stale"], true, "no read, so the cache: {data}");
    assert!(
        data["lines"]
            .as_array()
            .expect("an array")
            .iter()
            .any(|l| l["text"] == "ov latched 1"),
        "and the cache is what the last read brought: {data}"
    );
}
