//! TCA6424A unit expander, address 0x22, and the line map of one unit
//! board.
//!
//! Register map and the auto-increment bit are from the TCA6424A
//! datasheet (SCPS224, "Register descriptions" and the command-byte
//! table): the command byte's bit 7 set makes the part walk to the next
//! register inside one transaction, so the three input ports are one
//! read and the three output ports one write. A configuration bit is 1
//! for an input, 0 for an output, and every register resets to 0xFF, so
//! after a reset every line is an input *and* every output latch holds
//! high.
//!
//! That reset value is why [`Expander::configure`] writes the output
//! latches before it writes the configuration registers: while the pins
//! are still inputs a latch write has no electrical effect, and once the
//! configuration flips them to outputs they drive what was already put
//! there. Writing configuration first would drive 0xFF for the time of
//! one I2C transaction, which on this board means every rail off and
//! both buttons pressed. This is ADR 0002 decision 3 step 2, "writes the
//! expander's photoMOS outputs (PWRD, RSTD) released, before any read".
//!
//! The line map is power/pp-48/tools/sch/blocks/unit.py `EXP_IO` keyed
//! by VQFN package pin, resolved through the TCA6424ARGJR symbol in
//! power/pp-48/pp.kicad_sym (pins 1-24 are P00..P27 in order). It agrees
//! with ADR 0002: slot A on P00-P07 / P10-P12, P13 U_OFF, slot B on
//! P14-P17 / P20-P26, P27 U_PGD.
//!
//! | Line | Slot A | Slot B | Direction | Active |
//! |---|---|---|---|---|
//! | DISARM | P00 | P26 | out | high asserts (Q7 pulls PGD low) |
//! | OFF | P01 | P25 | out | high asserts (Q2 pulls UVLO low) |
//! | PGD | P02 | P24 | in | high = hot-swap power good |
//! | LQ | P03 | P23 | in | high = OV latch set |
//! | LCLR | P04 | P22 | out | high clears the OV latch (Q4) |
//! | OVK | P05 | P21 | in | low = a trip source pulls OV_KILL |
//! | PG | P06 | P20 | in | high = module PGOOD (pull-up, open = good) |
//! | RUNOFF | P07 | P17 | out | high stops the module (Q5 pulls RUN low) |
//! | LED | P10 | P16 | in | **low = the host's power LED is lit** (opto) |
//! | PWRD | P11 | P14 | out | high closes the PWR_SW photoMOS |
//! | RSTD | P12 | P15 | out | high closes the RST_SW photoMOS |
//! | U_OFF | P13 | - | out | high switches the unit's hot-swap off |
//! | U_PGD | - | P27 | in | high = unit hot-swap power good |
//!
//! Every output has a 100k pull-down at its gate, so an all-zero output
//! latch is the hardware default: rails on, latches kept, modules
//! running, buttons released.

use crate::device::{DevResult, SegmentBus};

/// The expander's I2C address on a unit segment (ADDR to GND).
pub const EXP_ADDR: u8 = 0x22;

/// Input Port 0. Ports 1 and 2 follow at 0x01 and 0x02.
pub const REG_INPUT0: u8 = 0x00;
/// Output Port 0. Ports 1 and 2 follow at 0x05 and 0x06.
pub const REG_OUTPUT0: u8 = 0x04;
/// Polarity Inversion Port 0. The plane leaves these at 0 and inverts
/// the LED line in software instead.
pub const REG_POLARITY0: u8 = 0x08;
/// Configuration Port 0. Ports 1 and 2 follow at 0x0D and 0x0E.
pub const REG_CONFIG0: u8 = 0x0C;

/// Command-byte bit that makes the part auto-increment its register
/// pointer (SCPS224: "command byte", bit 7).
pub const CMD_AUTO_INCREMENT: u8 = 0x80;

/// A line on the expander, as `port * 8 + bit`.
pub type Line = u8;

