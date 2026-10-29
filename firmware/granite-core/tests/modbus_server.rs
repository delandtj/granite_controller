//! The Modbus TCP frame handler: one test per function code, the
//! exceptions the map calls for, and the allow-list.
//!
//! Frames are written out byte by byte on purpose. A PLC integrator reads
//! these as the wire contract, so a change here is a change to what is
//! already deployed.

use granite_core::Target;
use granite_core::modbus_map::{FwVersion, MAP_VERSION, ResultCode, cmd_word, fault_bits};
use granite_core::modbus_server::{
    AllowList, Cidr, MBAP_HEADER, RegisterImage, handle_frame, mbap_payload_len,
};
use granite_core::msg::{Command, CommandKind, Reply};
use granite_core::observed::{NodeObs, Observed, PlaneObs, ProbeObs, Stamped};
use granite_core::plane::Plane;
use granite_core::plane::model::{HostState, PowerState};

const UNIT: u8 = 1;

/// Node command words and coils start here in map version 2.
const NODE: u16 = 100;

fn snapshot() -> Observed {
    let mut o = Observed::new();
    let mut plane = PlaneObs::of_plane(&Plane::new(0x30));
    plane.info_ms = 5;
    plane.ts_ms = 10;
    o.planes.push(plane);
    o.nodes.push(node(1, PowerState::On, HostState::On));
    o.nodes.push(node(2, PowerState::Off, HostState::Unknown));
    o.nodes.push(node(3, PowerState::On, HostState::Off));
    o.probes.push(ProbeObs {
        rom: 1,
        name: "inlet".into(),
        centi_c: Some(2_537),
        ts_ms: 10,
    });
    o.board_temp = Stamped::new(Some(3_100), 10);
    o.vin_mv = Stamped::new(Some(12_120), 10);
    o.dry_in = Stamped::new(Some(0b0001), 10);
    o.link_up = Stamped::new(true, 10);
    o.uptime_s = Stamped::new(0x0001_2345, 10);
    o
}

/// One present node of plane 0.
fn node(id: granite_core::NodeId, power: PowerState, host: HostState) -> NodeObs {
    let (address, slot) = granite_core::plane::node_location(id).expect("a node id");
    let mut obs = NodeObs::unknown(id, address, slot);
    obs.present = true;
    obs.power = power;
    obs.host = host;
    obs.ts_ms = 10;
    obs
}

fn image(o: &Observed) -> RegisterImage<'_> {
    RegisterImage::new(
        o,
        FwVersion { major: 0, minor: 1 },
        fault_bits::NODE_FAULT,
        ResultCode::Idle,
    )
}

/// One MBAP frame: transaction id 0x1234, unit [`UNIT`].
fn frame(pdu: &[u8]) -> Vec<u8> {
    let mut f = vec![0x12, 0x34, 0x00, 0x00];
    let len = pdu.len() + 1;
    f.extend_from_slice(&(len as u16).to_be_bytes());
    f.push(UNIT);
    f.extend_from_slice(pdu);
    f
}

fn read_pdu(func: u8, addr: u16, count: u16) -> Vec<u8> {
    let mut p = vec![func];
    p.extend_from_slice(&addr.to_be_bytes());
    p.extend_from_slice(&count.to_be_bytes());
    p
}

/// A dispatcher that records every command and answers `ok`.
#[derive(Default)]
struct Recorder {
    seen: Vec<Command>,
    reply: Option<Reply>,
}

impl Recorder {
    fn run(&mut self, request: &[u8], img: &RegisterImage<'_>) -> Outcome {
        let mut seq = 0u32;
        let mut seen = Vec::new();
        let canned = self.reply.clone();
        let out = handle_frame(UNIT, request, img, &mut seq, &mut |cmd| {
            seen.push(cmd.clone());
            canned.clone().unwrap_or_else(|| Reply::ok(&cmd.id, "ok"))
        });
        self.seen = seen;
        Outcome {
            response: out.response,
            broken: out.broken,
            result: out.result,
        }
    }
}

