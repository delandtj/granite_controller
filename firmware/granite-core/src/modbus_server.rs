//! Modbus server logic over byte slices: the part of ADR 0001 component 9
//! that does not need a socket.
//!
//! The register map itself is [`crate::modbus_map`]; this module is the
//! frame handler built on it. Everything here is `no_std` + `alloc` and
//! takes a request as a slice, so the ESP-IDF server
//! (`granite-fw/src/modbus.rs`), the host simulator and the tests all run
//! the same code:
//!
//! - [`RegisterImage`] is the read side: the four tables rendered from one
//!   [`Observed`] snapshot. Cheap enough to rebuild per request.
//! - [`handle_frame`] parses one Modbus TCP (MBAP) request with `rmodbus`,
//!   serves reads from the image, turns writes into [`Command`]s through
//!   the map and returns the response bytes.
//! - [`AllowList`] is the IPv4/CIDR guard the protocol itself cannot
//!   provide. Modbus has no authentication, so a peer that is not on the
//!   list never gets a frame read.
//!
//! What is deliberately not here: sockets, threads, timeouts, logging.
//!
//! ### Exceptions
//!
//! Everything outside the map is an exception, never a guess:
//!
//! | Case | Exception |
//! |---|---|
//! | unsupported function code | 1, illegal function (by `rmodbus`) |
//! | any address in the requested span is not in the map | 2, illegal data address |
//! | quantity 0, or a command word / coil value the map rejects | 3, illegal data value |
//! | a malformed or truncated frame | no response; the caller closes the connection |
//!
//! A write whose command is refused or fails is still a successful write
//! at the protocol level: the outcome lands in holding register 9
//! (`last_result`), because Modbus has no room for an error string.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use rmodbus::consts::ModbusFunction;
use rmodbus::server::{ModbusFrame, Read, ReadBits, ReadWords, Write, WriteBits, WriteWords};
use rmodbus::{ErrorKind, ModbusProto};

use crate::modbus_map::{
    COIL_COUNT, DISCRETE_COUNT, FwVersion, HOLDING_COUNT, INPUT_COUNT, MapError, ResultCode,
    check_coil, check_holding, coils, command_from_coil, command_from_holding, discrete_inputs,
    holding_registers, input_registers,
};
use crate::msg::{Command, Reply};
use crate::observed::Observed;

/// Bytes of the MBAP header that precede every Modbus TCP PDU.
pub const MBAP_HEADER: usize = 6;

/// Largest frame the protocol allows, header included.
pub const MAX_FRAME: usize = 256;

/// Default TCP port of a Modbus server.
pub const DEFAULT_PORT: u16 = 502;

/// Length of the PDU a well-formed MBAP header announces.
///
/// `header` is the first [`MBAP_HEADER`] bytes of a frame: transaction id,
/// protocol id (must be 0) and the byte count of what follows. `None`
/// means the header is not Modbus TCP and the connection is unusable.
pub fn mbap_payload_len(header: &[u8]) -> Option<usize> {
    if header.len() < MBAP_HEADER {
        return None;
    }
    if u16::from_be_bytes([header[2], header[3]]) != 0 {
        return None;
    }
    let len = u16::from_be_bytes([header[4], header[5]]) as usize;
    // rmodbus accepts 6..=250; anything else cannot be a request this
    // server could answer.
    if (2..=MAX_FRAME - MBAP_HEADER).contains(&len) {
        Some(len)
    } else {
        None
    }
}

// ---------------------------------------------------------------------
// The read side
// ---------------------------------------------------------------------

/// The four Modbus tables as one snapshot.
///
/// Built from an [`Observed`] through [`crate::modbus_map`], so the map is
/// still the single source of the layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisterImage {
    /// Function 2.
    pub discretes: [bool; DISCRETE_COUNT],
    /// Function 4.
    pub inputs: [u16; INPUT_COUNT],
    /// Function 3.
    pub holdings: [u16; HOLDING_COUNT],
    /// Function 1.
    pub coils: [bool; COIL_COUNT],
}

impl RegisterImage {
    /// Render the tables from a snapshot.
    ///
    /// `faults` is the bitmap of [`crate::modbus_map::fault_bits`] and
    /// `last_result` is what the last command word produced.
    pub fn build(o: &Observed, fw: FwVersion, faults: u16, last_result: ResultCode) -> Self {
        RegisterImage {
            discretes: discrete_inputs(o),
            inputs: input_registers(o, fw, faults),
            holdings: holding_registers(o, last_result),
            coils: coils(o),
        }
    }

