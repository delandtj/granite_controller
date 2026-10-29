//! The fake plane link: up to eight simulated planes on one isolated
//! I2C bus (controller ADR 0004 architecture item 3).
//!
//! Each plane is a real [`plane_logic::Plane`] over plane-logic's own
//! fake devices (`plane-logic/src/sim`): the expander with pin
//! semantics, the INA238s, the ID ADCs and a host per slot whose power
//! LED follows the PWR presses and the rail. The link in front of them
//! does what the controller's I2C peripheral does and no more: it hands
//! the request bytes to [`plane_logic::Plane::handle_frame`] and gives
//! the answer bytes back. Framing, CRC, records and retries are
//! granite-proto and the plane client's business (ADR 0003).
//!
//! A plane is a separate MCU: [`FakePlaneLink::tick`] advances every
//! plane at its own 20 Hz cadence whether or not the controller talks to
//! it, which is why a sequence finishes and an event shows up on EXP_INT
//! with nobody polling.
//!
//! The fault knobs ([`FakePlane::inject`]) are the point of the whole
//! thing: they are what a plane client's tests need and what a bench
//! cannot produce on demand.

use granite_core::hal::{LinkError, PlaneLink};
use granite_proto::{MAX_PLANES, SLOT_COUNT, UNIT_COUNT, frame, plane_of_address};
use plane_logic::drivers::{idadc, ina238};
use plane_logic::model::POLL_PERIOD_MS;
use plane_logic::reset_cause;
use plane_logic::sim::{FakeSlot, FakeUnit, Fakes, Sim};

use crate::scenario::{
    FaultKind, PlaneFault, PlaneRespond, PlaneScenario, PlaneSlotScenario, Scenario,
};

/// One step of a plane's own cycle \[ms\]: the 20 Hz bus task of
/// ADR 0002 decision 5.
pub const PLANE_TICK_MS: u64 = POLL_PERIOD_MS as u64;

/// One simulated plane: the plane logic, its fake hardware and the
/// faults scheduled for it.
#[derive(Debug)]
pub struct FakePlane {
    /// The plane, its devices and its clock.
    pub sim: Sim,
    /// What the host in each slot draws while its rail is up \[mA\].
    pub load_ma: [u32; SLOT_COUNT],
    /// Transactions still to be answered with a NAK.
    pub drop_next: u32,
    /// Answers still to come back with a byte flipped.
    pub corrupt_next: u32,
    /// Transactions the plane answered.
    pub transactions: u64,
    /// Transactions dropped on the way in.
    pub dropped: u64,
    /// Answers sent back corrupted.
    pub corrupted: u64,
    /// Resets the plane performed on itself (a commanded reboot, or a
    /// probation that ran out).
    pub resets: u32,
    /// The scenario's faults, soonest first.
    faults: Vec<PlaneFault>,
    /// How many of them have fired.
    fired: usize,
}

impl FakePlane {
    /// Build a plane from its scenario section and boot it.
    pub fn new(scenario: &PlaneScenario) -> Self {
        // `with_units(0)` is an empty chain with the core's pins at
        // 48 V and 25 C; the units go on the channels the scenario
        // names, which need not be the first ones.
        let mut dev = Fakes::with_units(0);
        let mut load_ma = [0u32; SLOT_COUNT];
        for unit in 0..UNIT_COUNT as u8 {
            if !scenario.has_unit(unit) {
                continue;
            }
            let mut fake = FakeUnit::new();
            for half in 0..2usize {
                let index = 2 * unit + half as u8;
                let slot = scenario.slot(index);
                load_ma[index as usize] = slot.load_ma;
                apply_slot(&mut fake.slots[half], &slot);
            }
            dev.bus.units[unit as usize] = Some(fake);
        }

        let mut plane = FakePlane {
            sim: Sim::with_address(scenario.address, dev),
            load_ma,
            drop_next: 0,
            corrupt_next: 0,
            transactions: 0,
            dropped: 0,
            corrupted: 0,
            resets: 0,
            faults: scenario.faults.clone(),
            fired: 0,
        };
        plane.sim.boot();
        if let Some([min_period_ms, max_period_ms]) = scenario.blink_window_ms {
            plane.sim.fast_led(min_period_ms, max_period_ms);
        }
        plane
    }

    /// The plane's I2C address.
    pub fn address(&self) -> u8 {
        self.sim.plane.address()
    }

    /// Simulated time this plane has reached \[ms\].
    pub fn now_ms(&self) -> u64 {
        self.sim.now_ms
    }