struct Outcome {
    response: Vec<u8>,
    broken: bool,
    result: Option<ResultCode>,
}

impl Outcome {
    /// The PDU, with the MBAP header checked and stripped.
    fn pdu(&self) -> &[u8] {
        assert!(
            self.response.len() > MBAP_HEADER,
            "no response: {:?}",
            self.response
        );
        assert_eq!(&self.response[0..2], &[0x12, 0x34], "transaction id echoed");
        assert_eq!(&self.response[2..4], &[0x00, 0x00], "protocol id 0");
        let len = u16::from_be_bytes([self.response[4], self.response[5]]) as usize;
        assert_eq!(len, self.response.len() - MBAP_HEADER, "MBAP length");
        assert_eq!(self.response[6], UNIT, "unit id");
        &self.response[7..]
    }
}

fn serve(request: &[u8]) -> Outcome {
    let o = snapshot();
    Recorder::default().run(request, &image(&o))
}

/// Run one frame against the default snapshot with a recorder.
fn record(request: &[u8]) -> (Recorder, Outcome) {
    let o = snapshot();
    let mut rec = Recorder::default();
    let out = rec.run(request, &image(&o));
    (rec, out)
}

// -------------------------------------------------------------------
// Reads
// -------------------------------------------------------------------

#[test]
fn fc1_reads_the_node_rail_coils() {
    let out = serve(&frame(&read_pdu(1, NODE, 3)));
    // func, byte count, one byte of bits: nodes 1 and 3 on, node 2 off.
    assert_eq!(out.pdu(), &[1, 1, 0b0000_0101]);
}

#[test]
fn fc2_reads_the_discrete_inputs() {
    let out = serve(&frame(&read_pdu(2, 8, 6)));
    let pdu = out.pdu();
    assert_eq!(pdu[0], 2);
    assert_eq!(pdu[1], 1, "6 bits take one byte");
    // bit 0 dry 1, bit 4 link, bit 5 MQTT.
    assert_eq!(pdu[2], 0b0001_0001);

    // The node bits live in their own blocks.
    let power = serve(&frame(&read_pdu(2, 100, 3)));
    assert_eq!(power.pdu(), &[2, 1, 0b0000_0101]);
    let host = serve(&frame(&read_pdu(2, 200, 3)));
    assert_eq!(host.pdu(), &[2, 1, 0b0000_0001], "only node 1's host is on");
}

#[test]
fn fc3_reads_the_holding_registers() {
    let out = serve(&frame(&read_pdu(3, 8, 2)));
    let pdu = out.pdu();
    assert_eq!(pdu[0], 3);
    assert_eq!(pdu[1], 4);
    let words: Vec<u16> = pdu[2..]
        .chunks(2)
        .map(|c| u16::from_be_bytes([c[0], c[1]]))
        .collect();
    assert_eq!(words[0], 0, "the on_all trigger reads 0");
    assert_eq!(words[1], ResultCode::Idle.as_u16());

    let nodes = serve(&frame(&read_pdu(3, NODE, 2)));
    let pdu = nodes.pdu();
    let words: Vec<u16> = pdu[2..]
        .chunks(2)
        .map(|c| u16::from_be_bytes([c[0], c[1]]))
        .collect();
    assert_eq!(words[0], 2, "node 1's word reads back on");
    assert_eq!(words[1], 1, "node 2 is off");
}

#[test]
fn fc4_reads_the_input_registers() {
    let out = serve(&frame(&read_pdu(4, 0, 17)));
    let pdu = out.pdu();
    assert_eq!(pdu[0], 4);
    let words: Vec<u16> = pdu[2..]
        .chunks(2)
        .map(|c| u16::from_be_bytes([c[0], c[1]]))
        .collect();
    assert_eq!(words[0], 2_537, "probe 1 in centi-degrees");
    assert_eq!(words[8], 3_100, "board temp");
    assert_eq!(words[9], 12_120, "vin in mV");
    assert_eq!(words[10], 1, "uptime high word");
    assert_eq!(words[11], 0x2345, "uptime low word");
    assert_eq!(words[12], FwVersion { major: 0, minor: 1 }.as_u16());
    assert_eq!(words[13], fault_bits::NODE_FAULT);
    assert_eq!(words[14], MAP_VERSION);
    assert_eq!(words[15], 0b1, "plane 0 answers");
    assert_eq!(words[16], 3, "three nodes are present");
}