    /// An all-zero image: what a reader sees before the first snapshot.
    pub fn empty() -> Self {
        RegisterImage {
            discretes: [false; DISCRETE_COUNT],
            inputs: [0; INPUT_COUNT],
            holdings: [0; HOLDING_COUNT],
            coils: [false; COIL_COUNT],
        }
    }
}

impl Default for RegisterImage {
    fn default() -> Self {
        Self::empty()
    }
}

// ---------------------------------------------------------------------
// One frame
// ---------------------------------------------------------------------

/// What one request produced.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FrameOutcome {
    /// Bytes to write back. Empty means "say nothing": a broadcast, a
    /// frame for another unit id, or a frame too broken to answer.
    pub response: Vec<u8>,
    /// `true` when the frame was so malformed that the stream cannot be
    /// trusted any more and the caller should close the connection.
    pub broken: bool,
    /// Set when a write produced a command: the code holding register 9
    /// reports from now on.
    pub result: Option<ResultCode>,
}

/// Handle one Modbus TCP request.
///
/// `request` is one whole frame: the MBAP header plus the PDU, exactly
/// [`MBAP_HEADER`] + [`mbap_payload_len`] bytes. Reads are served from
/// `image`. Writes become [`Command`]s (ids `modbus-<seq>`, `seq` is
/// bumped per command) and are handed to `submit`, which is expected to
/// run the command through the one dispatcher and return its ack, the
/// same path the HTTP and MQTT transports take.
///
/// A multi-register write is validated in full before any command runs,
/// so a function 16 that touches one bad address changes nothing.
pub fn handle_frame(
    unit_id: u8,
    request: &[u8],
    image: &RegisterImage,
    seq: &mut u32,
    submit: &mut dyn FnMut(&Command) -> Reply,
) -> FrameOutcome {
    let mut response: Vec<u8> = Vec::new();
    let mut result = None;
    let mut broken = false;
    let mut answer = false;

    {
        let mut frame = ModbusFrame::new(unit_id, request, ModbusProto::TcpUdp, &mut response);
        if frame.parse().is_err() {
            return FrameOutcome {
                response: Vec::new(),
                broken: true,
                result: None,
            };
        }
        if frame.processing_required {
            // rmodbus trusts the declared byte count of a bulk write and
            // of a single write; a truncated frame would slice past the
            // end of the buffer. Check before handing it over.
            if !write_span_present(&frame, request) {
                return FrameOutcome {
                    response: Vec::new(),
                    broken: true,
                    result: None,
                };
            }
            let outcome = if frame.readonly {
                serve_read(&mut frame, image)
            } else {
                serve_write(&mut frame, seq, submit, &mut result)
            };
            if outcome.is_err() {
                // Only a response buffer that cannot grow lands here, and
                // the response buffer is a `Vec`.
                broken = true;
            }
        }
        if !broken && frame.response_required {
            answer = frame.finalize_response().is_ok();
            broken = !answer;
        }
    }

    FrameOutcome {
        response: if answer { response } else { Vec::new() },
        broken,
        result,
    }
}

/// True when every byte a write function is about to read is in `request`.
fn write_span_present<V: rmodbus::VectorTrait<u8>>(
    frame: &ModbusFrame<'_, V>,
    request: &[u8],
) -> bool {
    let start = frame.frame_start;
    match frame.func {
        ModbusFunction::SetCoil | ModbusFunction::SetHolding => request.len() >= start + 6,
        ModbusFunction::SetCoilsBulk | ModbusFunction::SetHoldingsBulk => {
            if request.len() < start + 7 {
                return false;
            }
            let bytes = request[start + 6] as usize;
            request.len() >= start + 7 + bytes
        }
        _ => true,
    }
}

// ---------------------------------------------------------------------
// Reads
// ---------------------------------------------------------------------