    /// True while this plane holds EXP_INT: it has unacknowledged
    /// events.
    pub fn event_pending(&self) -> bool {
        self.sim.plane.event_pending()
    }

    /// One 50 ms step of the plane's cycle.
    ///
    /// The slot loads are re-applied first: [`FakeUnit::advance`] zeroes
    /// a slot's current while its rail is down, so a load is a property
    /// to restate every step rather than a value to set once.
    ///
    /// A plane that asked for a reset gets one, because a plane is its
    /// own MCU: nothing on the controller's side performs it, and a
    /// commanded reboot or a probation that ran out has to really
    /// restart the thing.
    pub fn step(&mut self) {
        for index in 0..SLOT_COUNT {
            let load = self.load_ma[index];
            if let Some(slot) = self.slot_mut(index as u8)
                && slot.pgd
            {
                slot.ina.current_ma = load;
            }
        }
        self.sim.step(PLANE_TICK_MS);
        if self.sim.plane.reset_requested() {
            self.self_reset();
        }
    }

    /// The reset the plane asked for, with the bootloader's part of
    /// ADR 0002 decision 9.
    ///
    /// A marked swap runs the new image on probation; a probation that
    /// ran out (the plane asks for a reset and nothing confirmed the
    /// image) swaps back and the plane comes up saying it rolled back.
    /// The DFU slot's bytes are not executed by anything here, so the
    /// version the plane reports does not change: the controller
    /// confirms on the units it sees, not on the version (controller
    /// ADR 0004 decision 8).
    fn self_reset(&mut self) {
        let store = &mut self.sim.dev.update;
        if store.swap_marked {
            store.swap_marked = false;
            store.booted = false;
            store.unconfirmed = true;
            store.rolled_back = false;
        } else if store.unconfirmed && !store.booted {
            store.unconfirmed = false;
            store.booted = true;
            store.rolled_back = true;
        }
        self.sim.core_reset(reset_cause::SOFTWARE);
        self.resets += 1;
    }

    /// Advance to `now_ms` in whole cycles, firing the faults due on the
    /// way.
    pub fn advance_to(&mut self, now_ms: u64) {
        self.fire_due();
        while self.sim.now_ms + PLANE_TICK_MS <= now_ms {
            self.step();
            self.fire_due();
        }
    }

    /// Fire every scheduled fault the plane's own clock has reached.
    fn fire_due(&mut self) {
        while let Some(fault) = self.faults.get(self.fired).copied() {
            if fault.at_s.saturating_mul(1_000) > self.sim.now_ms {
                break;
            }
            self.fired += 1;
            self.inject(fault);
        }
    }

    // --- fault knobs --------------------------------------------------

    /// Inject one fault now.
    pub fn inject(&mut self, fault: PlaneFault) {
        match fault.kind {
            FaultKind::DropTransactions => {
                self.drop_next = self.drop_next.saturating_add(fault.count);
            }
            FaultKind::CorruptAnswer => {
                self.corrupt_next = self.corrupt_next.saturating_add(fault.count);
            }
            FaultKind::Watchdog => self.reboot(reset_cause::WATCHDOG),
            FaultKind::PowerLoss => self.power_loss(),
            FaultKind::UnitUnplug => self.set_unit_power(fault.unit, false),
            FaultKind::UnitReplug => self.set_unit_power(fault.unit, true),
            FaultKind::UnitHotSwapLatch => self.latch_unit_hot_swap(fault.unit),
            FaultKind::OvLatch => self.latch_ov(fault.slot),
            FaultKind::PgdLoss => self.drop_pgd(fault.slot),
            FaultKind::PowerLimitTrip => self.trip_power_limit(fault.slot, fault.value as u16),
            FaultKind::IgnorePwr => self.ignore_pwr(fault.slot, fault.on),
            FaultKind::Load => self.set_load(fault.slot, fault.value),
        }
    }

    /// NAK the next `count` transactions.
    pub fn drop_transactions(&mut self, count: u32) {
        self.drop_next = self.drop_next.saturating_add(count);
    }

    /// Flip a byte of the next `count` answers, so the controller's CRC
    /// check rejects them.
    pub fn corrupt_answers(&mut self, count: u32) {
        self.corrupt_next = self.corrupt_next.saturating_add(count);
    }

    /// Reset the plane with VDD held: the backup registers and the
    /// units' expanders survive, so an OFF slot never came on
    /// (ADR 0002 decision 3).
    pub fn reboot(&mut self, cause: u8) {
        self.sim.core_reset(cause);
    }

    /// Cut the plane's own power: the backup registers go with it, OFF
    /// slots come back ON and STATUS flags "intended state lost".
    pub fn power_loss(&mut self) {
        self.sim.core_power_loss();
    }