/// The six output lines of one slot, in the order the plane writes them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Out {
    /// OFF: high cuts the slot's rail.
    Off,
    /// LCLR: a high pulse clears the slot's OV latch.
    Lclr,
    /// RUNOFF: high stops the module. Never without [`Out::Disarm`].
    Runoff,
    /// DISARM: high keeps the PGOOD-loss trip disarmed.
    Disarm,
    /// PWRD: high closes the front panel's PWR contact.
    Pwrd,
    /// RSTD: high closes the front panel's RST contact.
    Rstd,
}

/// The five input lines of one slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum In {
    /// PGD: the hot-swap's power-good.
    Pgd,
    /// PG: the module's PGOOD.
    Pg,
    /// LQ: the OV latch's Q.
    Lq,
    /// LED: the host's power LED, **inverted** (low = lit).
    Led,
    /// OVK: the OV_KILL node, low while a trip source pulls it.
    Ovk,
}

/// Lines of slot A (the core side), indexed as `port * 8 + bit`.
const A: [Line; 11] = [
    0x00, // P00 A_DISARM
    0x01, // P01 A_OFF
    0x02, // P02 A_PGD
    0x03, // P03 A_LQ
    0x04, // P04 A_LCLR
    0x05, // P05 A_OVK
    0x06, // P06 A_PG
    0x07, // P07 A_RUNOFF
    0x08, // P10 A_LED
    0x09, // P11 A_PWRD
    0x0A, // P12 A_RSTD
];

/// Lines of slot B (the far side); the drawing mirrors the order.
const B: [Line; 11] = [
    0x16, // P26 B_DISARM
    0x15, // P25 B_OFF
    0x14, // P24 B_PGD
    0x13, // P23 B_LQ
    0x12, // P22 B_LCLR
    0x11, // P21 B_OVK
    0x10, // P20 B_PG
    0x0F, // P17 B_RUNOFF
    0x0E, // P16 B_LED
    0x0C, // P14 B_PWRD
    0x0D, // P15 B_RSTD
];

/// P13: the unit hot-swap's off line (output).
pub const LINE_U_OFF: Line = 0x0B;
/// P27: the unit hot-swap's power-good (input).
pub const LINE_U_PGD: Line = 0x17;

/// Index into [`A`] / [`B`] of a line name.
const fn slot_index_out(line: Out) -> usize {
    match line {
        Out::Disarm => 0,
        Out::Off => 1,
        Out::Lclr => 4,
        Out::Runoff => 7,
        Out::Pwrd => 9,
        Out::Rstd => 10,
    }
}

/// Index into [`A`] / [`B`] of an input line name.
const fn slot_index_in(line: In) -> usize {
    match line {
        In::Pgd => 2,
        In::Lq => 3,
        In::Ovk => 5,
        In::Pg => 6,
        In::Led => 8,
    }
}

/// The expander line of an output on half `b` (false = A, true = B).
pub const fn out_line(b: bool, line: Out) -> Line {
    let i = slot_index_out(line);
    if b { B[i] } else { A[i] }
}

/// The expander line of an input on half `b` (false = A, true = B).
pub const fn in_line(b: bool, line: In) -> Line {
    let i = slot_index_in(line);
    if b { B[i] } else { A[i] }
}

/// Three port bytes with only the output lines set.
///
/// Port 0: A_DISARM, A_OFF, A_LCLR, A_RUNOFF. Port 1: A_LED is an input,
/// so A_PWRD, A_RSTD, U_OFF, B_PWRD, B_RSTD, B_RUNOFF. Port 2: B_LCLR,
/// B_OFF, B_DISARM.
pub const OUTPUT_MASK: [u8; 3] = output_mask();

