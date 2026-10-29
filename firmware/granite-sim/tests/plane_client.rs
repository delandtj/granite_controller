//! The plane client of controller ADR 0004 decision 5, against the
//! simulated planes.
//!
//! [`granite_core::plane::PlaneClient`] is driven exactly as the
//! firmware drives it: `poll(now_ms, &mut link)` every
//! [`POLL_MS`] with a fake clock, while the planes run on their own
//! 20 Hz cycle behind [`FakePlaneLink`]. Nothing here reaches into the
//! client's state: a scenario is set up, the clock runs, and what the
//! client says through its models, its events and its finished jobs is
//! what is asserted.
//!
//! The properties, in the order ADR 0004 and ADR 0003 state them:
//!
//! - discovery finds exactly the scenario's planes, and node ids follow
//!   the straps and the slot (`8 * (addr - 0x30) + slot + 1`),
//! - the poll schedule fills the models: both state axes, the slot
//!   telemetry and the units' inputs,
//! - every verb of decision 4 ends with the right result, `force_off`
//!   on a host that ignores its button included (the rail is cut),
//! - a write whose result never comes back is resent under the same
//!   sequence number and does not run twice,
//! - a corrupted answer is retried and costs nothing but a counter,
//! - EXP_INT becomes client events and is acknowledged,
//! - a plane that reboots on its watchdog keeps its OFF slots; one that
//!   lost its power says so and gets the intended OFF put back,
//! - a unit unplugged takes its nodes absent and a replug brings them
//!   back,
//! - a config set is pushed when the plane's hash does not match,
//! - a plane firmware update runs to a confirmed image, and one the
//!   plane does not keep is reported as rolled back.

use std::path::Path;

use granite_core::hal::Switch;
use granite_core::plane::{
    BufferedImage, ClientEvent, DesiredConfig, FinishedJob, JobId, JobOutcome, Node, NodeCommand,
    PlaneClient, PlaneCommand, PowerState, UpdateFailure, UpdatePolicy,
};
use granite_proto::{
    Config as ConfigRecord, EventKind, ImageState, PowerLimit, ResultCode, SlotStatus, StatusFlags,
    UnitStatus, Version,
};
use granite_sim::Scenario;
use granite_sim::planes::{FakePlaneLink, PLANE_TICK_MS};
use granite_sim::scenario::{PlaneRespond, PlaneScenario, PlaneSlotScenario};

/// How often the firmware calls `poll` \[ms\]; the plane's own cycle is
/// the same 50 ms, so one step of the harness is one step of both.
const POLL_MS: u64 = PLANE_TICK_MS;

/// The client's sequence seed. A constant here, the firmware's RNG on a
/// board (ADR 0003: the counter must not start at a fixed value).
const SEED: u32 = 0x5EED_1234;

// --- the harness ------------------------------------------------------

/// The client, the planes behind a link, and one clock for both.
struct Harness {
    link: FakePlaneLink,
    client: PlaneClient,
    now_ms: u64,
    events: Vec<ClientEvent>,
    jobs: Vec<FinishedJob>,
}

impl Harness {
    /// A harness over `scenario`, settled: the planes have run long
    /// enough for their LEDs to be called steady, and the client has
    /// discovered them and filled its models.
    fn new(scenario: &Scenario) -> Self {
        let mut link = FakePlaneLink::from_scenario(scenario);
        // The planes come up before the controller does.
        let mut now_ms = 0;
        while now_ms < 2_000 {
            now_ms += POLL_MS;
            link.tick(now_ms);
        }
        let mut harness = Harness {
            link,
            client: PlaneClient::new(SEED),
            now_ms,
            events: Vec::new(),
            jobs: Vec::new(),
        };
        harness.run(3_000);
        harness
    }

    /// Run both sides for `ms`, polling the client every [`POLL_MS`].
    fn run(&mut self, ms: u64) {
        let until = self.now_ms + ms;
        while self.now_ms < until {
            self.step();
        }
    }

    /// One step of the clock: the planes first, then the client.
    fn step(&mut self) {
        self.now_ms += POLL_MS;
        self.link.tick(self.now_ms);
        self.client.poll(self.now_ms, &mut self.link);
        self.events.append(&mut self.client.drain_events());
        self.jobs.append(&mut self.client.drain_jobs());
    }

    /// Run until `done` holds, at most `ms`. True when it did.
    fn run_until(&mut self, ms: u64, done: impl Fn(&Harness) -> bool) -> bool {
        let until = self.now_ms + ms;
        while self.now_ms < until {
            if done(self) {
                return true;
            }
            self.step();
        }
        done(self)
    }

    /// Submit a command and wait for it to finish, at most `ms`.
    fn do_verb(
        &mut self,
        command: impl Into<granite_core::plane::Command>,
        ms: u64,
    ) -> FinishedJob {
        let id = self.client.submit(self.now_ms, command);
        assert!(
            self.run_until(ms, |h| h.finished(id).is_some()),
            "the job did not finish inside {ms} ms"
        );
        self.finished(id).expect("just checked").clone()
    }

