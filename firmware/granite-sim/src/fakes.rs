//! The fake board: U14 relay outputs, the power LEDs that answer them,
//! the 1-wire probes, the dry contacts, VIN and the board temperature.
//!
//! The press reaction model is the interesting part, and it is the same
//! model the granite-core tests use (`granite-core/tests/common`), with
//! the motherboard behaviour the ADR action table assumes:
//!
//! - a short PWR press on a dark node powers it on after `on_delay_ms`,
//! - a short PWR press on a lit node starts an ACPI shutdown and the LED
//!   goes dark `off_delay_ms` later,
//! - PWR held for 4 s or more cuts power immediately, which is why the
//!   actuator's hardware deadline matters,
//! - a short RST press does not change the LED.

use granite_core::NodeId;
use granite_core::hal::{HalResult, NodeSwitches, Switch};

use crate::scenario::{Respond, Scenario};

/// The motherboard force-off threshold: PWR held this long cuts power.
pub const FORCE_OFF_HOLD_MS: u64 = 4_000;

/// Longest press still treated as a short press by the fake board.
pub const SHORT_PRESS_MAX_MS: u64 = 1_200;

/// One scheduled LED change.
#[derive(Debug, Clone, Copy)]
struct Pending {
    at_ms: u64,
    node: NodeId,
    value: bool,
}

/// The fake U14 plus the LEDs it moves. The actuator owns this through
/// [`granite_core::actuator::Actuator`], so every relay action in the
/// simulator goes through exactly the code the board runs.
#[derive(Debug)]
pub struct FakeSwitches {
    now_ms: u64,
    /// LED per node, index 0 is node 1.
    led: [bool; 8],
    /// Whether the LED can be read at all.
    sense_broken: [bool; 8],
    respond: [Respond; 8],
    on_delay: [u64; 8],
    off_delay: [u64; 8],
    /// When PWR/RST was asserted, per node.
    pressed_at: [[Option<u64>; 2]; 8],
    pending: Vec<Pending>,
    /// Relays currently closed, for the "one press at a time" assertion
    /// the simulator makes on itself.
    closed: usize,
    /// Set when the deadline was armed; the simulator does not run a real
    /// timer, the actuator's software backstop is enough on the host.
    pub armed_ms: Option<u32>,
    /// Press log, shown on the simulator console.
    pub log: Vec<String>,
}

fn switch_index(sw: Switch) -> usize {
    match sw {
        Switch::Pwr => 0,
        Switch::Rst => 1,
    }
}

impl FakeSwitches {
    /// Build the fake board from a scenario.
    pub fn new(scenario: &Scenario) -> Self {
        let mut hw = FakeSwitches {
            now_ms: 0,
            led: [false; 8],
            sense_broken: [false; 8],
            respond: [Respond::Normal; 8],
            on_delay: [1_500; 8],
            off_delay: [4_000; 8],
            pressed_at: [[None; 2]; 8],
            pending: Vec::new(),
            closed: 0,
            armed_ms: None,
            log: Vec::new(),
        };
        for n in scenario.nodes.iter() {
            let i = n.node as usize - 1;
            hw.led[i] = n.powered;
            hw.sense_broken[i] = n.sense_broken;
            hw.respond[i] = n.respond;
            hw.on_delay[i] = n.on_delay_ms;
            hw.off_delay[i] = n.off_delay_ms;
            if n.respond == Respond::Slow {
                hw.on_delay[i] = n.on_delay_ms.max(20_000);
                hw.off_delay[i] = n.off_delay_ms.max(200_000);
            }
        }
        hw
    }