/// Build [`OUTPUT_MASK`] from the line map, so the map is the only place
/// the wiring is written down.
const fn output_mask() -> [u8; 3] {
    let mut mask = [0u8; 3];
    let outs = [
        Out::Off,
        Out::Lclr,
        Out::Runoff,
        Out::Disarm,
        Out::Pwrd,
        Out::Rstd,
    ];
    let mut half = 0;
    while half < 2 {
        let mut i = 0;
        while i < outs.len() {
            let line = out_line(half == 1, outs[i]);
            mask[(line >> 3) as usize] |= 1 << (line & 7);
            i += 1;
        }
        half += 1;
    }
    mask[(LINE_U_OFF >> 3) as usize] |= 1 << (LINE_U_OFF & 7);
    mask
}

/// Configuration registers for [`OUTPUT_MASK`]: a 1 bit is an input.
pub const CONFIG_BYTES: [u8; 3] = [!OUTPUT_MASK[0], !OUTPUT_MASK[1], !OUTPUT_MASK[2]];

/// One unit's TCA6424A, with the output latch it believes it holds.
#[derive(Debug, Clone, Copy, Default)]
pub struct Expander {
    /// What was last written to Output Port 0-2.
    latch: [u8; 3],
    /// Input Port 0-2 as last read.
    inputs: [u8; 3],
}

impl Expander {
    /// An expander whose latch is all zero: the released state, and what
    /// [`Expander::configure`] writes first.
    pub const fn new() -> Self {
        Expander {
            latch: [0; 3],
            inputs: [0; 3],
        }
    }

    /// The output latch this driver holds.
    pub const fn latch(&self) -> [u8; 3] {
        self.latch
    }

    /// Input Port 0-2 as last read.
    pub const fn inputs(&self) -> [u8; 3] {
        self.inputs
    }

    /// True while `line` is high in the output latch.
    pub const fn get_output(&self, line: Line) -> bool {
        self.latch[(line >> 3) as usize] & (1 << (line & 7)) != 0
    }

    /// Set `line` in the output latch. Nothing reaches the bus until
    /// [`Expander::flush`].
    pub fn set_output(&mut self, line: Line, on: bool) {
        let (port, bit) = ((line >> 3) as usize, 1u8 << (line & 7));
        if on {
            self.latch[port] |= bit;
        } else {
            self.latch[port] &= !bit;
        }
    }

    /// Release every output line (the hardware default).
    pub fn release_all(&mut self) {
        self.latch = [0; 3];
    }

    /// True while `line` read high in the last input sample.
    pub const fn get_input(&self, line: Line) -> bool {
        self.inputs[(line >> 3) as usize] & (1 << (line & 7)) != 0
    }

    /// Probe the part: read Input Port 0. A NAK here is "no unit"
    /// (ADR 0002 decision 4).
    pub fn probe<B: SegmentBus>(&mut self, bus: &mut B) -> DevResult<()> {
        let mut byte = [0u8; 1];
        bus.read(EXP_ADDR, &[REG_INPUT0], &mut byte)?;
        Ok(())
    }

    /// Configure a unit that just appeared, outputs before direction.
    ///
    /// `intended` is the latch the slots' intended state asks for. The
    /// released latch goes out first even when `intended` differs, so a
    /// press can never be held by a stale latch (ADR 0002 decision 3).
    pub fn configure<B: SegmentBus>(&mut self, bus: &mut B, intended: [u8; 3]) -> DevResult<()> {
        self.latch = [0; 3];
        self.write_latch(bus)?;
        bus.write(
            EXP_ADDR,
            &[
                CMD_AUTO_INCREMENT | REG_CONFIG0,
                CONFIG_BYTES[0],
                CONFIG_BYTES[1],
                CONFIG_BYTES[2],
            ],
        )?;
        // Polarity inversion stays off: the LED line is inverted where
        // it is read, so a register dump matches the schematic.
        bus.write(
            EXP_ADDR,
            &[CMD_AUTO_INCREMENT | REG_POLARITY0, 0x00, 0x00, 0x00],
        )?;
        self.latch = [
            intended[0] & OUTPUT_MASK[0],
            intended[1] & OUTPUT_MASK[1],
            intended[2] & OUTPUT_MASK[2],
        ];
        self.write_latch(bus)
    }