    /// Submit a command and expect it to end well.
    fn verb_ok(&mut self, command: impl Into<granite_core::plane::Command>, ms: u64) {
        let verb = {
            let c: granite_core::plane::Command = command.into();
            let verb = c.verb();
            let job = self.do_verb(c, ms);
            assert_eq!(
                job.outcome,
                JobOutcome::Done,
                "{verb} ended {:?} ({:?})",
                job.outcome,
                job.code
            );
            assert_eq!(job.code, Some(ResultCode::Ok));
            verb
        };
        let _ = verb;
    }

    /// A finished job by id.
    fn finished(&self, id: JobId) -> Option<&FinishedJob> {
        self.jobs.iter().find(|j| j.id == id)
    }

    /// One node of the client's model.
    fn node(&self, id: u8) -> &Node {
        self.client
            .node(id)
            .unwrap_or_else(|| panic!("node {id} is on a plane the client knows"))
    }

    /// The slot status word the client holds for a node.
    fn status(&self, id: u8) -> SlotStatus {
        self.node(id).status
    }

    /// True while a plane holds EXP_INT (the line is the wired OR of
    /// every plane's).
    fn int_asserted(&self) -> bool {
        self.link
            .planes()
            .any(granite_sim::planes::FakePlane::event_pending)
    }

    /// True when any event so far matches.
    fn saw(&self, matches: impl Fn(&ClientEvent) -> bool) -> bool {
        self.events.iter().any(matches)
    }

    /// True when the client has seen a plane event of this kind for a
    /// node.
    fn saw_node_kind(&self, node: u8, kind: EventKind) -> bool {
        self.saw(
            |e| matches!(e, ClientEvent::Node { node: n, kind: k, .. } if *n == node && *k == kind),
        )
    }

    /// Forget the events and jobs collected so far.
    fn clear(&mut self) {
        self.events.clear();
        self.jobs.clear();
    }

    /// The plane behind the link, to inject a fault.
    fn fake(&mut self, address: u8) -> &mut granite_sim::planes::FakePlane {
        self.link
            .plane_mut(address)
            .unwrap_or_else(|| panic!("the scenario has a plane at {address:#04x}"))
    }
}

// --- scenarios --------------------------------------------------------

/// A plane section with the blink window shortened, so a wait on a host
/// LED is about a second rather than the five a shipped plane takes.
fn plane(address: u8, units: Vec<u8>, slots: Vec<PlaneSlotScenario>) -> PlaneScenario {
    PlaneScenario {
        address,
        units,
        blink_window_ms: Some([100, 500]),
        slots,
        faults: Vec::new(),
    }
}

/// One slot with a module and a host.
fn slot(slot: u8, host_on: bool, load_ma: u32) -> PlaneSlotScenario {
    PlaneSlotScenario {
        slot,
        host_on,
        load_ma,
        ..Default::default()
    }
}

/// The scenario the behaviour tests run on: a plane at 0x30 with two
/// units and four running hosts, and a plane at 0x31 with one unit whose
/// B half is dark. Node ids 1-4 and 9-10.
fn scenario() -> Scenario {
    let mut scenario = Scenario {
        planes: vec![
            plane(
                0x30,
                vec![0, 1],
                (0..4).map(|s| slot(s, true, 1_000)).collect(),
            ),
            plane(
                0x31,
                vec![0],
                vec![slot(0, true, 1_300), slot(1, false, 700)],
            ),
        ],
        ..Default::default()
    };
    scenario.normalise();
    scenario
}

/// The sample scenario shipped next to the other ones.
fn sample() -> Scenario {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("scenarios/two-planes.toml");
    Scenario::load(&path).expect("the sample scenario parses")
}

/// An image of `len` bytes that is not all 0xFF, with its CRC-32.
fn image(len: usize) -> (Vec<u8>, u32) {
    let bytes: Vec<u8> = (0..len).map(|i| (i * 31 + 7) as u8).collect();
    let crc = plane_logic::crc32::crc32(&bytes);
    (bytes, crc)
}

// --- discovery and the models -----------------------------------------

#[test]
fn discovery_finds_the_scenario_planes_and_numbers_their_nodes() {
    let harness = Harness::new(&scenario());

    assert_eq!(harness.client.addresses(), vec![0x30, 0x31]);
    assert!(harness.saw(|e| matches!(
        e,
        ClientEvent::PlaneFound {
            address: 0x30,
            boot_count: 1,
            ..
        }
    )));
    assert!(harness.saw(|e| matches!(e, ClientEvent::PlaneFound { address: 0x31, .. })));

    // An address with no plane stays absent and is not an error the
    // client reports.
    assert!(harness.client.plane(0x33).is_none());
    assert!(harness.client.errors(0x33).link > 0, "it NAKed");
    assert!(!harness.client.collision_suspected(0x33));

    // Node ids follow the straps and the slot (ADR 0004 decision 2).
    assert_eq!(harness.node(1).address, 0x30);
    assert_eq!(harness.node(1).slot, 0);
    assert_eq!(harness.node(8).slot, 7);
    assert_eq!(harness.node(9).address, 0x31);
    assert_eq!(harness.node(10).slot, 1);
    assert_eq!(harness.node(11).power, PowerState::Absent, "0x31 slot 2");
    assert!(harness.client.node(17).is_none(), "0x32 has no plane");

    // Present nodes are the populated slots of the units on the chain:
    // four on 0x30, two on 0x31.
    let present: Vec<u8> = harness.client.nodes().map(|n| n.id).collect();
    assert_eq!(present, vec![1, 2, 3, 4, 9, 10]);

    let plane = harness.client.plane(0x30).expect("0x30");
    assert_eq!(
        plane.plane_type,
        Some(granite_proto::PlaneType::Pp48Chained)
    );
    assert_eq!(plane.slot_capacity, 8);
    assert_eq!(plane.units_present.bits(), 0b0011);
    assert_eq!(plane.firmware, Version::new(0, 1, 0));
    assert!(plane.core_volts > 4_000, "the 48 V input");
    assert_eq!(plane.present_units().count(), 2);
}

