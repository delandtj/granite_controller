//! The plane client over the fake CAN FD link (firmware ADR 0005).
//!
//! The same [`granite_core::plane::PlaneClient`] and the same simulated
//! planes as `plane_client.rs`, with [`granite_sim::can::CanBus`]
//! between them: every request, answer, EVENT and SYNC is segmented by
//! `granite_proto::can::segment` and put back together by a
//! `Reassembler` on the other side.
//!
//! The properties:
//!
//! - the client discovers the planes and fills its models over the CAN
//!   transport, with no message lost and nothing dropped by either
//!   reassembler,
//! - a message longer than one frame really is segmented,
//! - EVENT names the plane that raised it (decision 6), repeats every
//!   second while events are unacknowledged, and stops once EV_ACK has
//!   covered them; the I2C link answers 0xFF instead, because EXP_INT
//!   cannot say who pulled it,
//! - SYNC goes out once a second with a rising epoch (decision 7),
//! - a lost frame is a timeout the ADR 0003 retry recovers from, and a
//!   bus that loses one frame in twelve still keeps the planes, their
//!   models and the verbs.

use granite_core::hal::{PlaneLink, Switch};
use granite_core::plane::{
    ClientEvent, FinishedJob, JobId, JobOutcome, NodeCommand, PlaneClient, Transactions,
};
use granite_proto::{BLK_STATUS, INDEX_NONE, frame};
use granite_sim::Scenario;
use granite_sim::can::{EVENT_PERIOD_MS, FrameLoss};
use granite_sim::planes::{FakePlaneLink, PLANE_TICK_MS};
use granite_sim::scenario::{CanScenario, LinkTransport, PlaneScenario, PlaneSlotScenario};

/// How often the firmware calls `poll` \[ms\].
const POLL_MS: u64 = PLANE_TICK_MS;

/// The client's sequence and epoch seed.
const SEED: u32 = 0x5EED_1234;

// --- the harness ------------------------------------------------------

/// The client, the planes behind a CAN link, and one clock for both.
struct Harness {
    link: FakePlaneLink,
    client: PlaneClient,
    now_ms: u64,
    events: Vec<ClientEvent>,
    jobs: Vec<FinishedJob>,
}