    /// Write Output Port 0-2 in one transaction.
    pub fn flush<B: SegmentBus>(&mut self, bus: &mut B) -> DevResult<()> {
        self.write_latch(bus)
    }

    /// Read Input Port 0-2 in one transaction (the 20 Hz sample).
    pub fn read_inputs<B: SegmentBus>(&mut self, bus: &mut B) -> DevResult<[u8; 3]> {
        let mut v = [0u8; 3];
        bus.read(EXP_ADDR, &[CMD_AUTO_INCREMENT | REG_INPUT0], &mut v)?;
        self.inputs = v;
        Ok(v)
    }

    /// Read Configuration Port 0-2 back, to tell a reset expander from a
    /// configured one.
    pub fn read_config<B: SegmentBus>(&mut self, bus: &mut B) -> DevResult<[u8; 3]> {
        let mut v = [0u8; 3];
        bus.read(EXP_ADDR, &[CMD_AUTO_INCREMENT | REG_CONFIG0], &mut v)?;
        Ok(v)
    }

    fn write_latch<B: SegmentBus>(&mut self, bus: &mut B) -> DevResult<()> {
        bus.write(
            EXP_ADDR,
            &[
                CMD_AUTO_INCREMENT | REG_OUTPUT0,
                self.latch[0],
                self.latch[1],
                self.latch[2],
            ],
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_line_map_is_the_adr_split() {
        // Slot A on P00-P07 and P10-P12, slot B on P14-P17 and P20-P26.
        for i in 0..11 {
            assert!(A[i] <= 0x0A, "slot A line {i} is {:#04x}", A[i]);
            assert!((0x0C..=0x16).contains(&B[i]), "slot B line {i}");
        }
        assert_eq!(LINE_U_OFF, 0x0B);
        assert_eq!(LINE_U_PGD, 0x17);
        // Every one of the 24 lines is claimed exactly once.
        let mut seen = [false; 24];
        for line in A.iter().chain(B.iter()).chain([LINE_U_OFF, LINE_U_PGD].iter()) {
            assert!(!seen[*line as usize], "line {line:#04x} twice");
            seen[*line as usize] = true;
        }
        assert!(seen.iter().all(|s| *s));
    }

    #[test]
    fn masks_match_the_hand_count() {
        // Port 0: bits 0, 1, 4, 7. Port 1: 1, 2, 3, 4, 5, 7. Port 2: 2,
        // 5, 6.
        assert_eq!(OUTPUT_MASK, [0b1001_0011, 0b1011_1110, 0b0110_0100]);
        assert_eq!(CONFIG_BYTES, [0b0110_1100, 0b0100_0001, 0b1001_1011]);
        // Eleven lines per slot, six of them outputs, plus U_OFF.
        assert_eq!(
            OUTPUT_MASK.iter().map(|b| b.count_ones()).sum::<u32>(),
            6 + 6 + 1
        );
    }

    #[test]
    fn named_lines_land_where_the_table_says() {
        assert_eq!(out_line(false, Out::Off), 0x01);
        assert_eq!(out_line(true, Out::Off), 0x15);
        assert_eq!(out_line(false, Out::Pwrd), 0x09);
        assert_eq!(out_line(true, Out::Pwrd), 0x0C);
        assert_eq!(in_line(false, In::Led), 0x08);
        assert_eq!(in_line(true, In::Led), 0x0E);
        assert_eq!(in_line(true, In::Pgd), 0x14);
    }

    #[test]
    fn latch_bits_round_trip() {
        let mut e = Expander::new();
        e.set_output(out_line(true, Out::Rstd), true);
        assert_eq!(e.latch(), [0, 1 << 5, 0]);
        assert!(e.get_output(out_line(true, Out::Rstd)));
        e.release_all();
        assert_eq!(e.latch(), [0; 3]);
    }
}