#[test]
fn the_poll_schedule_fills_both_axes_the_telemetry_and_the_units() {
    let mut harness = Harness::new(&scenario());

    // Both axes, from SUMMARY (ADR 0004 decision 3).
    for id in [1, 2, 3, 4, 9] {
        let node = harness.node(id);
        assert_eq!(node.power, PowerState::On, "node {id} rail");
        assert_eq!(
            node.host,
            granite_core::plane::HostState::On,
            "node {id} host"
        );
        assert!(node.telemetry.amps > 0, "node {id} draws its load");
    }
    // Plane 0x31's B half has a module and a dark host: the rail is up,
    // the host is down.
    assert_eq!(harness.node(10).power, PowerState::On);
    assert_eq!(
        harness.node(10).host,
        granite_core::plane::HostState::Off,
        "the host is dark, not unknown: its rail is up"
    );
    // A slot with no unit behind it is absent, not unknown.
    assert_eq!(harness.node(5).power, PowerState::Absent);

    // The SLOT and UNIT sweeps bring what SUMMARY does not carry.
    let node = harness.node(1);
    assert!(node.detail_ms > 0, "the SLOT block arrived");
    assert_eq!(node.telemetry.energy_boot_count, 1);
    assert!(node.telemetry.module_id_raw > 0);
    let plane = harness.client.plane(0x30).expect("0x30");
    let unit = &plane.units[0];
    assert!(unit.updated_ms > 0, "the UNIT block arrived");
    assert!(unit.is_present() && !unit.is_latched_off());
    assert!(unit.volts > 4_000, "the unit's own input");

    // The 1 Hz schedule is what it says: ten seconds of polling is
    // some tens of transactions per plane, not hundreds.
    let (reads_before, _) = harness.client.transaction_counts();
    harness.run(10_000);
    let (reads_after, _) = harness.client.transaction_counts();
    let per_plane = (reads_after - reads_before) / 2;
    assert!(
        (20..=60).contains(&per_plane),
        "{per_plane} reads per plane in ten seconds"
    );
}

#[test]
fn a_plane_that_stops_answering_is_lost_and_found_again() {
    let mut harness = Harness::new(&scenario());
    harness.clear();

    // Two dropped transactions: a NAK is absence, not noise, so it is
    // not retried, and two in a row lose the plane.
    harness.fake(0x31).drop_transactions(2);
    assert!(
        harness.run_until(5_000, |h| h.client.plane(0x31).is_none()),
        "the plane is lost after two failed transactions"
    );
    assert!(harness.saw(|e| matches!(e, ClientEvent::PlaneLost { address: 0x31 })));
    assert!(harness.client.node(9).is_none(), "its nodes went with it");
    assert_eq!(harness.client.addresses(), vec![0x30], "0x30 is untouched");

    // The next scan finds it again, with its nodes.
    harness.clear();
    assert!(
        harness.run_until(12_000, |h| h.client.plane(0x31).is_some()),
        "the 10 s scan finds it again"
    );
    assert!(harness.saw(|e| matches!(e, ClientEvent::PlaneFound { address: 0x31, .. })));
    assert!(harness.run_until(2_000, |h| h.node(9).power == PowerState::On));
}

#[test]
fn a_corrupted_answer_is_retried_and_the_plane_stays() {
    let mut harness = Harness::new(&scenario());
    harness.clear();
    let before = harness.client.errors(0x30).bad_crc;

    // One flipped byte per answer, twice: the read's own retries
    // (ADR 0003: three tries) swallow them.
    harness.fake(0x30).corrupt_answers(1);
    harness.run(1_200);
    harness.fake(0x30).corrupt_answers(1);
    harness.run(1_200);

    let errors = harness.client.errors(0x30);
    assert!(errors.bad_crc >= before + 2, "the CRC failures are counted");
    assert_eq!(errors.failed, 0, "no read failed for good");
    assert!(!errors.collision_suspected);
    assert!(harness.client.plane(0x30).is_some());
    assert!(!harness.saw(|e| matches!(e, ClientEvent::PlaneLost { .. })));
}

