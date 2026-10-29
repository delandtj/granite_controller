//! The fake controller board: the 1-wire probes, the dry contacts, VIN
//! and the board temperature.
//!
//! Rev D has nothing else on it (controller ADR 0004 decision 1): the
//! node side of the hardware is the plane, and the planes are
//! [`crate::planes`], which run the real plane firmware core over the
//! real wire format. Dry inputs sit straight on MCU GPIOs, so there is
//! no expander to fake either.

use granite_core::hal::{DryInputs, HalResult};

use crate::scenario::Scenario;

/// The fake sensors: probes, dry contacts, VIN, board temperature.
#[derive(Debug)]
pub struct FakeSensors {
    scenario: Scenario,
    /// Probes the last scan found, in scenario order.
    pub present: Vec<u64>,
}

impl FakeSensors {
    /// Sensors from a scenario, with every probe present.
    pub fn new(scenario: &Scenario) -> Self {
        let mut s = FakeSensors {
            scenario: scenario.clone(),
            present: Vec::new(),
        };
        s.scan();
        s
    }

    /// Re-enumerate the bus.
    pub fn scan(&mut self) -> Vec<u64> {
        self.present = self
            .scenario
            .probes
            .iter()
            .filter_map(|p| granite_core::hal::rom_id_from_hex(&p.rom))
            .collect();
        self.present.clone()
    }

    /// Temperature of one probe in centi-degrees, with the scenario ramp
    /// applied.
    pub fn probe_centi_c(&self, rom: u64, now_ms: u64) -> Option<i16> {
        let p = self
            .scenario
            .probes
            .iter()
            .find(|p| granite_core::hal::rom_id_from_hex(&p.rom) == Some(rom))?;
        let minutes = now_ms as f32 / 60_000.0;
        let c = p.temp_c + p.ramp_c_per_min * minutes;
        Some((c * 100.0) as i16)
    }

    /// Board temperature in centi-degrees.
    pub fn board_centi_c(&self) -> i16 {
        (self.scenario.hardware.board_temp_c * 100.0) as i16
    }

    /// Bus voltage in millivolts, with the scenario ramp applied.
    pub fn vin_mv(&self, now_ms: u64) -> u32 {
        let minutes = now_ms as f64 / 60_000.0;
        let drift = self.scenario.hardware.vin_ramp_mv_per_min as f64 * minutes;
        (self.scenario.hardware.vin_mv as f64 + drift).max(0.0) as u32
    }

    /// The dry-contact byte.
    pub fn dry_bits(&self) -> u8 {
        let mut bits = 0u8;
        for (i, closed) in self.scenario.hardware.dry_in.iter().enumerate() {
            if *closed {
                bits |= 1 << i;
            }
        }
        bits
    }

    /// Ethernet link.
    pub fn link_up(&self) -> bool {
        self.scenario.hardware.link_up
    }

    /// Close or open a dry contact at runtime.
    pub fn set_dry(&mut self, input: usize, closed: bool) {
        if (1..=4).contains(&input) {
            self.scenario.hardware.dry_in[input - 1] = closed;
        }
    }

    /// The scenario behind these sensors.
    pub fn scenario(&self) -> &Scenario {
        &self.scenario
    }
}

impl DryInputs for FakeSensors {
    fn read_dry(&mut self) -> HalResult<u8> {
        Ok(self.dry_bits())
    }
}