    /// Pull or restore a unit's 3V3 (its chain cable).
    pub fn set_unit_power(&mut self, unit: u8, powered: bool) {
        if let Some(fake) = self.unit_mut(unit) {
            if powered {
                fake.power_on();
            } else {
                fake.power_off();
            }
        }
    }

    /// Latch a unit's own hot-swap off (LM5069-1): only CTRL 0x4A
    /// re-arms it.
    pub fn latch_unit_hot_swap(&mut self, unit: u8) {
        if let Some(fake) = self.unit_mut(unit) {
            fake.hotswap_latched = true;
        }
    }

    /// Set a slot's overvoltage latch; only LCLR clears it.
    pub fn latch_ov(&mut self, slot: u8) {
        if let Some(fake) = self.slot_mut(slot) {
            fake.ov_latched = true;
        }
    }

    /// Drop a slot's PGD for one cycle: a hot-swap that retried.
    pub fn drop_pgd(&mut self, slot: u8) {
        if let Some(fake) = self.slot_mut(slot) {
            fake.glitch_pgd = true;
        }
    }

    /// Trip a slot's power limit by writing PWR_LIMIT under its load.
    ///
    /// `deciwatt` of 0 picks half the power the slot is drawing, which
    /// trips on the next sample whatever the load is.
    pub fn trip_power_limit(&mut self, slot: u8, deciwatt: u16) {
        if let Some(fake) = self.slot_mut(slot) {
            let limit = if deciwatt > 0 {
                deciwatt
            } else {
                ((fake.ina.power_mw() / 100 / 2) as u16).max(1)
            };
            fake.ina.power_limit = ina238::power_limit_register(limit);
        }
    }

    /// The host in this slot stops (or resumes) answering its PWR
    /// button: an operating system that will not shut down.
    pub fn ignore_pwr(&mut self, slot: u8, on: bool) {
        if let Some(fake) = self.slot_mut(slot) {
            fake.node.ignores_short_press = on;
            fake.node.ignores_hold = on;
        }
    }

    /// Set what the host in this slot draws \[mA\].
    pub fn set_load(&mut self, slot: u8, milliamps: u32) {
        if (slot as usize) < SLOT_COUNT {
            self.load_ma[slot as usize] = milliamps;
        }
    }

    /// One slot's fake hardware, if its unit is on the chain.
    pub fn slot_mut(&mut self, slot: u8) -> Option<&mut FakeSlot> {
        let (unit, half) = ((slot / 2) as usize, (slot % 2) as usize);
        self.sim
            .dev
            .bus
            .units
            .get_mut(unit)?
            .as_mut()
            .map(|fake| &mut fake.slots[half])
    }

    /// One unit's fake hardware, if it is on the chain.
    pub fn unit_mut(&mut self, unit: u8) -> Option<&mut FakeUnit> {
        self.sim.dev.bus.units.get_mut(unit as usize)?.as_mut()
    }

    // --- the link's side ----------------------------------------------

    /// True when this transaction is one of the dropped ones.
    fn dropping(&mut self) -> bool {
        if self.drop_next == 0 {
            return false;
        }
        self.drop_next -= 1;
        self.dropped += 1;
        true
    }

    /// Answer a read request.
    fn answer(&mut self, request: &[u8], resp: &mut [u8]) -> Result<usize, LinkError> {
        if self.dropping() {
            return Err(LinkError::Nak);
        }
        let mut out = [0u8; frame::MAX_ANSWER];
        // A read is always answered; nothing comes back only when the
        // caller handed a write frame to `write_read`.
        let answer = self
            .sim
            .plane
            .handle_frame(request, &mut out)
            .ok_or(LinkError::Bus)?;
        let n = answer.len();
        if resp.len() < n {
            return Err(LinkError::BufferTooSmall);
        }
        self.transactions += 1;
        resp[..n].copy_from_slice(answer);
        if self.corrupt_next > 0 {
            self.corrupt_next -= 1;
            self.corrupted += 1;
            // One flipped bit in the payload. The length byte still
            // matches the bytes that follow, so nothing but the CRC
            // check can tell: exactly the case ADR 0003 retries.
            let i = if n > 2 { 1 } else { n - 1 };
            resp[i] ^= 0x01;
        }
        Ok(n)
    }