#[test]
fn an_address_that_never_answers_cleanly_is_flagged_as_a_collision() {
    let mut harness = Harness::new(&scenario());
    harness.clear();

    // Two planes on one address mix their answers: every read fails its
    // CRC while the length stays valid (ADR 0003, "Two planes in one
    // frame"). A long run of corrupted answers is the same signature.
    harness.fake(0x31).corrupt_answers(1_000);
    assert!(
        harness.run_until(10_000, |h| h.client.collision_suspected(0x31)),
        "a run of CRC failures with valid lengths is flagged"
    );
    assert!(harness.saw(|e| matches!(e, ClientEvent::AddressCollisionSuspected { address: 0x31 })));
    assert!(harness.saw(|e| matches!(e, ClientEvent::PlaneLost { address: 0x31 })));
}

// --- the verbs --------------------------------------------------------

#[test]
fn every_verb_ends_with_the_right_result() {
    let mut harness = Harness::new(&scenario());

    // on, on a node whose host is already lit: the plane has nothing to
    // do and says so at once.
    harness.verb_ok(NodeCommand::on(1), 2_000);
    assert_eq!(harness.node(1).host, granite_core::plane::HostState::On);

    // press: a raw PWR press. The host takes it and goes down.
    harness.verb_ok(
        NodeCommand::Press {
            node: 1,
            switch: Switch::Pwr,
            ms: 200,
        },
        5_000,
    );
    assert!(harness.run_until(4_000, |h| h.node(1).host
        == granite_core::plane::HostState::Off));

    // on, on a node whose host is dark: the plane waits for the LED,
    // presses PWR and waits again.
    harness.verb_ok(NodeCommand::on(1), 60_000);
    assert_eq!(harness.node(1).host, granite_core::plane::HostState::On);

    // off: the short press is enough for a host that listens.
    harness.verb_ok(NodeCommand::off(1), 20_000);
    harness.run(2_000);
    assert!(
        harness.status(1).contains(SlotStatus::ENABLED),
        "a host that obeyed its button keeps its standby rail"
    );
    assert_eq!(
        harness.client.intended_off(),
        &[] as &[u8],
        "so nothing to put back after a plane power loss"
    );

    // reset: an RST press.
    harness.verb_ok(NodeCommand::Reset { node: 2 }, 5_000);

    // cycle: rail off, wait, rail on, press if dark.
    harness.verb_ok(NodeCommand::cycle(2), 60_000);
    assert_eq!(harness.node(2).power, PowerState::On);

    // power_off / power_on: the rail and nothing else.
    harness.verb_ok(NodeCommand::PowerOff { node: 3 }, 3_000);
    assert!(harness.run_until(3_000, |h| h.node(3).power == PowerState::Off));
    assert!(!harness.status(3).contains(SlotStatus::ENABLED));
    assert_eq!(
        harness.client.intended_off(),
        &[3],
        "power_off is what the controller means to keep off"
    );
    harness.verb_ok(NodeCommand::PowerOn { node: 3 }, 3_000);
    assert!(harness.client.intended_off().is_empty());
    assert!(harness.run_until(3_000, |h| h.node(3).power == PowerState::On));

    // clear_latch: deliberate, and harmless on a slot with no latch.
    harness.verb_ok(NodeCommand::ClearLatch { node: 4 }, 3_000);

    // unit_reset: the unit's own hot-swap latch.
    harness.verb_ok(
        PlaneCommand::UnitReset {
            address: 0x30,
            unit: 1,
        },
        5_000,
    );

    // on_all: one stagger per plane, then a node-on per slot.
    let job = harness.do_verb(NodeCommand::on_all(vec![9, 10]), 90_000);
    assert_eq!(job.outcome, JobOutcome::Done, "{:?}", job.code);
    assert_eq!(harness.node(9).power, PowerState::On);
    assert_eq!(harness.node(10).power, PowerState::On);

    // plane_reboot: the plane answers nothing (it resets inside the
    // same cycle), so a delivered write is the outcome.
    harness.clear();
    let before = harness.client.plane(0x31).expect("0x31").boot_count;
    harness.verb_ok(PlaneCommand::Reboot { address: 0x31 }, 3_000);
    assert!(
        harness.run_until(20_000, |h| h
            .client
            .plane(0x31)
            .is_some_and(|p| p.boot_count > before)),
        "the plane came back with a new boot count"
    );
}

#[test]
fn a_verb_for_a_node_or_unit_that_is_not_there_fails_at_once() {
    let mut harness = Harness::new(&scenario());

    // Node 5 is on a unit that is not on the chain.
    let id = harness.client.submit(harness.now_ms, NodeCommand::on(5));
    let jobs = harness.client.drain_jobs();
    assert_eq!(jobs.len(), 1, "it never reached the link");
    assert_eq!(jobs[0].id, id);
    assert_eq!(jobs[0].outcome, JobOutcome::Absent);

    // A plane that is not there.
    harness.client.submit(harness.now_ms, NodeCommand::on(20));
    assert_eq!(harness.client.drain_jobs()[0].outcome, JobOutcome::Absent);

    // A unit that is not on the chain.
    harness.client.submit(
        harness.now_ms,
        PlaneCommand::UnitReset {
            address: 0x31,
            unit: 3,
        },
    );
    assert_eq!(harness.client.drain_jobs()[0].outcome, JobOutcome::Absent);
}