#[test]
fn a_read_of_one_register_in_the_middle_works() {
    let out = serve(&frame(&read_pdu(4, 9, 1)));
    assert_eq!(out.pdu(), &[4, 2, 0x2f, 0x58], "vin 12120 = 0x2f58");
}

#[test]
fn a_read_of_a_node_block_works() {
    let out = serve(&frame(&read_pdu(4, 100, 2)));
    assert_eq!(out.pdu(), &[4, 4, 0, 2, 0, 2], "node 1: on, host on");
}

// -------------------------------------------------------------------
// Read exceptions
// -------------------------------------------------------------------

#[test]
fn a_read_that_runs_off_a_block_is_illegal_data_address() {
    // One past the last node, one past the last plane, and a span that
    // starts inside a block and leaves it.
    for (func, addr, count) in [
        (1u8, 100u16, 65u16),
        (2, 200, 65),
        (3, 100, 65),
        (4, 100 + 12 * 63, 13),
        (4, 900 + 16 * 7, 17),
    ] {
        let out = serve(&frame(&read_pdu(func, addr, count)));
        assert_eq!(
            out.pdu(),
            &[func + 0x80, 0x02],
            "func {func} at {addr} for {count}"
        );
    }
}

#[test]
fn a_read_starting_outside_the_map_is_illegal_data_address() {
    // The rev C node registers are not in version 2 at all.
    let out = serve(&frame(&read_pdu(3, 0, 1)));
    assert_eq!(out.pdu(), &[0x83, 0x02]);
    let coil = serve(&frame(&read_pdu(1, 0, 1)));
    assert_eq!(coil.pdu(), &[0x81, 0x02]);
    let input = serve(&frame(&read_pdu(4, 50, 1)));
    assert_eq!(input.pdu(), &[0x84, 0x02]);
}

#[test]
fn a_zero_quantity_read_is_illegal_data_value() {
    let out = serve(&frame(&read_pdu(3, 8, 0)));
    assert_eq!(out.pdu(), &[0x83, 0x03]);
}

#[test]
fn an_unsupported_function_is_illegal_function() {
    // Function 0x17 (read/write multiple registers) is not in the map.
    let out = serve(&frame(&[0x17, 0, 0, 0, 1]));
    assert_eq!(out.pdu(), &[0x97, 0x01]);
}

#[test]
fn a_frame_shorter_than_the_protocol_minimum_closes_the_connection() {
    // rmodbus requires an MBAP length of at least 6, so a two-byte PDU
    // (function 7, read exception status, which the map does not have)
    // gets no exception: the connection is dropped instead.
    let out = serve(&frame(&[7]));
    assert!(out.response.is_empty());
    assert!(out.broken);
}

#[test]
fn unit_0xff_and_0_are_this_unit_and_echoed() {
    for unit in [0xFFu8, 0] {
        let mut f = frame(&read_pdu(3, 8, 1));
        f[6] = unit;
        let out = serve(&f);
        assert!(!out.response.is_empty(), "unit {unit} gets an answer");
        assert_eq!(out.response[6], unit, "unit {unit} is echoed");
        assert_eq!(out.response[7], 3);
    }
}

#[test]
fn a_write_to_unit_0xff_is_answered() {
    let mut f = frame(&[5, 0, 100, 0xFF, 0x00]);
    f[6] = 0xFF;
    let out = serve(&f);
    assert_eq!(out.response[6], 0xFF);
    assert_eq!(&out.response[7..], &[5, 0, 100, 0xFF, 0x00]);
}