    /// Take a write frame.
    fn take(&mut self, bytes: &[u8]) -> Result<(), LinkError> {
        if self.dropping() {
            return Err(LinkError::Nak);
        }
        let mut out = [0u8; frame::MAX_ANSWER];
        // A write has no answer (ADR 0003 decision 1); the plane
        // answering means the caller wrote a read request.
        if self.sim.plane.handle_frame(bytes, &mut out).is_some() {
            return Err(LinkError::Bus);
        }
        self.transactions += 1;
        Ok(())
    }
}

/// Apply one slot's scenario to its fake hardware.
fn apply_slot(fake: &mut FakeSlot, scenario: &PlaneSlotScenario) {
    fake.module = scenario.module;
    fake.adc.counts = if scenario.module {
        scenario.module_id.min(idadc::FULL_SCALE)
    } else {
        idadc::FULL_SCALE
    };
    fake.pgd = true;
    fake.module_pgood = scenario.module;
    fake.node.on = scenario.module && scenario.host_on;
    fake.ina.current_ma = if fake.node.on { scenario.load_ma } else { 0 };
    fake.node.blink_period_ms = (scenario.blink_period_ms > 0).then_some(scenario.blink_period_ms);
    let (short, hold) = match scenario.respond {
        PlaneRespond::Normal => (false, false),
        PlaneRespond::IgnoresShortPress => (true, false),
        PlaneRespond::IgnoresHold => (true, true),
    };
    fake.node.ignores_short_press = short;
    fake.node.ignores_hold = hold;
}

/// The planes on one link, keyed by their address.
#[derive(Debug, Default)]
pub struct FakePlaneLink {
    /// Index n is address 0x30 + n.
    planes: [Option<FakePlane>; MAX_PLANES],
    /// Transactions the link carried, dropped ones and NAKs included.
    pub transactions: u64,
    /// Transactions to an address no plane answers on.
    pub unanswered: u64,
}

impl FakePlaneLink {
    /// A link with no planes on it.
    pub fn new() -> Self {
        FakePlaneLink::default()
    }

    /// The planes a scenario describes, booted.
    pub fn from_scenario(scenario: &Scenario) -> Self {
        let mut link = FakePlaneLink::new();
        for section in &scenario.planes {
            if let Some(n) = plane_of_address(section.address) {
                link.planes[n as usize] = Some(FakePlane::new(section));
            }
        }
        link
    }

    /// Every address a plane answers on, lowest first.
    pub fn addresses(&self) -> Vec<u8> {
        self.planes
            .iter()
            .flatten()
            .map(|plane| plane.address())
            .collect()
    }

    /// The plane at `address`.
    pub fn plane(&self, address: u8) -> Option<&FakePlane> {
        let n = plane_of_address(address)?;
        self.planes[n as usize].as_ref()
    }

    /// The plane at `address`, to set up or inspect its hardware.
    pub fn plane_mut(&mut self, address: u8) -> Option<&mut FakePlane> {
        let n = plane_of_address(address)?;
        self.planes[n as usize].as_mut()
    }

    /// Every plane on the link.
    pub fn planes(&self) -> impl Iterator<Item = &FakePlane> {
        self.planes.iter().flatten()
    }

    /// Every plane on the link, mutably.
    pub fn planes_mut(&mut self) -> impl Iterator<Item = &mut FakePlane> {
        self.planes.iter_mut().flatten()
    }

    /// Advance every plane to `now_ms`. Called from the simulator's main
    /// loop: a plane runs on its own clock, not on the controller's
    /// attention.
    pub fn tick(&mut self, now_ms: u64) {
        for plane in self.planes.iter_mut().flatten() {
            plane.advance_to(now_ms);
        }
    }

    /// Inject a fault into the plane at `address`; false when no plane
    /// is there.
    pub fn inject(&mut self, address: u8, fault: PlaneFault) -> bool {
        match self.plane_mut(address) {
            Some(plane) => {
                plane.inject(fault);
                true
            }
            None => false,
        }
    }
}

impl PlaneLink for FakePlaneLink {
    fn write_read(&mut self, addr: u8, req: &[u8], resp: &mut [u8]) -> Result<usize, LinkError> {
        self.transactions += 1;
        match self.plane_mut(addr) {
            Some(plane) => plane.answer(req, resp),
            None => {
                self.unanswered += 1;
                Err(LinkError::Nak)
            }
        }
    }

    fn write(&mut self, addr: u8, frame: &[u8]) -> Result<(), LinkError> {
        self.transactions += 1;
        match self.plane_mut(addr) {
            Some(plane) => plane.take(frame),
            None => {
                self.unanswered += 1;
                Err(LinkError::Nak)
            }
        }
    }

    fn int_asserted(&mut self) -> bool {
        self.planes().any(FakePlane::event_pending)
    }
}