#[test]
fn force_off_cuts_the_rail_of_a_host_that_ignores_its_button() {
    let mut scenario = scenario();
    // The host in slot 0 of plane 0x31 (node 9) will not shut down.
    scenario.planes[1].slots[0].respond = PlaneRespond::IgnoresHold;
    scenario.normalise();
    let mut harness = Harness::new(&scenario);
    assert_eq!(harness.node(9).host, granite_core::plane::HostState::On);

    // force_off is node off with the timeout at zero: no short press,
    // straight to the five-second hold, then the rail (ADR 0004
    // decision 4).
    harness.verb_ok(NodeCommand::force_off(9), 30_000);
    assert!(
        !harness.status(9).contains(SlotStatus::ENABLED),
        "the rail was cut"
    );
    assert!(harness.run_until(3_000, |h| h.node(9).power == PowerState::Off));

    // A node the controller cut the rail of is what it means to keep
    // off: it is in the set the firmware persists.
    assert!(
        harness.run_until(2_000, |h| h.client.intended_off() == [9]),
        "intended off: {:?}",
        harness.client.intended_off()
    );
    assert!(harness.client.take_intent_dirty());
    assert!(!harness.client.take_intent_dirty());
}

#[test]
fn a_write_whose_result_is_lost_is_resent_and_does_not_run_twice() {
    let mut harness = Harness::new(&scenario());
    // Node 10's host is dark; one press brings it up and a second one
    // would take it down again.
    assert_eq!(harness.node(10).host, granite_core::plane::HostState::Off);
    let (_, writes_before) = harness.client.transaction_counts();

    // The CTRL write itself is dropped: the plane never sees it, and the
    // client resends the identical frame under the same sequence number
    // (ADR 0003 decisions 3 and 4).
    harness.fake(0x31).drop_transactions(1);
    let job = harness.do_verb(
        NodeCommand::Press {
            node: 10,
            switch: Switch::Pwr,
            ms: 200,
        },
        20_000,
    );
    assert_eq!(job.outcome, JobOutcome::Done);
    let (_, writes_after) = harness.client.transaction_counts();
    assert!(
        writes_after - writes_before >= 2,
        "the write went out twice"
    );
    assert!(harness.run_until(4_000, |h| h.node(10).host
        == granite_core::plane::HostState::On));

    // And the press ran once: a second one would have shut the host
    // down again.
    harness.run(6_000);
    assert_eq!(
        harness.node(10).host,
        granite_core::plane::HostState::On,
        "the resend did not press a second time"
    );
}

// --- events -----------------------------------------------------------

#[test]
fn exp_int_becomes_client_events_and_is_acknowledged() {
    let mut harness = Harness::new(&scenario());
    // The boot events of both planes have been read and acknowledged
    // during the settle, so the shared line is free again.
    assert!(
        !harness.int_asserted(),
        "every event the planes raised has been acknowledged"
    );
    assert!(harness.saw(|e| matches!(
        e,
        ClientEvent::Plane {
            kind: EventKind::Boot,
            ..
        }
    )));
    harness.clear();

    // A press takes the host down, which the plane sees as an LED
    // change and a finished sequence.
    harness.verb_ok(
        NodeCommand::Press {
            node: 4,
            switch: Switch::Pwr,
            ms: 200,
        },
        10_000,
    );
    assert!(harness.run_until(6_000, |h| h.saw_node_kind(4, EventKind::LedChanged)));
    assert!(harness.saw_node_kind(4, EventKind::SequenceDone));
    assert!(harness.saw(|e| matches!(
        e,
        ClientEvent::JobFinished {
            outcome: JobOutcome::Done,
            ..
        }
    )));
    assert!(
        harness.run_until(3_000, |h| !h.int_asserted()),
        "EV_ACK released the line"
    );

    // An event about a slot also brings its SLOT block forward, so the
    // telemetry a reader sees is the one after the change.
    assert!(harness.node(4).detail_ms > harness.node(4).summary_ms - 2_000);
}

#[test]
fn a_tripped_power_limit_shows_as_a_fault_with_its_event() {
    let mut harness = Harness::new(&scenario());
    harness.clear();

    harness.fake(0x30).trip_power_limit(2, 0);
    assert!(
        harness.run_until(5_000, |h| h.node(3).power == PowerState::Fault),
        "the power axis is `fault` (ADR 0004 decision 3)"
    );
    assert!(harness.saw_node_kind(3, EventKind::PowerLimitTrip));
    assert!(
        harness
            .status(3)
            .contains(SlotStatus::POWER_LIMIT_TRIPPED | SlotStatus::OV_LATCHED)
    );
    assert_eq!(
        harness.node(3).host,
        granite_core::plane::HostState::Unknown,
        "with the rail down the LED says nothing"
    );
}

// --- what the planes keep ---------------------------------------------

