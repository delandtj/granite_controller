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
//! count on the wire (SLOT 0x23) and maps it to a class (0x22) itself,
//! because the resistor table is a module question and not an ADC one.

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

/// Counts above which a slot counts as empty.
///
/// The divider is 10k to 3V3 over the module's resistor to ground, so an
/// empty slot sits at the reference and a module of up to 390k still
/// reads under this. Four counts of slack absorb the ADC's own noise.
pub const EMPTY_THRESHOLD: u16 = FULL_SCALE - 4;

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

/// The module class of an ID count.
///
/// The plane divides the range into 16 bands and reports the band; the
/// controller owns the resistor-to-model table (ADR 0003 0x22 is "the
/// plane's own code"). Class 0 is "no module", so a band is 1-16.
pub const fn module_class(counts: u16) -> u8 {
    if is_empty(counts) {
        0
    } else {
        1 + (counts >> 8) as u8
    }
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
    fn classes_are_bands_of_the_range() {
        assert_eq!(module_class(0), 1);
        assert_eq!(module_class(255), 1);
        assert_eq!(module_class(256), 2);
        assert_eq!(module_class(0x0E00), 15);
    }
}