#[test]
fn a_frame_for_another_unit_is_ignored() {
    let mut f = frame(&read_pdu(3, 8, 1));
    f[6] = 9;
    let out = serve(&f);
    assert!(out.response.is_empty(), "no answer for unit 9");
    assert!(
        !out.broken,
        "a frame for another unit is not a protocol error"
    );
}

#[test]
fn a_broken_header_closes_the_connection() {
    // Protocol id 7 is not Modbus TCP.
    let out = serve(&[0x00, 0x01, 0x00, 0x07, 0x00, 0x06, UNIT, 3, 0, 8, 0, 1]);
    assert!(out.response.is_empty());
    assert!(out.broken);
    assert_eq!(
        mbap_payload_len(&[0x00, 0x01, 0x00, 0x07, 0x00, 0x06]),
        None
    );
}

#[test]
fn a_truncated_bulk_write_is_refused_without_panicking() {
    // Function 16, two registers, byte count 4, but only one byte of
    // data. rmodbus would slice past the end of the buffer.
    let out = serve(&frame(&[16, 0, 100, 0, 2, 4, 0x00]));
    assert!(out.response.is_empty());
    assert!(out.broken);
    // Same for a single write with the value missing.
    let out = serve(&frame(&[6, 0, 100]));
    assert!(out.response.is_empty());
    assert!(out.broken);
}

#[test]
fn the_mbap_length_is_sanity_checked() {
    assert_eq!(mbap_payload_len(&[0, 1, 0, 0, 0, 6]), Some(6));
    assert_eq!(mbap_payload_len(&[0, 1, 0, 0, 0, 0]), None);
    assert_eq!(mbap_payload_len(&[0, 1, 0, 0, 0xff, 0xff]), None);
    assert_eq!(mbap_payload_len(&[0, 1, 0, 0, 0]), None);
}

// -------------------------------------------------------------------
// Writes
// -------------------------------------------------------------------

#[test]
fn fc6_on_a_node_word_becomes_one_command() {
    let (rec, out) = record(&frame(&[6, 0, 102, 0, cmd_word::FORCE_OFF as u8]));
    // The echo is the request PDU.
    assert_eq!(out.pdu(), &[6, 0, 102, 0, 3]);
    assert_eq!(rec.seen.len(), 1);
    assert_eq!(rec.seen[0].action, CommandKind::ForceOff);
    assert_eq!(rec.seen[0].target, Target::Node(3));
    assert_eq!(rec.seen[0].id, "modbus-0");
    assert_eq!(out.result, Some(ResultCode::Ok));
}

#[test]
fn fc6_on_a_plane_word_reboots_and_resets_units() {
    let (rec, out) = record(&frame(&[6, 0, 200, 0, 1]));
    assert_eq!(out.pdu(), &[6, 0, 200, 0, 1]);
    assert_eq!(rec.seen[0].action, CommandKind::PlaneReboot);
    assert_eq!(rec.seen[0].target, Target::Plane(0));

    let (rec, _) = record(&frame(&[6, 0, 201, 0, 3]));
    assert_eq!(rec.seen[0].action, CommandKind::UnitReset);
    assert_eq!(rec.seen[0].target, Target::Unit { plane: 1, unit: 1 });
}

#[test]
fn fc6_with_zero_clears_the_word_without_a_command() {
    let (rec, out) = record(&frame(&[6, 0, 100, 0, 0]));
    assert_eq!(out.pdu(), &[6, 0, 100, 0, 0]);
    assert!(rec.seen.is_empty());
    assert_eq!(out.result, None);
}

#[test]
fn fc6_on_the_on_all_trigger_runs_on_all() {
    let (rec, _) = record(&frame(&[6, 0, 8, 0, 1]));
    assert_eq!(rec.seen.len(), 1);
    assert_eq!(rec.seen[0].action, CommandKind::OnAll);
    assert_eq!(rec.seen[0].target, Target::All);
}