    /// Advance the fake board: apply scheduled LED changes and the
    /// hardware force-off of a long PWR hold.
    pub fn tick(&mut self, now_ms: u64) {
        self.now_ms = now_ms;
        let due: Vec<Pending> = self
            .pending
            .iter()
            .copied()
            .filter(|p| p.at_ms <= now_ms)
            .collect();
        self.pending.retain(|p| p.at_ms > now_ms);
        for p in due {
            self.set_led(p.node, p.value);
        }
        for node in 1..=8u8 {
            let i = node as usize - 1;
            if let Some(start) = self.pressed_at[i][0]
                && now_ms.saturating_sub(start) >= FORCE_OFF_HOLD_MS
                && self.led[i]
                && self.respond[i] != Respond::Never
            {
                self.log
                    .push(format!("node {node}: 4 s PWR hold, power cut"));
                self.set_led(node, false);
            }
        }
    }

    /// The U15 LED byte, or `None` when no bit can be trusted.
    pub fn leds(&self) -> Option<u8> {
        if self.sense_broken.iter().all(|b| *b) {
            return None;
        }
        let mut bits = 0u8;
        for (i, on) in self.led.iter().enumerate() {
            if *on && !self.sense_broken[i] {
                bits |= 1 << i;
            }
        }
        Some(bits)
    }

    /// True while the LED of `node` is lit.
    pub fn is_lit(&self, node: NodeId) -> bool {
        self.led[node as usize - 1]
    }

    /// Force a LED, as a scenario step would.
    pub fn set_led(&mut self, node: NodeId, value: bool) {
        self.led[node as usize - 1] = value;
    }

    fn schedule(&mut self, node: NodeId, value: bool, delay_ms: u64) {
        self.pending.push(Pending {
            at_ms: self.now_ms + delay_ms,
            node,
            value,
        });
    }

    /// What a motherboard does when the button is let go.
    fn on_release(&mut self, node: NodeId, sw: Switch, held_ms: u64) {
        let i = node as usize - 1;
        if self.respond[i] == Respond::Never {
            return;
        }
        if sw == Switch::Rst {
            // A reset keeps the power LED lit; nothing to schedule.
            return;
        }
        if held_ms >= FORCE_OFF_HOLD_MS {
            // Already handled while the button was held.
            return;
        }
        if held_ms > SHORT_PRESS_MAX_MS {
            // A medium press: some boards ignore it. The simulator does.
            self.log
                .push(format!("node {node}: {held_ms} ms press ignored"));
            return;
        }
        if self.led[i] {
            let delay = self.off_delay[i];
            self.log
                .push(format!("node {node}: soft-off, LED off in {delay} ms"));
            self.schedule(node, false, delay);
        } else {
            let delay = self.on_delay[i];
            self.log
                .push(format!("node {node}: power-on, LED on in {delay} ms"));
            self.schedule(node, true, delay);
        }
    }
}

impl NodeSwitches for FakeSwitches {
    fn assert(&mut self, node: NodeId, sw: Switch) -> HalResult<()> {
        let i = node as usize - 1;
        self.pressed_at[i][switch_index(sw)] = Some(self.now_ms);
        self.closed += 1;
        debug_assert!(
            self.closed <= 1,
            "the actuator closed two relays at once, which the hardware forbids"
        );
        Ok(())
    }

    fn release(&mut self, node: NodeId, sw: Switch) -> HalResult<()> {
        let i = node as usize - 1;
        let held = self.pressed_at[i][switch_index(sw)]
            .map(|start| self.now_ms.saturating_sub(start))
            .unwrap_or(0);
        self.pressed_at[i][switch_index(sw)] = None;
        self.closed = self.closed.saturating_sub(1);
        self.on_release(node, sw, held);
        Ok(())
    }

    fn release_all(&mut self) -> HalResult<()> {
        for i in 0..8 {
            for s in 0..2 {
                self.pressed_at[i][s] = None;
            }
        }
        self.closed = 0;
        self.armed_ms = None;
        Ok(())
    }

    fn arm_deadline(&mut self, ms: u32) -> HalResult<()> {
        self.armed_ms = Some(ms);
        Ok(())
    }

    fn disarm_deadline(&mut self) -> HalResult<()> {
        self.armed_ms = None;
        Ok(())
    }
}

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
