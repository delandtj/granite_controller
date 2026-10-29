//! The simulated segment bus: a TCA9548A in front of up to four
//! [`FakeUnit`]s, routing every transfer the drivers make.
//!
//! Address decoding is the board's: 0x70 is the mux, and behind the
//! selected channel 0x22 is the expander, 0x40 / 0x41 the slot monitors,
//! 0x44 the unit input monitor and 0x51 / 0x52 the ID ADCs. Anything
//! else, an unselected channel or a unit without 3V3 answers with a NAK,
//! which is exactly what discovery reads as absence.

use crate::device::{DevResult, DeviceError, SegmentBus};
use crate::drivers::{expander, idadc, ina238, mux};
use crate::sim::unit::FakeUnit;

/// The simulated bus.
#[derive(Debug, Default)]
pub struct FakeBus {
    /// The mux's control register, bit per channel.
    pub mux_control: u8,
    /// The units, one per channel; `None` is an empty slot in the chain.
    pub units: [Option<FakeUnit>; granite_proto::SEGMENT_CHANNELS],
    /// SDA is held low: every transfer gives [`DeviceError::Stuck`]
    /// until it is cleared, which [`SegmentBus::unstick`] does.
    pub stuck: bool,
    /// How often MUX_RST was pulsed.
    pub mux_resets: u32,
    /// How often nine clocks were asked for.
    pub unsticks: u32,
    /// Transfers that answered, for a test that wants to count traffic.
    pub transfers: u32,
}

impl FakeBus {
    /// An empty chain.
    pub fn new() -> Self {
        FakeBus::default()
    }

    /// A chain with `n` populated units on channels 0..n.
    pub fn with_units(n: usize) -> Self {
        let mut bus = FakeBus::new();
        for ch in 0..n {
            bus.units[ch] = Some(FakeUnit::populated());
        }
        bus
    }

    /// The channel the mux has selected, if exactly one is on.
    pub fn selected(&self) -> Option<usize> {
        if self.mux_control.count_ones() != 1 {
            return None;
        }
        Some(self.mux_control.trailing_zeros() as usize)
    }

    /// The unit on the selected channel, if it has 3V3.
    fn live_unit(&mut self) -> Option<&mut FakeUnit> {
        let ch = self.selected()?;
        self.units
            .get_mut(ch)?
            .as_mut()
            .filter(|unit| unit.powered)
    }

    /// Advance every unit's hardware by `dt`, with the core's press
    /// deadline as `press_en`.
    pub fn advance(&mut self, dt: u32, press_en: bool) {
        for unit in self.units.iter_mut().flatten() {
            unit.advance(dt, press_en);
        }
    }

    /// One unit, for a test that sets up or inspects the hardware.
    pub fn unit(&mut self, ch: usize) -> &mut FakeUnit {
        self.units[ch]
            .as_mut()
            .expect("no unit on that channel")
    }
}

impl SegmentBus for FakeBus {
    fn write(&mut self, addr: u8, bytes: &[u8]) -> DevResult<()> {
        if self.stuck {
            return Err(DeviceError::Stuck);
        }
        self.transfers += 1;
        if addr == mux::MUX_ADDR {
            // The written byte is the control register.
            let [control] = bytes else {
                return Err(DeviceError::Bus);
            };
            self.mux_control = *control;
            return Ok(());
        }
        let Some(unit) = self.live_unit() else {
            return Err(DeviceError::Nak);
        };
        match (addr, bytes) {
            (expander::EXP_ADDR, [cmd, rest @ ..]) => {
                let auto = cmd & expander::CMD_AUTO_INCREMENT != 0;
                let base = cmd & !expander::CMD_AUTO_INCREMENT;
                if rest.is_empty() {
                    // A pointer-only write: nothing to do, the fake has
                    // no register pointer of its own.
                    return Ok(());
                }
                if !auto && rest.len() > 1 {
                    return Err(DeviceError::Bus);
                }
                for (i, byte) in rest.iter().enumerate() {
                    let reg = base + i as u8;
                    let port = match reg {
                        r if (expander::REG_OUTPUT0..expander::REG_OUTPUT0 + 3).contains(&r) => {
                            &mut unit.expander.output[(r - expander::REG_OUTPUT0) as usize]
                        }
                        r if (expander::REG_POLARITY0..expander::REG_POLARITY0 + 3).contains(&r) => {
                            &mut unit.expander.polarity[(r - expander::REG_POLARITY0) as usize]
                        }
                        r if (expander::REG_CONFIG0..expander::REG_CONFIG0 + 3).contains(&r) => {
                            &mut unit.expander.config[(r - expander::REG_CONFIG0) as usize]
                        }
                        // The input ports are read only.
                        _ => return Err(DeviceError::Bus),
                    };
                    *port = *byte;
                }
                Ok(())
            }
            (ina238::INA_SLOT_A | ina238::INA_SLOT_B | ina238::INA_UNIT, [reg, hi, lo]) => {
                let part = match addr {
                    ina238::INA_SLOT_A => &mut unit.slots[0].ina,
                    ina238::INA_SLOT_B => &mut unit.slots[1].ina,
                    _ => &mut unit.input,
                };
                if !part.present {
                    return Err(DeviceError::Nak);
                }
                if part.write(*reg, u16::from_be_bytes([*hi, *lo])) {
                    Ok(())
                } else {
                    Err(DeviceError::Bus)
                }
            }
            (idadc::ADC_SLOT_A | idadc::ADC_SLOT_B, _) => {
                let part = if addr == idadc::ADC_SLOT_A {
                    unit.slots[0].adc
                } else {
                    unit.slots[1].adc
                };
                if part.present {
                    Ok(())
                } else {
                    Err(DeviceError::Nak)
                }
            }
            _ => Err(DeviceError::Nak),
        }
    }