#[test]
fn a_watchdog_reboot_keeps_off_slots_and_a_power_loss_re_applies_them() {
    let mut harness = Harness::new(&scenario());
    harness.verb_ok(NodeCommand::PowerOff { node: 9 }, 3_000);
    assert_eq!(harness.client.intended_off(), &[9]);
    harness.clear();

    // A watchdog reset keeps VDD, so the plane restores the OFF slot
    // from its backup registers and says nothing was lost.
    harness
        .fake(0x31)
        .reboot(plane_logic::reset_cause::WATCHDOG);
    assert!(harness.run_until(15_000, |h| {
        h.saw(|e| matches!(e, ClientEvent::PlaneRebooted { address: 0x31, .. }))
    }));
    harness.run(2_000);
    assert!(
        !harness.saw(|e| matches!(e, ClientEvent::IntendedOffReapplied { .. })),
        "nothing to re-apply: the plane kept its state"
    );
    assert!(!harness.status(9).contains(SlotStatus::ENABLED));
    assert!(
        !harness
            .client
            .plane(0x31)
            .expect("0x31")
            .intended_state_lost()
    );

    // A power loss takes the backup registers with it: the slot comes
    // back ON, the plane flags it, and the client puts the node back
    // off (ADR 0004, "What an Expert Would Ask").
    harness.clear();
    harness.fake(0x31).power_loss();
    assert!(
        harness.run_until(20_000, |h| h.saw(|e| matches!(
            e,
            ClientEvent::IntendedOffReapplied { address: 0x31, .. }
        ))),
        "the client re-applied OFF"
    );
    assert!(
        harness.run_until(5_000, |h| !h.status(9).contains(SlotStatus::ENABLED)),
        "and the rail is down again"
    );
    assert_eq!(harness.client.intended_off(), &[9]);

    // Only once per boot of the plane: the flag stays up until it
    // reboots again, and the client does not keep writing.
    let reapplies = harness
        .events
        .iter()
        .filter(|e| matches!(e, ClientEvent::IntendedOffReapplied { .. }))
        .count();
    harness.run(10_000);
    assert_eq!(
        harness
            .events
            .iter()
            .filter(|e| matches!(e, ClientEvent::IntendedOffReapplied { .. }))
            .count(),
        reapplies,
        "the re-apply happens once per plane boot"
    );
}

#[test]
fn a_persisted_intended_off_set_is_re_applied_to_a_plane_that_lost_its_state() {
    // The controller rebooted too: the set comes from NVS and the
    // planes are found fresh, flagging "intended state lost" because
    // their backup registers were empty at power-on.
    let mut link = FakePlaneLink::from_scenario(&scenario());
    let mut now_ms = 0;
    while now_ms < 2_000 {
        now_ms += POLL_MS;
        link.tick(now_ms);
    }
    let mut harness = Harness {
        link,
        client: PlaneClient::new(SEED),
        now_ms,
        events: Vec::new(),
        jobs: Vec::new(),
    };
    harness.client.load_intended_off(&[2, 9]);
    assert!(
        !harness.client.take_intent_dirty(),
        "a load is not a change to store back"
    );

    // Both planes get their OFF nodes back. The client keeps at it
    // while a slot it wants off reports itself enabled, so a plane that
    // was busy with its own boot sequence may take a couple of goes.
    let down = |h: &Harness, id: u8| {
        h.client
            .node(id)
            .is_some_and(|node| node.summary_ms != 0 && !node.status.contains(SlotStatus::ENABLED))
    };
    assert!(
        harness.run_until(15_000, |h| down(h, 2) && down(h, 9)),
        "the rails never went down again"
    );
    for address in [0x30u8, 0x31] {
        assert!(
            harness.saw(|e| matches!(
                e,
                ClientEvent::IntendedOffReapplied { address: a, .. } if *a == address
            )),
            "plane {address:#x} never got its OFF back"
        );
    }
    // And once the rails are down it stops: nothing to put back.
    let before = harness
        .events
        .iter()
        .filter(|e| matches!(e, ClientEvent::IntendedOffReapplied { .. }))
        .count();
    harness.run(10_000);
    assert_eq!(
        harness
            .events
            .iter()
            .filter(|e| matches!(e, ClientEvent::IntendedOffReapplied { .. }))
            .count(),
        before,
        "the re-apply stops once the slots are off"
    );
    assert_eq!(harness.client.intended_off(), &[2, 9]);
}

#[test]
fn a_unit_unplugged_takes_its_nodes_absent_and_a_replug_brings_them_back() {
    let mut harness = Harness::new(&scenario());
    harness.verb_ok(NodeCommand::PowerOff { node: 3 }, 3_000);
    harness.clear();

    // Unit 1's chain cable comes out: no 3V3, so the expander forgets
    // its outputs and the slots are gone.
    harness.fake(0x30).set_unit_power(1, false);
    assert!(
        harness.run_until(10_000, |h| h.node(3).power == PowerState::Absent),
        "a node without its unit is absent"
    );
    assert!(harness.saw(|e| matches!(
        e,
        ClientEvent::Unit {
            unit: 1,
            kind: EventKind::UnitLost,
            ..
        }
    )));
    let plane = harness.client.plane(0x30).expect("0x30");
    assert!(plane.units[1].status.contains(UnitStatus::LOST));
    assert!(plane.flags.contains(StatusFlags::ANY_UNIT_FAULT));
    assert_eq!(
        harness.client.nodes().map(|n| n.id).collect::<Vec<_>>(),
        vec![1, 2, 9, 10],
        "the absent nodes are not listed"
    );

    // A command for a node whose unit is gone fails without a write.
    harness.client.submit(harness.now_ms, NodeCommand::on(3));
    assert_eq!(harness.client.drain_jobs()[0].outcome, JobOutcome::Absent);

    // And back in: the plane puts the OFF slot back off by itself
    // (ADR 0002 decision 4), and the client sees the unit again.
    harness.clear();
    harness.fake(0x30).set_unit_power(1, true);
    assert!(
        harness.run_until(15_000, |h| h.node(4).power == PowerState::On),
        "the unit's other node came back on"
    );
    assert!(harness.saw(|e| matches!(
        e,
        ClientEvent::Unit {
            unit: 1,
            kind: EventKind::UnitPresent,
            ..
        }
    )));
    assert_eq!(
        harness.node(3).power,
        PowerState::Off,
        "OFF went back OFF without the controller asking"
    );
}