#[test]
fn fc16_writes_every_word_in_the_span() {
    let (rec, out) = record(&frame(&[
        16,
        0,
        100,
        0,
        2,
        4,
        0,
        cmd_word::ON as u8,
        0,
        cmd_word::RESET as u8,
    ]));
    assert_eq!(out.pdu(), &[16, 0, 100, 0, 2], "echo: address and count");
    assert_eq!(rec.seen.len(), 2);
    assert_eq!(rec.seen[0].target, Target::Node(1));
    assert_eq!(rec.seen[0].action, CommandKind::On);
    assert_eq!(rec.seen[1].target, Target::Node(2));
    assert_eq!(rec.seen[1].action, CommandKind::Reset);
    assert_eq!(rec.seen[1].id, "modbus-1", "ids are unique per command");
}

#[test]
fn fc5_on_a_coil_switches_a_node() {
    let (rec, out) = record(&frame(&[5, 0, 101, 0xff, 0x00]));
    assert_eq!(out.pdu(), &[5, 0, 101, 0xff, 0x00]);
    assert_eq!(rec.seen.len(), 1);
    assert_eq!(rec.seen[0].target, Target::Node(2));
    assert_eq!(rec.seen[0].action, CommandKind::On);

    let (rec, _) = record(&frame(&[5, 0, 101, 0x00, 0x00]));
    assert_eq!(rec.seen[0].action, CommandKind::Off);
}

#[test]
fn fc15_writes_a_run_of_coils() {
    // Three coils from node 1: on, off, on.
    let (rec, out) = record(&frame(&[15, 0, 100, 0, 3, 1, 0b0000_0101]));
    assert_eq!(out.pdu(), &[15, 0, 100, 0, 3]);
    let actions: Vec<_> = rec.seen.iter().map(|c| c.action).collect();
    assert_eq!(
        actions,
        vec![CommandKind::On, CommandKind::Off, CommandKind::On]
    );
    let targets: Vec<_> = rec.seen.iter().map(|c| c.target).collect();
    assert_eq!(
        targets,
        vec![Target::Node(1), Target::Node(2), Target::Node(3)]
    );
}

// -------------------------------------------------------------------
// Write exceptions
// -------------------------------------------------------------------

#[test]
fn an_unknown_command_word_is_illegal_data_value() {
    let (rec, out) = record(&frame(&[6, 0, 100, 0, 99]));
    assert_eq!(out.pdu(), &[0x86, 0x03]);
    assert!(rec.seen.is_empty(), "nothing is commanded on an exception");
}

#[test]
fn a_write_outside_the_holding_table_is_illegal_data_address() {
    let (rec, out) = record(&frame(&[6, 0, 40, 0, 1]));
    assert_eq!(out.pdu(), &[0x86, 0x02]);
    assert!(rec.seen.is_empty());
    // A rev C client writing node 1's old word gets the same answer.
    let (rec, out) = record(&frame(&[6, 0, 0, 0, 1]));
    assert_eq!(out.pdu(), &[0x86, 0x02]);
    assert!(rec.seen.is_empty());
}

#[test]
fn a_write_to_the_result_register_is_refused() {
    let out = serve(&frame(&[6, 0, 9, 0, 1]));
    assert_eq!(out.pdu(), &[0x86, 0x03], "last_result is read only");
}

#[test]
fn one_bad_word_in_a_bulk_write_commands_nothing() {
    // Node 1 on, then an illegal word for node 2.
    let (rec, out) = record(&frame(&[16, 0, 100, 0, 2, 4, 0, cmd_word::ON as u8, 0, 42]));
    assert_eq!(out.pdu(), &[0x90, 0x03]);
    assert!(
        rec.seen.is_empty(),
        "the whole span is validated before anything runs"
    );
}