impl Harness {
    /// A harness over `scenario`, settled the way `plane_client.rs`
    /// settles its own: the planes boot first, then the client
    /// discovers them.
    fn new(scenario: &Scenario) -> Self {
        let mut link = FakePlaneLink::from_scenario(scenario);
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

    /// One step of the clock: the planes first, then the client.
    fn step(&mut self) {
        self.now_ms += POLL_MS;
        self.link.tick(self.now_ms);
        self.client.poll(self.now_ms, &mut self.link);
        self.events.append(&mut self.client.drain_events());
        self.jobs.append(&mut self.client.drain_jobs());
    }

    /// Run both sides for `ms`.
    fn run(&mut self, ms: u64) {
        let until = self.now_ms + ms;
        while self.now_ms < until {
            self.step();
        }
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

    /// A finished job by id.
    fn finished(&self, id: JobId) -> Option<&FinishedJob> {
        self.jobs.iter().find(|j| j.id == id)
    }

    /// Submit a command and wait for it to finish, at most `ms`.
    fn do_verb(&mut self, command: impl Into<granite_core::plane::Command>, ms: u64) -> JobOutcome {
        let id = self.client.submit(self.now_ms, command);
        assert!(
            self.run_until(ms, |h| h.finished(id).is_some()),
            "the job did not finish inside {ms} ms"
        );
        self.finished(id).expect("just checked").outcome
    }

    /// The bus under the link.
    fn bus(&self) -> &granite_sim::can::CanBus {
        self.link.can().expect("the scenario asked for a CAN link")
    }
}

// --- scenarios --------------------------------------------------------

/// One slot with a module and a running host.
fn slot(slot: u8, host_on: bool) -> PlaneSlotScenario {
    PlaneSlotScenario {
        slot,
        host_on,
        load_ma: 1_000,
        ..Default::default()
    }
}

/// A plane with its blink window shortened, so a wait on a host LED is
/// about a second rather than five.
fn plane(address: u8, units: Vec<u8>, slots: Vec<PlaneSlotScenario>) -> PlaneScenario {
    PlaneScenario {
        address,
        units,
        blink_window_ms: Some([100, 500]),
        slots,
        faults: Vec::new(),
    }
}

/// Two planes, over `link`, losing one frame in `drop_every`.
fn scenario(link: LinkTransport, drop_every: u32) -> Scenario {
    let mut scenario = Scenario {
        link,
        can: CanScenario { drop_every },
        planes: vec![
            plane(0x30, vec![0, 1], (0..4).map(|s| slot(s, true)).collect()),
            plane(0x31, vec![0], vec![slot(0, true), slot(1, false)]),
        ],
        ..Default::default()
    };
    scenario.normalise();
    scenario
}

/// A clean CAN bus with both planes on it.
fn can() -> Scenario {
    scenario(LinkTransport::Can, 0)
}

// --- the transport ----------------------------------------------------

#[test]
fn the_client_runs_over_the_can_transport() {
    let mut harness = Harness::new(&can());
    assert_eq!(harness.client.addresses(), vec![0x30, 0x31]);
    // The models are filled from the same blocks as on I2C.
    let plane = harness.client.plane(0x30).expect("plane 0x30 answers");
    assert_eq!(plane.present_nodes().count(), 4);
    assert!(harness.client.node(1).expect("node 1").summary_ms > 0);
    assert!(harness.client.node(1).expect("node 1").detail_ms > 0);
    assert_eq!(harness.client.errors(0x30).failed, 0);

    // Nothing was lost, so no reassembler dropped anything: every
    // message arrived whole and in order (ADR 0005 decision 4).
    let bus = harness.bus();
    assert!(bus.messages > 50, "only {} messages", bus.messages);
    assert_eq!((bus.dropped, bus.lost_messages), (0, 0));
    assert_eq!(bus.controller_counters().total(), 0);
    for node in 0..2 {
        assert_eq!(bus.plane_counters(node).total(), 0);
    }
    // And the transport really is segmenting: more frames than
    // messages means at least one message did not fit in one frame
    // (decision 3).
    assert!(
        bus.frames > bus.messages,
        "{} frames for {} messages: nothing was segmented",
        bus.frames,
        bus.messages
    );

    // A verb still works end to end.
    harness.run(1_000);
    assert_eq!(
        harness.do_verb(
            NodeCommand::Press {
                node: 1,
                switch: Switch::Pwr,
                ms: 200,
            },
            10_000
        ),
        JobOutcome::Done
    );
}

#[test]
fn a_long_message_takes_several_frames() {
    let mut link = FakePlaneLink::from_scenario(&can());
    link.tick(2_000);
    let request = frame::read_request(BLK_STATUS, INDEX_NONE);
    let mut buf = [0u8; frame::MAX_ANSWER];

    let before = link.can().expect("a CAN link").frames;
    let n = link
        .write_read(0x30, &request, &mut buf)
        .expect("the plane answered");
    let payload = frame::check_answer(0x30, &request, &buf[..n]).expect("the answer checks out");
    assert!(!payload.is_empty());
    let frames = link.can().expect("a CAN link").frames - before;
    // One frame carries the request; the answer takes as many as its
    // own length asks for.
    let answer_frames = granite_proto::can::segment_count(payload.len() + 2);
    assert_eq!(frames, 1 + answer_frames as u64);
}

// --- events (decision 6) ---------------------------------------------

#[test]
fn event_frames_name_the_plane_that_raised_them() {
    let mut link = FakePlaneLink::from_scenario(&can());
    // The planes boot and raise their boot events; nobody has read
    // them, so both send EVENT from their own node.
    link.tick(PLANE_TICK_MS);
    assert_eq!(link.event_nodes(), 0b11);
    // Reading the mask clears it: a plane repeats its EVENT once a
    // second, not once a poll.
    assert_eq!(link.event_nodes(), 0);
    link.tick(PLANE_TICK_MS * 2);
    assert_eq!(link.event_nodes(), 0);
    link.tick(PLANE_TICK_MS * 2 + EVENT_PERIOD_MS);
    assert_eq!(link.event_nodes(), 0b11);
    assert!(link.can().expect("a CAN link").events >= 4);

    // The same planes on an I2C link: EXP_INT is the wired OR, so the
    // mask is every plane while it is low.
    let mut i2c = FakePlaneLink::from_scenario(&scenario(LinkTransport::I2c, 0));
    i2c.tick(PLANE_TICK_MS);
    assert_eq!(i2c.event_nodes(), 0xFF);
    assert_eq!(i2c.event_nodes(), 0xFF, "the line is still low");
}

#[test]
fn an_event_becomes_a_status_read_and_ev_ack_stops_it() {
    let mut harness = Harness::new(&can());
    // The boot events have been read and acknowledged during the
    // settle, so no plane is sending EVENT any more.
    assert!(
        harness.run_until(3_000, |h| h.link.planes().all(|p| !p.event_pending())),
        "every event the planes raised has been acknowledged"
    );
    let events_before = harness.bus().events;
    harness.events.clear();

    // A press takes the host down: the plane raises a sequence-done and
    // an LED change, sends EVENT, and the client reads EVENTS and
    // acknowledges them.
    assert_eq!(
        harness.do_verb(
            NodeCommand::Press {
                node: 4,
                switch: Switch::Pwr,
                ms: 200,
            },
            10_000
        ),
        JobOutcome::Done
    );
    assert!(
        harness.run_until(6_000, |h| h.events.iter().any(|e| matches!(
            e,
            ClientEvent::Node {
                kind: granite_proto::EventKind::SequenceDone,
                ..
            }
        )))
    );
    assert!(
        harness.bus().events > events_before,
        "the plane sent EVENT frames"
    );
    assert!(
        harness.run_until(4_000, |h| h.link.planes().all(|p| !p.event_pending())),
        "EV_ACK covered them"
    );
}

// --- SYNC (decision 7) -----------------------------------------------

#[test]
fn sync_goes_out_once_a_second_with_a_rising_epoch() {
    let mut harness = Harness::new(&can());
    let syncs = harness.bus().syncs;
    let epoch = harness.bus().last_epoch.expect("a SYNC has gone out");
    assert_eq!(epoch + 1, harness.client.sync_epoch());

    harness.run(5_000);
    assert_eq!(harness.bus().syncs, syncs + 5);
    assert_eq!(
        harness.bus().last_epoch,
        Some(harness.client.sync_epoch() - 1)
    );

    // An I2C link has nowhere to put a SYNC and says so by doing
    // nothing: the default body of the trait method.
    let mut i2c = FakePlaneLink::from_scenario(&scenario(LinkTransport::I2c, 0));
    assert!(i2c.sync(7).is_ok());
    assert!(i2c.can().is_none());
}

// --- frame loss -------------------------------------------------------

#[test]
fn a_lost_frame_is_a_timeout_the_retry_recovers_from() {
    // The scenario is the I2C one, with a CAN bus put under it by hand:
    // the transport is the link's business, not the planes'.
    let mut link =
        FakePlaneLink::from_scenario(&scenario(LinkTransport::I2c, 0)).with_can(FrameLoss::NONE);
    link.tick(2_000);
    let mut tx = Transactions::new();
    let mut buf = [0u8; frame::MAX_ANSWER];
    // The planes' boot EVENTs have been on the bus during the tick; the
    // loss starts now, so it hits the read and nothing else.
    let lost_before = link.can().expect("a CAN link").lost_messages;
    link.can.as_mut().expect("a CAN link").loss.burst = 1;

    // The request's only frame is the one that goes missing, so the
    // first try times out and the second answers (ADR 0003: three
    // tries).
    let n = tx
        .read(&mut link, 0x30, BLK_STATUS, INDEX_NONE, &mut buf)
        .expect("the retry got through");
    assert!(n > 0);
    assert_eq!(tx.errors(0x30).link, 1);
    assert_eq!(tx.errors(0x30).failed, 0);
    assert_eq!(
        link.can().expect("a CAN link").lost_messages,
        lost_before + 1
    );

    // Three frames gone is one frame per try: the read fails, and the
    // plane is not lost until a second transaction fails too.
    link.can.as_mut().expect("a CAN link").loss.burst = 3;
    assert!(
        tx.read(&mut link, 0x30, BLK_STATUS, INDEX_NONE, &mut buf)
            .is_err()
    );
    assert_eq!(tx.errors(0x30).failed, 1);
    assert!(!tx.errors(0x30).is_lost());
    // And the next read, on a clean bus, is good again.
    assert!(
        tx.read(&mut link, 0x30, BLK_STATUS, INDEX_NONE, &mut buf)
            .is_ok()
    );
    assert_eq!(tx.errors(0x30).consecutive_failures, 0);
}

#[test]
fn the_client_keeps_its_planes_on_a_lossy_bus() {
    let mut harness = Harness::new(&scenario(LinkTransport::Can, 12));
    // Discovery survived the loss.
    assert_eq!(harness.client.addresses(), vec![0x30, 0x31]);
    harness.run(10_000);

    let bus = harness.bus();
    assert!(bus.dropped > 0, "the bus lost nothing");
    assert!(bus.lost_messages > 0, "no message was ever incomplete");
    // Some of the loss hit a message that had already started, which is
    // what the reassemblers count (decision 4).
    assert!(
        bus.controller_counters().total() + bus.plane_counters(0).total() > 0,
        "no partial message was ever dropped"
    );
    // The transactions were retried, not given up on: the client still
    // has both planes and fresh telemetry for their nodes.
    assert!(harness.client.errors(0x30).link > 0, "nothing was retried");
    assert_eq!(harness.client.present_count(), 2);
    for node in [1u8, 4, 9] {
        let seen = harness.client.node(node).expect("the node is there");
        assert!(
            seen.summary_ms + 4_000 > harness.now_ms,
            "node {node} telemetry is {} ms old",
            harness.now_ms - seen.summary_ms
        );
    }
    // And a verb still completes.
    assert_eq!(
        harness.do_verb(
            NodeCommand::Press {
                node: 1,
                switch: Switch::Pwr,
                ms: 200,
            },
            20_000
        ),
        JobOutcome::Done
    );
}
