//! ADC121C027 module ID ADC: 0x51 for slot A, 0x52 for slot B.
//!
//! Register numbers and the result format are from the ADC121C027
//! datasheet (SNAS510, "Register definitions"): a 2-byte read of the
//! conversion register gives the alert flag in bit 15 and the 12-bit
//! result in bits 11-0, and the configuration register's reset value
//! leaves the part in a plain converted-on-read mode, which is all the
//! plane needs.
//!
//! VA is the part's reference (unit.py U4: "VA = reference, reads
//! ID / 3V3"), and the slot's ID divider is a 10k pull-up to 3V3 against
//! the module's resistor to ground, so a full-scale reading (0xFFF) is
//! an open slot and anything lower is a module. The plane keeps the raw
//! count on the wire (SLOT 0x23) and maps it to a class (0x22) itself:
//! the class is the entry of the ID table in docs/pm-interface.md,
//! decided at the midpoints between neighbouring entries
//! ([`module_class`]).

use crate::device::{DevResult, SegmentBus};

/// Slot A's ID ADC (ADR0 to GND).
pub const ADC_SLOT_A: u8 = 0x51;
/// Slot B's ID ADC (ADR0 to VA).
pub const ADC_SLOT_B: u8 = 0x52;

/// Conversion result: alert flag in bit 15, result in bits 11-0.
pub const REG_RESULT: u8 = 0x00;
/// Alert status.
pub const REG_ALERT_STATUS: u8 = 0x01;
/// Configuration: the plane leaves it at its reset value.
pub const REG_CONFIGURATION: u8 = 0x02;

/// Full scale of the 12-bit result.
pub const FULL_SCALE: u16 = 0x0FFF;

/// Counts at and above which a slot counts as empty.
///
/// The divider is 10k to 3V3 over the module's resistor to ground, so an
/// empty slot sits at the reference. The highest resistor in the ID
/// table (docs/pm-interface.md) is 47k, 0.82 of full scale (3378
/// counts); open is 1.00. The threshold sits at 0.92 (3768), between
/// the two, so an empty slot whose reading lands a little under full
/// scale (the ADC's gain and offset error, VA and the pull-up not being
/// exactly the same rail, leakage on the ID net) is still empty.
///
/// It was full scale minus 4 counts until 2026-11-25: the first unit-fd
/// on the bench reported class 16 (0xF00 to 0xFFA) on an empty slot A
/// and nothing on slot B.
pub const EMPTY_THRESHOLD: u16 = 3768;

/// One slot's ID ADC.
#[derive(Debug, Clone, Copy)]
pub struct IdAdc {
    addr: u8,
}

impl IdAdc {
    /// The ADC at `addr`.
    pub const fn new(addr: u8) -> Self {
        IdAdc { addr }
    }

    /// The ADC of half `b` (false = A, true = B).
    pub const fn slot(b: bool) -> Self {
        IdAdc::new(if b { ADC_SLOT_B } else { ADC_SLOT_A })
    }

    /// The part's I2C address.
    pub const fn addr(&self) -> u8 {
        self.addr
    }

    /// Read the 12-bit ID count. A NAK is a unit fault, not an empty
    /// slot (ADR 0002 decision 4).
    pub fn read_id<B: SegmentBus>(&self, bus: &mut B) -> DevResult<u16> {
        let mut v = [0u8; 2];
        bus.read(self.addr, &[REG_RESULT], &mut v)?;
        Ok(u16::from_be_bytes(v) & FULL_SCALE)
    }

    /// Read the configuration register, the presence probe: it answers
    /// even with nothing on the analogue input.
    pub fn read_configuration<B: SegmentBus>(&self, bus: &mut B) -> DevResult<u8> {
        let mut v = [0u8; 1];
        bus.read(self.addr, &[REG_CONFIGURATION], &mut v)?;
        Ok(v[0])
    }
}

/// True when an ID count means "no module in the slot".
pub const fn is_empty(counts: u16) -> bool {
    counts >= EMPTY_THRESHOLD
}

