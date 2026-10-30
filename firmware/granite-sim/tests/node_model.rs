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

#[test]
fn a_boot_policy_is_applied_once_per_controller_start() {
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