// --- config -----------------------------------------------------------

#[test]
fn a_config_set_is_pushed_when_the_plane_hash_does_not_match() {
    let mut harness = Harness::new(&scenario());
    let before = harness.client.plane(0x30).expect("0x30").config_hash;
    harness.clear();

    harness.client.set_plane_config(
        0x30,
        DesiredConfig::new(
            vec![
                ConfigRecord::PowerLimit(PowerLimit {
                    slot: 0,
                    watts: 7_500,
                }),
                ConfigRecord::PowerLimit(PowerLimit {
                    slot: 1,
                    watts: 7_500,
                }),
            ],
            None,
        ),
    );
    assert!(
        harness.run_until(5_000, |h| h
            .saw(|e| matches!(e, ClientEvent::ConfigPushed { address: 0x30, .. }))),
        "the set went out and the hash moved"
    );
    let plane = harness.client.plane(0x30).expect("0x30");
    assert_ne!(plane.config_hash, before);
    assert_eq!(
        harness
            .client
            .plane_config(0x30)
            .expect("the set")
            .stored_hash(),
        Some(plane.config_hash),
        "the client remembers the hash this set produces"
    );
    assert!(harness.saw(|e| matches!(
        e,
        ClientEvent::Plane {
            kind: EventKind::ConfigStored,
            ..
        }
    )));
    // The limit really landed on the slot.
    assert!(
        harness.run_until(12_000, |h| h.node(1).telemetry.power_limit == 7_500),
        "the plane put the limit in force"
    );

    // And nothing more is written: the hash matches now.
    harness.clear();
    let (_, writes) = harness.client.transaction_counts();
    harness.run(10_000);
    let (_, writes_after) = harness.client.transaction_counts();
    assert_eq!(writes, writes_after, "no second push");

    // A plane that comes back with the same set is left alone: the
    // client was given the hash, so it does not write at all.
    let hash = harness.client.plane(0x30).expect("0x30").config_hash;
    let mut second = PlaneClient::new(SEED ^ 1);
    second.set_plane_config(
        0x30,
        DesiredConfig::new(
            vec![ConfigRecord::PowerLimit(PowerLimit {
                slot: 0,
                watts: 7_500,
            })],
            Some(hash),
        ),
    );
    harness.client = second;
    harness.clear();
    let (_, writes) = harness.client.transaction_counts();
    harness.run(5_000);
    let (_, after) = harness.client.transaction_counts();
    assert!(
        after - writes <= 2,
        "only the event acknowledgements, no config write"
    );
    assert!(!harness.saw(|e| matches!(e, ClientEvent::ConfigPushed { .. })));
}

// --- firmware updates -------------------------------------------------

#[test]
fn a_plane_firmware_update_runs_to_a_confirmed_image() {
    let mut harness = Harness::new(&scenario());
    let (bytes, crc) = image(2_048);
    let version = Version::new(0, 2, 0);
    harness.clear();
    assert_eq!(harness.client.policy(), UpdatePolicy::Manual);
    assert_eq!(
        harness.client.update_candidates(Version::new(0, 1, 0)),
        Vec::<u8>::new(),
        "both planes run that version already"
    );
    assert_eq!(harness.client.update_candidates(version), vec![0x30, 0x31]);

    let boot_before = harness.client.plane(0x31).expect("0x31").boot_count;
    harness
        .client
        .start_update(
            harness.now_ms,
            0x31,
            Box::new(BufferedImage::new(bytes.clone(), crc, version)),
        )
        .expect("the plane is there and idle");
    // One at a time (ADR 0004 decision 8).
    assert!(
        harness
            .client
            .start_update(
                harness.now_ms,
                0x30,
                Box::new(BufferedImage::new(bytes.clone(), crc, version))
            )
            .is_err()
    );

    assert!(
        harness.run_until(60_000, |h| h
            .saw(|e| matches!(e, ClientEvent::UpdateConfirmed { address: 0x31, .. }))),
        "the update did not finish: {:?}",
        harness.client.update_stage()
    );
    assert!(harness.client.update_stage().is_none());

    // The image is in the plane's slot, the plane rebooted into it and
    // the client confirmed it.
    let fake = harness.fake(0x31);
    assert_eq!(fake.sim.dev.update.image(2_048), &bytes[..]);
    assert!(fake.sim.dev.update.booted);
    assert!(fake.resets >= 1, "the swap took a reset");
    let plane = harness.client.plane(0x31).expect("0x31");
    assert!(plane.boot_count > boot_before);
    assert_eq!(plane.image_state, ImageState::Confirmed);
    assert!(harness.saw(|e| matches!(
        e,
        ClientEvent::Plane {
            kind: EventKind::ImageConfirmed,
            ..
        }
    )));

    // The other plane is still there and still polled.
    assert_eq!(harness.client.addresses(), vec![0x30, 0x31]);
    assert_eq!(harness.node(1).power, PowerState::On);
}