/// The host-side pull-up of the ID divider \[ohm\] (unit.py R15 / R65,
/// docs/pm-interface.md: "host 10k pull-up to 3V3").
pub const ID_PULLUP_OHMS: u32 = 10_000;

/// The module ID resistors of docs/pm-interface.md \[ohm\], in table
/// order.
///
/// The class of a reading is this table's index plus one, so the class
/// *is* the table entry: 1 the short (fault), 2 the 2.2k, 3 the 4.7k,
/// 4 the 10k (PM-19V-200W rev 0.1), 5 the 22k (PM-19V-250W), 6 the 47k.
/// Class 0 is "no module", which is `open` at the bottom of the table.
/// Adding a module means adding its resistor here, in order.
pub const ID_RESISTORS_OHMS: [u32; 6] = [0, 2_200, 4_700, 10_000, 22_000, 47_000];

/// Class of a slot with no module in it (ADR 0003 0x22).
pub const CLASS_NO_MODULE: u8 = 0;

/// Class of a shorted ID pin: a module fault, never a model
/// (docs/pm-interface.md: "0 (short) | 0 | fault").
pub const CLASS_SHORT: u8 = 1;

/// Class of the PM-19V-250W, the 22k entry.
pub const CLASS_PM_19V_250W: u8 = 5;

/// The count a nominal table entry reads: `4095 * R / (R + 10k)`,
/// because VA is the ADC's own reference and the divider hangs on the
/// same 3V3 (unit.py U4: "VA = reference, reads ID / 3V3").
pub const fn table_counts(r_ohms: u32) -> u16 {
    ((FULL_SCALE as u32 * r_ohms) / (r_ohms + ID_PULLUP_OHMS)) as u16
}

/// The decision boundary between two neighbouring table entries
/// \[counts\]: their midpoint, rounded up, so a reading at the boundary
/// belongs to the higher entry and no count belongs to two classes.
pub const fn class_boundary(i: usize) -> u16 {
    let lo = table_counts(ID_RESISTORS_OHMS[i]) as u32;
    let hi = table_counts(ID_RESISTORS_OHMS[i + 1]) as u32;
    (lo + hi).div_ceil(2) as u16
}