#[test]
fn a_bulk_write_past_the_coil_table_is_illegal_data_address() {
    // Four coils from node 62: node 65 does not exist.
    let (rec, out) = record(&frame(&[15, 0, 161, 0, 4, 1, 0b0000_1111]));
    assert_eq!(out.pdu(), &[0x8f, 0x02]);
    assert!(rec.seen.is_empty());
}

#[test]
fn an_illegal_coil_value_is_illegal_data_value() {
    // Function 5 only knows 0x0000 and 0xff00.
    let out = serve(&frame(&[5, 0, 100, 0x12, 0x34]));
    assert_eq!(out.pdu(), &[0x85, 0x03]);
}

// -------------------------------------------------------------------
// The result register
// -------------------------------------------------------------------

#[test]
fn the_result_register_reflects_the_ack() {
    let o = snapshot();
    for (reply, code) in [
        (Reply::ok("x", "ok"), ResultCode::Ok),
        (Reply::accepted("x"), ResultCode::Accepted),
        (Reply::err("x", "node 3 is not present"), ResultCode::Absent),
        (
            Reply::err("x", "press needs a single node target"),
            ResultCode::Refused,
        ),
        (
            Reply::err("x", "failed: sequence failed"),
            ResultCode::Failed,
        ),
    ] {
        let mut rec = Recorder {
            seen: Vec::new(),
            reply: Some(reply),
        };
        let out = rec.run(&frame(&[6, 0, 100, 0, cmd_word::ON as u8]), &image(&o));
        assert_eq!(out.result, Some(code));
    }
}

#[test]
fn the_image_carries_the_result_of_the_last_command() {
    let o = snapshot();
    let img = RegisterImage::new(&o, FwVersion::default(), 0, ResultCode::Accepted);
    let out = Recorder::default().run(&frame(&read_pdu(3, 9, 1)), &img);
    assert_eq!(out.pdu(), &[3, 2, 0, ResultCode::Accepted.as_u16() as u8]);
}

// -------------------------------------------------------------------
// The allow-list
// -------------------------------------------------------------------

#[test]
fn a_bare_address_is_a_host_route() {
    let c = Cidr::parse("192.168.1.10").expect("parses");
    assert_eq!(c.prefix, 32);
    assert!(c.contains([192, 168, 1, 10]));
    assert!(!c.contains([192, 168, 1, 11]));
}

#[test]
fn a_cidr_matches_its_whole_network() {
    let c = Cidr::parse("10.1.0.0/16").expect("parses");
    assert!(c.contains([10, 1, 0, 1]));
    assert!(c.contains([10, 1, 255, 254]));
    assert!(!c.contains([10, 2, 0, 1]));

    let any = Cidr::parse("0.0.0.0/0").expect("parses");
    assert!(any.contains([8, 8, 8, 8]), "/0 is every address");
}

#[test]
fn host_bits_outside_the_prefix_are_ignored() {
    let c = Cidr::parse("10.1.2.3/24").expect("parses");
    assert!(c.contains([10, 1, 2, 200]));
}

#[test]
fn nonsense_entries_do_not_parse() {
    for bad in [
        "",
        "10.1.2",
        "10.1.2.3.4",
        "300.1.2.3",
        "10.1.2.3/33",
        "10.1.2.3/x",
        "host.example",
        "::1",
    ] {
        assert!(Cidr::parse(bad).is_none(), "{bad} should not parse");
    }
}

#[test]
fn the_allow_list_keeps_what_it_could_not_parse() {
    let list = AllowList::parse(&["10.0.0.0/8", "nonsense", " 192.168.5.7 ", ""]);
    assert_eq!(list.len(), 2);
    assert_eq!(list.rejected, vec!["nonsense".to_string()]);
    assert!(list.allows([10, 9, 8, 7]));
    assert!(list.allows([192, 168, 5, 7]));
    assert!(!list.allows([192, 168, 5, 8]));
}

#[test]
fn an_empty_allow_list_allows_nothing() {
    let list = AllowList::parse::<String>(&[]);
    assert!(list.is_empty());
    assert!(!list.allows([127, 0, 0, 1]));
}