#[test]
fn an_update_whose_plane_comes_back_different_is_not_confirmed_and_rolls_back() {
    let mut harness = Harness::new(&scenario());
    let (bytes, crc) = image(1_024);
    let version = Version::new(0, 3, 0);
    harness.clear();

    harness
        .client
        .start_update(
            harness.now_ms,
            0x30,
            Box::new(BufferedImage::new(bytes.clone(), crc, version)),
        )
        .expect("the plane is there and idle");

    // While the image goes over, a unit board is unplugged. The plane
    // that comes back has fewer units than the one that went down, so
    // the client refuses to confirm (ADR 0004 decision 8) and the
    // plane's own probation rolls the image back.
    assert!(harness.run_until(30_000, |h| matches!(
        h.client.update_stage(),
        Some((_, granite_core::plane::UpdateStage::Sending, p)) if p > 20
    )));
    harness.fake(0x30).set_unit_power(1, false);

    assert!(
        harness.run_until(60_000, |h| h.saw(|e| matches!(
            e,
            ClientEvent::UpdateFailed {
                address: 0x30,
                reason: UpdateFailure::UnitsChanged,
                ..
            }
        ))),
        "the client should refuse to confirm: {:?}",
        harness.client.update_stage()
    );
    assert!(harness.client.update_stage().is_none());

    // The probation runs out, the plane resets and the bootloader puts
    // the old image back.
    assert!(
        harness.run_until(plane_logic::plane::PROBATION_MS + 20_000, |h| h
            .client
            .plane(0x30)
            .is_some_and(|p| p.image_state == ImageState::RolledBack)),
        "the plane rolled back on its own"
    );
    assert!(
        harness.saw(|e| matches!(
            e,
            ClientEvent::Plane {
                kind: EventKind::RolledBack,
                ..
            }
        )) || harness.client.plane(0x30).expect("0x30").image_state == ImageState::RolledBack
    );
    assert!(!harness.fake(0x30).sim.dev.update.unconfirmed);
}

#[test]
fn an_update_is_refused_while_a_sequence_runs() {
    let mut harness = Harness::new(&scenario());
    let (bytes, crc) = image(512);
    let version = Version::new(0, 2, 0);

    // A node off on a host that will not listen keeps a sequence
    // running for a while.
    harness.client.submit(harness.now_ms, NodeCommand::off(9));
    assert!(harness.run_until(5_000, |h| {
        h.client.plane(0x31).is_some_and(|p| p.sequence_running())
    }));
    let refused = harness.client.start_update(
        harness.now_ms,
        0x31,
        Box::new(BufferedImage::new(bytes.clone(), crc, version)),
    );
    assert!(matches!(
        refused,
        Err(granite_core::plane::UpdateRefused::SequenceRunning)
            | Err(granite_core::plane::UpdateRefused::CommandsInFlight)
    ));

    // And an address with no plane is refused outright.
    assert_eq!(
        harness.client.start_update(
            harness.now_ms,
            0x35,
            Box::new(BufferedImage::new(bytes.clone(), crc, version))
        ),
        Err(granite_core::plane::UpdateRefused::PlaneAbsent)
    );
}

// --- the sample scenario ----------------------------------------------

#[test]
fn the_sample_scenario_runs_through_its_faults() {
    let mut harness = Harness::new(&sample());
    assert_eq!(harness.client.addresses(), vec![0x30, 0x31]);
    assert_eq!(
        harness
            .client
            .plane(0x30)
            .expect("0x30")
            .units_present
            .bits(),
        0b1111
    );
    // Eight slots on four units; slot 5 has no module, so seven nodes.
    let present: Vec<u8> = harness
        .client
        .nodes()
        .filter(|n| n.address == 0x30)
        .map(|n| n.id)
        .collect();
    assert_eq!(present, vec![1, 2, 3, 4, 5, 7, 8]);
    assert_eq!(harness.node(4).host, granite_core::plane::HostState::Sleep);

    // The scenario's faults land on the client's models as it runs: a
    // dropped read, a corrupted answer, a unit out and back, a power
    // limit trip and a plane that loses its power.
    harness.run(100_000);
    assert!(harness.client.errors(0x30).bad_crc > 0);
    assert!(harness.saw(|e| matches!(
        e,
        ClientEvent::Unit {
            kind: EventKind::UnitLost,
            ..
        }
    )));
    assert!(harness.saw_node_kind(7, EventKind::PowerLimitTrip));
    assert_eq!(harness.node(7).power, PowerState::Fault);
    assert!(harness.saw(
        |e| matches!(e, ClientEvent::PlaneRebooted { address: 0x31, .. })
            || matches!(e, ClientEvent::PlaneFound { address: 0x31, .. })
    ));
    assert!(
        harness
            .client
            .plane(0x31)
            .expect("0x31")
            .intended_state_lost()
    );
}