/// The module class of an ID count.
///
/// The class is the entry of the ID table in docs/pm-interface.md the
/// reading is nearest to, with the boundaries at the midpoints in counts
/// between neighbouring entries ([`class_boundary`]). The widest table
/// gap is 1 % resistors' worth of error several times over, so a module
/// never reads as its neighbour; the plane still reports the raw count
/// as well (SLOT 0x23), so a reading that drifts can be seen.
///
/// Until 2026-11-25 this divided the range into 16 equal bands and
/// reported the band. The 22k entry (2815 counts) sat exactly on the
/// 11 / 12 edge, so two identical PM-19V-250W modules could report
/// different classes.
pub const fn module_class(counts: u16) -> u8 {
    if is_empty(counts) {
        return CLASS_NO_MODULE;
    }
    let mut i = 0;
    while i + 1 < ID_RESISTORS_OHMS.len() {
        if counts < class_boundary(i) {
            return (i + 1) as u8;
        }
        i += 1;
    }
    ID_RESISTORS_OHMS.len() as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_open_slot_is_class_zero() {
        assert!(is_empty(FULL_SCALE));
        assert!(is_empty(EMPTY_THRESHOLD));
        assert!(!is_empty(EMPTY_THRESHOLD - 1));
        assert_eq!(module_class(FULL_SCALE), 0);
    }

    #[test]
    fn an_empty_slot_a_little_under_full_scale_is_empty() {
        assert!(is_empty(0x0FF0));
        assert!(is_empty(0x0F00));
        assert_eq!(module_class(0x0FF0), 0);
    }

    #[test]
    fn every_table_resistor_reads_as_a_module() {
        // counts = 4095 * R / (R + 10k), 1 % resistors on the high side
        for r_k in [2.2f32, 4.7, 10.0, 22.0, 47.0] {
            let r = r_k * 1.01;
            let counts = (4095.0 * r / (r + 10.0 * 0.99)) as u16;
            assert!(!is_empty(counts), "{r_k}k reads {counts}");
        }
    }

    /// The count a resistor reads with the two tolerances pushed the
    /// same way: `hi` takes the module resistor high and the pull-up
    /// low, which is the largest reading, and the other way round for
    /// the smallest. Integer throughout, like the firmware itself.
    fn corner_counts(r_ohms: u32, hi: bool) -> u16 {
        let (r, pu) = if hi {
            (r_ohms * 101 / 100, ID_PULLUP_OHMS * 99 / 100)
        } else {
            (r_ohms * 99 / 100, ID_PULLUP_OHMS * 101 / 100)
        };
        ((FULL_SCALE as u64 * r as u64) / (r as u64 + pu as u64)) as u16
    }

    #[test]
    fn the_class_is_the_table_entry() {
        // docs/pm-interface.md, in order: 0 (short), 2.2k, 4.7k, 10k,
        // 22k, 47k, open.
        assert_eq!(module_class(0), CLASS_SHORT);
        assert_eq!(module_class(table_counts(2_200)), 2);
        assert_eq!(module_class(table_counts(4_700)), 3);
        assert_eq!(module_class(table_counts(10_000)), 4);
        assert_eq!(module_class(table_counts(22_000)), CLASS_PM_19V_250W);
        assert_eq!(module_class(table_counts(47_000)), 6);
        assert_eq!(module_class(FULL_SCALE), CLASS_NO_MODULE);
        // The nominal counts, so a change in the arithmetic is visible.
        assert_eq!(
            [
                table_counts(0),
                table_counts(2_200),
                table_counts(4_700),
                table_counts(10_000),
                table_counts(22_000),
                table_counts(47_000),
            ],
            [0, 738, 1309, 2047, 2815, 3376]
        );
    }

    #[test]
    fn every_table_resistor_at_one_percent_keeps_its_own_class() {
        for (i, r) in ID_RESISTORS_OHMS.iter().enumerate() {
            let class = (i + 1) as u8;
            for hi in [false, true] {
                let counts = corner_counts(*r, hi);
                assert_eq!(
                    module_class(counts),
                    class,
                    "{r} ohm at the {} corner reads {counts}",
                    if hi { "high" } else { "low" }
                );
            }
            // And the nominal itself.
            assert_eq!(module_class(table_counts(*r)), class, "{r} ohm nominal");
        }
    }

    #[test]
    fn no_count_falls_between_two_classes() {
        // Every boundary is the first count of the higher class, and
        // the boundaries rise, so the classes tile the whole range.
        let mut previous = 0u16;
        for i in 0..ID_RESISTORS_OHMS.len() - 1 {
            let boundary = class_boundary(i);
            assert!(boundary > previous, "boundary {i} is {boundary}");
            assert_eq!(module_class(boundary - 1), (i + 1) as u8);
            assert_eq!(module_class(boundary), (i + 2) as u8);
            previous = boundary;
        }
        assert!(
            class_boundary(ID_RESISTORS_OHMS.len() - 2) < EMPTY_THRESHOLD,
            "the top class has room under the empty threshold"
        );
        // The 22k entry used to sit exactly on a band edge, which is
        // what this numbering is for.
        assert_eq!(module_class(2_815), CLASS_PM_19V_250W);
        assert_eq!(module_class(2_816), CLASS_PM_19V_250W);
    }

    #[test]
    fn a_boundary_has_room_for_the_adcs_own_error() {
        // The tightest corner is the one nearest a boundary; keep a
        // couple of hundred counts there, which is far more than the
        // ADC121C027's unadjusted error.
        for (i, r) in ID_RESISTORS_OHMS.iter().enumerate() {
            let hi = corner_counts(*r, true);
            let lo = corner_counts(*r, false);
            if i + 1 < ID_RESISTORS_OHMS.len() {
                assert!(
                    class_boundary(i) - hi > 200,
                    "{r} ohm high corner {hi} is close to {}",
                    class_boundary(i)
                );
            }
            if i > 0 {
                assert!(
                    lo - class_boundary(i - 1) > 200,
                    "{r} ohm low corner {lo} is close to {}",
                    class_boundary(i - 1)
                );
            }
        }
    }
}