fn serve_read<V: rmodbus::VectorTrait<u8>>(
    frame: &mut ModbusFrame<'_, V>,
    image: &RegisterImage,
) -> Result<(), ErrorKind> {
    let func = frame.func;
    let outcome = match frame.get_external_read() {
        Ok(Read::Bits(ReadBits {
            address,
            count,
            buf,
        })) => {
            let table: &[bool] = match func {
                ModbusFunction::GetCoils => &image.coils,
                _ => &image.discretes,
            };
            fill_bits(table, address, count, buf)
        }
        Ok(Read::Words(ReadWords {
            address,
            count,
            buf,
        })) => {
            let table: &[u16] = match func {
                ModbusFunction::GetHoldings => &image.holdings,
                _ => &image.inputs,
            };
            fill_words(table, address, count, buf)
        }
        Err(e) => Err(e),
    };
    frame.process_external_read(outcome)
}

/// Pack `count` bits starting at `address` into `buf`, LSB of byte 0
/// first. `buf` arrives zeroed and is exactly as long as it needs to be.
fn fill_bits(table: &[bool], address: u16, count: u16, buf: &mut [u8]) -> Result<(), ErrorKind> {
    if count == 0 {
        return Err(ErrorKind::IllegalDataValue);
    }
    let end = address as usize + count as usize;
    if end > table.len() {
        return Err(ErrorKind::IllegalDataAddress);
    }
    for i in 0..count as usize {
        if table[address as usize + i] {
            buf[i / 8] |= 1 << (i % 8);
        }
    }
    Ok(())
}

/// Write `count` big-endian words starting at `address` into `buf`.
fn fill_words(table: &[u16], address: u16, count: u16, buf: &mut [u8]) -> Result<(), ErrorKind> {
    if count == 0 {
        return Err(ErrorKind::IllegalDataValue);
    }
    let end = address as usize + count as usize;
    if end > table.len() {
        return Err(ErrorKind::IllegalDataAddress);
    }
    for i in 0..count as usize {
        let word = table[address as usize + i].to_be_bytes();
        buf[i * 2] = word[0];
        buf[i * 2 + 1] = word[1];
    }
    Ok(())
}

// ---------------------------------------------------------------------
// Writes
// ---------------------------------------------------------------------

fn serve_write<V: rmodbus::VectorTrait<u8>>(
    frame: &mut ModbusFrame<'_, V>,
    seq: &mut u32,
    submit: &mut dyn FnMut(&Command) -> Reply,
    result: &mut Option<ResultCode>,
) -> Result<(), ErrorKind> {
    let outcome = match frame.get_external_write() {
        Ok(Write::Words(w)) => write_holdings(&w, seq, submit, result),
        Ok(Write::Bits(w)) => write_coils(&w, seq, submit, result),
        Err(e) => Err(e),
    };
    frame.process_external_write(outcome)
}

/// Functions 6 and 16: command words.
fn write_holdings(
    w: &WriteWords<'_>,
    seq: &mut u32,
    submit: &mut dyn FnMut(&Command) -> Reply,
    result: &mut Option<ResultCode>,
) -> Result<(), ErrorKind> {
    let words = plan_words(w)?;
    // Validate the whole span first: a function 16 that touches one
    // register the map does not have changes nothing.
    for (addr, value) in &words {
        check_holding(*addr, *value).map_err(exception)?;
    }
    for (addr, value) in &words {
        let id = next_id(seq);
        match command_from_holding(*addr, *value, &id) {
            Ok(Some(cmd)) => *result = Some(ResultCode::of_reply(&submit(&cmd))),
            Ok(None) => {}
            Err(e) => return Err(exception(e)),
        }
    }
    Ok(())
}

/// Functions 5 and 15: node power coils.
fn write_coils(
    w: &WriteBits<'_>,
    seq: &mut u32,
    submit: &mut dyn FnMut(&Command) -> Reply,
    result: &mut Option<ResultCode>,
) -> Result<(), ErrorKind> {
    let bits = plan_bits(w)?;
    for (addr, _) in &bits {
        check_coil(*addr).map_err(exception)?;
    }
    for (addr, on) in &bits {
        let id = next_id(seq);
        match command_from_coil(*addr, *on, &id) {
            Ok(Some(cmd)) => *result = Some(ResultCode::of_reply(&submit(&cmd))),
            Ok(None) => {}
            Err(e) => return Err(exception(e)),
        }
    }
    Ok(())
}

/// The (address, value) pairs a word write asks for.
fn plan_words(w: &WriteWords<'_>) -> Result<Vec<(u16, u16)>, ErrorKind> {
    if w.count == 0 || w.data.len() < w.count as usize * 2 {
        return Err(ErrorKind::IllegalDataValue);
    }
    let mut out = Vec::with_capacity(w.count as usize);
    for i in 0..w.count as usize {
        let addr = w
            .address
            .checked_add(u16::try_from(i).map_err(|_| ErrorKind::IllegalDataValue)?)
            .ok_or(ErrorKind::IllegalDataAddress)?;
        out.push((addr, u16::from_be_bytes([w.data[i * 2], w.data[i * 2 + 1]])));
    }
    Ok(out)
}