    fn read(&mut self, addr: u8, cmd: &[u8], out: &mut [u8]) -> DevResult<()> {
        if self.stuck {
            return Err(DeviceError::Stuck);
        }
        self.transfers += 1;
        if addr == mux::MUX_ADDR {
            if out.len() != 1 {
                return Err(DeviceError::Bus);
            }
            out[0] = self.mux_control;
            return Ok(());
        }
        let Some(unit) = self.live_unit() else {
            return Err(DeviceError::Nak);
        };
        match addr {
            expander::EXP_ADDR => {
                let [cmd] = cmd else {
                    return Err(DeviceError::Bus);
                };
                let auto = cmd & expander::CMD_AUTO_INCREMENT != 0;
                let base = cmd & !expander::CMD_AUTO_INCREMENT;
                if !auto && out.len() > 1 {
                    return Err(DeviceError::Bus);
                }
                let inputs = unit.input_ports();
                let readback = unit.expander.output_readback();
                for (i, byte) in out.iter_mut().enumerate() {
                    let reg = base + i as u8;
                    *byte = match reg {
                        r if (expander::REG_INPUT0..expander::REG_INPUT0 + 3).contains(&r) => {
                            inputs[(r - expander::REG_INPUT0) as usize]
                        }
                        r if (expander::REG_OUTPUT0..expander::REG_OUTPUT0 + 3).contains(&r) => {
                            readback[(r - expander::REG_OUTPUT0) as usize]
                        }
                        r if (expander::REG_POLARITY0..expander::REG_POLARITY0 + 3).contains(&r) => {
                            unit.expander.polarity[(r - expander::REG_POLARITY0) as usize]
                        }
                        r if (expander::REG_CONFIG0..expander::REG_CONFIG0 + 3).contains(&r) => {
                            unit.expander.config[(r - expander::REG_CONFIG0) as usize]
                        }
                        _ => return Err(DeviceError::Bus),
                    };
                }
                Ok(())
            }
            ina238::INA_SLOT_A | ina238::INA_SLOT_B | ina238::INA_UNIT => {
                let part = match addr {
                    ina238::INA_SLOT_A => unit.slots[0].ina,
                    ina238::INA_SLOT_B => unit.slots[1].ina,
                    _ => unit.input,
                };
                if !part.present {
                    return Err(DeviceError::Nak);
                }
                let [reg] = cmd else {
                    return Err(DeviceError::Bus);
                };
                if !part.read(*reg, out) {
                    return Err(DeviceError::Bus);
                }
                // Reading DIAG_ALRT clears its latched flags.
                if *reg == ina238::REG_DIAG_ALRT {
                    let target = match addr {
                        ina238::INA_SLOT_A => &mut unit.slots[0].ina,
                        ina238::INA_SLOT_B => &mut unit.slots[1].ina,
                        _ => &mut unit.input,
                    };
                    target.diag &= 0xF800;
                }
                Ok(())
            }
            idadc::ADC_SLOT_A | idadc::ADC_SLOT_B => {
                let part = if addr == idadc::ADC_SLOT_A {
                    unit.slots[0].adc
                } else {
                    unit.slots[1].adc
                };
                if !part.present {
                    return Err(DeviceError::Nak);
                }
                let [reg] = cmd else {
                    return Err(DeviceError::Bus);
                };
                match (*reg, out.len()) {
                    (idadc::REG_RESULT, 2) => {
                        out.copy_from_slice(&(part.counts & idadc::FULL_SCALE).to_be_bytes());
                        Ok(())
                    }
                    (idadc::REG_ALERT_STATUS | idadc::REG_CONFIGURATION, 1) => {
                        out[0] = 0;
                        Ok(())
                    }
                    _ => Err(DeviceError::Bus),
                }
            }
            _ => Err(DeviceError::Nak),
        }
    }

    fn reset_mux(&mut self) -> DevResult<()> {
        self.mux_resets += 1;
        self.mux_control = 0;
        Ok(())
    }

    fn unstick(&mut self) -> DevResult<()> {
        self.unsticks += 1;
        // Nine clocks free a slave that was mid-transfer.
        self.stuck = false;
        Ok(())
    }
}