/// The (address, value) pairs a bit write asks for.
fn plan_bits(w: &WriteBits<'_>) -> Result<Vec<(u16, bool)>, ErrorKind> {
    if w.count == 0 || w.data.len() * 8 < w.count as usize {
        return Err(ErrorKind::IllegalDataValue);
    }
    let mut out = Vec::with_capacity(w.count as usize);
    for i in 0..w.count as usize {
        let addr = w
            .address
            .checked_add(u16::try_from(i).map_err(|_| ErrorKind::IllegalDataValue)?)
            .ok_or(ErrorKind::IllegalDataAddress)?;
        out.push((addr, w.data[i / 8] & (1 << (i % 8)) != 0));
    }
    Ok(out)
}

fn next_id(seq: &mut u32) -> String {
    let id = format!("modbus-{seq}");
    *seq = seq.wrapping_add(1);
    id
}

/// Map a map error onto the Modbus exception the ADR calls for.
const fn exception(e: MapError) -> ErrorKind {
    match e {
        MapError::UnknownRegister { .. } => ErrorKind::IllegalDataAddress,
        MapError::BadValue { .. } => ErrorKind::IllegalDataValue,
    }
}

// ---------------------------------------------------------------------
// The allow-list
// ---------------------------------------------------------------------

/// One entry of the allow-list: an IPv4 network and its prefix length.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cidr {
    /// Network address, host bits already cleared.
    pub network: u32,
    /// Prefix length, 0..=32.
    pub prefix: u8,
}

impl Cidr {
    /// Parse `a.b.c.d` (implicitly /32) or `a.b.c.d/n`.
    ///
    /// Deliberately independent of `std::net`: this has to compile in the
    /// `no_std` core and the parse is four octets and a number.
    pub fn parse(s: &str) -> Option<Cidr> {
        let s = s.trim();
        let (addr, prefix) = match s.split_once('/') {
            Some((a, p)) => (a, p.trim().parse::<u8>().ok()?),
            None => (s, 32u8),
        };
        if prefix > 32 {
            return None;
        }
        let mut octets = [0u8; 4];
        let mut seen = 0usize;
        for part in addr.trim().split('.') {
            if seen == 4 || part.is_empty() || part.len() > 3 {
                return None;
            }
            octets[seen] = part.parse::<u8>().ok()?;
            seen += 1;
        }
        if seen != 4 {
            return None;
        }
        let bits = u32::from_be_bytes(octets);
        let mask = mask_of(prefix);
        Some(Cidr {
            network: bits & mask,
            prefix,
        })
    }

    /// True when `ip` falls inside this network.
    pub fn contains(&self, ip: [u8; 4]) -> bool {
        u32::from_be_bytes(ip) & mask_of(self.prefix) == self.network
    }
}

const fn mask_of(prefix: u8) -> u32 {
    if prefix == 0 {
        0
    } else {
        u32::MAX << (32 - prefix as u32)
    }
}

/// The parsed `sec.modbus.allow` list.
///
/// An empty list allows nothing: Modbus has no authentication, so the
/// server does not start without one (ADR 0001 component 9).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AllowList {
    /// Networks a peer may come from.
    pub entries: Vec<Cidr>,
    /// Entries that did not parse, kept verbatim so the caller can log
    /// them instead of silently widening or narrowing the guard.
    pub rejected: Vec<String>,
}

impl AllowList {
    /// Parse every entry; unparsable ones land in `rejected`.
    pub fn parse<S: AsRef<str>>(entries: &[S]) -> AllowList {
        let mut out = AllowList::default();
        for e in entries {
            let text = e.as_ref().trim();
            if text.is_empty() {
                continue;
            }
            match Cidr::parse(text) {
                Some(c) => out.entries.push(c),
                None => out.rejected.push(String::from(text)),
            }
        }
        out
    }

    /// True when no peer can be allowed.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// How many networks are on the list.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when `ip` is on the list.
    pub fn allows(&self, ip: [u8; 4]) -> bool {
        self.entries.iter().any(|c| c.contains(ip))
    }
}
