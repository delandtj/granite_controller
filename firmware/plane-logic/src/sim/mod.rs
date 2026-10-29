//! Simulated devices, behind the `sim` feature.
//!
//! Everything here is a fake of one of [`crate::device`]'s traits, with
//! enough of the board's behaviour that the plane logic cannot tell the
//! difference: expander registers with pin semantics, INA238s whose
//! power limit really trips the slot's OV latch, ID ADCs, and a host per
//! slot whose power LED follows the PWR presses and the rail, so a whole
//! `node off` or `power-cycle` sequence runs to its end.
//!
//! The press deadline is modelled too: [`store::FakePins`] records every
//! PC6 toggle and derives PRESS_EN from the last edge the way the
//! 74LVC1G123 does, and [`bus::FakeBus::advance`] passes it to the
//! photoMOS. A plane that stops toggling therefore really does lose its
//! presses, which is the property ADR 0002 asks the firmware to have.
//!
//! [`Sim`] ties the fakes to a [`Plane`] and a fake clock:
//!
//! ```
//! use plane_logic::sim::Sim;
//! let mut sim = Sim::with_units(2);
//! sim.boot();
//! sim.advance(5_000);
//! assert_eq!(sim.plane.units_present().bits(), 0b0011);
//! ```
//!
//! granite-sim's fake plane is the same pieces with a simulated I2C
//! slave in front of [`Plane::handle_frame`].

pub mod bus;
pub mod store;
pub mod unit;

use alloc::vec::Vec;

use granite_proto::{
    BLK_CTRL, BLK_CONFIG_W, BLK_EV_ACK, BLK_FW, Command, INDEX_NONE, MAX_PAYLOAD, RecordWriter,
    ResultCode, frame,
};

use crate::device::Devices;
use crate::model::POLL_PERIOD_MS;
use crate::plane::{Plane, Setup, reset_cause};

pub use bus::FakeBus;
pub use store::{FakeBackup, FakeConfigStore, FakePins, FakeUpdateStore};
pub use unit::{FakeExpander, FakeIdAdc, FakeIna, FakeNode, FakeSlot, FakeUnit};

/// Every fake device in one [`Devices`].
#[derive(Debug, Default)]
pub struct Fakes {
    /// The segment bus, the mux and the units.
    pub bus: FakeBus,
    /// The core's pins and sensors.
    pub pins: FakePins,
    /// The backup registers.
    pub backup: FakeBackup,
    /// The config pages.
    pub config: FakeConfigStore,
    /// The DFU slot.
    pub update: FakeUpdateStore,
}

impl Fakes {
    /// Fakes with `n` populated units on the first `n` channels.
    pub fn with_units(n: usize) -> Self {
        Fakes {
            bus: FakeBus::with_units(n),
            pins: FakePins::new(),
            ..Fakes::default()
        }
    }
}

impl Devices for Fakes {
    type Bus = FakeBus;
    type Pins = FakePins;
    type Backup = FakeBackup;
    type Config = FakeConfigStore;
    type Update = FakeUpdateStore;

    fn bus(&mut self) -> &mut FakeBus {
        &mut self.bus
    }

    fn pins(&mut self) -> &mut FakePins {
        &mut self.pins
    }

    fn backup(&mut self) -> &mut FakeBackup {
        &mut self.backup
    }

    fn config_store(&mut self) -> &mut FakeConfigStore {
        &mut self.config
    }

    fn update_store(&mut self) -> &mut FakeUpdateStore {
        &mut self.update
    }
}

/// The address the harness gives its plane.
pub const SIM_ADDRESS: u8 = 0x33;

/// A plane, its fake devices and a fake clock.
#[derive(Debug)]
pub struct Sim {
    /// The plane under test.
    pub plane: Plane,
    /// Its devices.
    pub dev: Fakes,
    /// The fake clock \[ms\].
    pub now_ms: u64,
    /// Ticks since the harness was built.
    pub ticks: u32,
    /// How often the plane said the watchdog could be fed.
    pub feeds: u32,
}

impl Sim {
    /// A plane with the fakes given, at [`SIM_ADDRESS`].
    pub fn new(dev: Fakes) -> Self {
        Sim::with_address(SIM_ADDRESS, dev)
    }

    /// A plane at a chosen address: the straps are 0x30 to 0x37, and a
    /// golden frame's CRC covers the address it was built for.
    pub fn with_address(address: u8, dev: Fakes) -> Self {
        Sim {
            plane: Plane::new(Setup::bare(address)),
            dev,
            now_ms: 0,
            ticks: 0,
            feeds: 0,
        }
    }

    /// A plane with `n` populated units.
    pub fn with_units(n: usize) -> Self {
        Sim::new(Fakes::with_units(n))
    }

    /// Boot the plane as a power-on reset: the backup registers are
    /// whatever the fakes hold.
    pub fn boot(&mut self) {
        self.boot_with(reset_cause::POWER_ON);
    }

    /// Boot the plane with a reset cause.
    pub fn boot_with(&mut self, cause: u8) {
        self.dev.pins.now_ms = self.now_ms;
        self.plane.boot(self.now_ms, &mut self.dev, cause);
    }

    /// A core reset that keeps VDD: the backup registers survive and so
    /// do the units' expanders, which is the case ADR 0002 decision 3
    /// calls "backup registers valid".
    pub fn core_reset(&mut self, cause: u8) {
        self.plane = Plane::new(Setup::bare(self.plane.address()));
        self.boot_with(cause);
    }

    /// A core power loss: the backup registers go with it, the units
    /// keep their own inputs (ADR 0005: one input per unit).
    pub fn core_power_loss(&mut self) {
        self.dev.backup.wipe();
        self.core_reset(reset_cause::POWER_ON);
    }

    /// One 50 ms step: the hardware first, then the plane.
    pub fn tick(&mut self) {
        self.step(POLL_PERIOD_MS as u64);
    }

    /// Advance by `ms`, in whole poll periods (at least one step).
    pub fn advance(&mut self, ms: u64) {
        let steps = (ms / POLL_PERIOD_MS as u64).max(1);
        for _ in 0..steps {
            self.step(POLL_PERIOD_MS as u64);
        }
    }

    /// One step of `dt` milliseconds.
    pub fn step(&mut self, dt: u64) {
        let press_en = self.dev.pins.press_en(self.now_ms);
        self.dev.bus.advance(dt as u32, press_en);
        self.now_ms += dt;
        self.dev.pins.now_ms = self.now_ms;
        self.plane.tick(self.now_ms, &mut self.dev);
        self.ticks += 1;
        if self.plane.may_feed_watchdog() {
            self.feeds += 1;
        }
    }

    /// Read a block through a real frame, checking its CRC on the way
    /// back: the payload records, as the controller would see them.
    pub fn read_block(&mut self, block: u8, index: u8) -> Vec<u8> {
        let address = self.plane.address();
        let request = frame::read_request(block, index);
        let mut out = [0u8; frame::MAX_ANSWER];
        let answer = self
            .plane
            .handle_frame(&request, &mut out)
            .expect("a read is answered")
            .to_vec();
        frame::check_answer(address, &request, &answer)
            .expect("the answer's CRC checks out")
            .to_vec()
    }

    /// Write a block through a real frame.
    pub fn write_block(&mut self, block: u8, payload: &[u8]) {
        let address = self.plane.address();
        let mut out = [0u8; frame::MAX_WRITE_FRAME];
        let written = frame::write_frame(&mut out, address, block, INDEX_NONE, payload)
            .expect("the write frame fits")
            .to_vec();
        let mut answer = [0u8; frame::MAX_ANSWER];
        assert!(
            self.plane.handle_frame(&written, &mut answer).is_none(),
            "a write has no answer"
        );
    }

    /// Send a CTRL write with a sequence number and some commands.
    pub fn ctrl(&mut self, seq: u8, commands: &[Command]) {
        let mut buf = [0u8; MAX_PAYLOAD];
        let mut w = RecordWriter::new(&mut buf);
        Command::write_ctrl(&mut w, seq, commands).expect("the commands fit");
        let payload = w.payload().to_vec();
        self.write_block(BLK_CTRL, &payload);
    }

    /// Send a CTRL write and run one tick, so an immediate command has
    /// its result.
    pub fn ctrl_now(&mut self, seq: u8, commands: &[Command]) -> ResultCode {
        self.ctrl(seq, commands);
        self.tick();
        self.result(seq).expect("the write has a result")
    }

    /// The result the plane holds for a command sequence number.
    pub fn result(&self, seq: u8) -> Option<ResultCode> {
        self.plane
            .command_results()
            .find(|held| held.seq == seq)
            .map(|held| held.code)
    }

    /// Acknowledge every event up to and including `seq`.
    pub fn ev_ack(&mut self, seq: u16) {
        let mut buf = [0u8; 8];
        let mut w = RecordWriter::new(&mut buf);
        granite_proto::AckThrough::new(seq)
            .encode(&mut w)
            .expect("the record fits");
        let payload = w.payload().to_vec();
        self.write_block(BLK_EV_ACK, &payload);
        self.tick();
    }

    /// Shorten the blink window so a test does not have to wait out the
    /// 4 s default before an LED is called steady.
    ///
    /// Every wait on an LED is at least the window long
    /// ([`Plane::led_settle_ms`] is not public, but the arithmetic is
    /// `max_period + 1 s`), which is the ADR's own constraint and not a
    /// test artifact: with the default window a `node off` takes some
    /// fifteen seconds.
    pub fn fast_led(&mut self, min_period_ms: u16, max_period_ms: u16) {
        self.config_write(&[
            granite_proto::Config::BlinkDetection(granite_proto::BlinkDetection {
                min_period_ms,
                max_period_ms,
            }),
            granite_proto::Config::Commit,
        ]);
    }

    /// Write a CONFIG_W set.
    pub fn config_write(&mut self, records: &[granite_proto::Config]) {
        let mut buf = [0u8; MAX_PAYLOAD];
        let mut w = RecordWriter::new(&mut buf);
        for record in records {
            record.encode(&mut w).expect("the set fits");
        }
        let payload = w.payload().to_vec();
        self.write_block(BLK_CONFIG_W, &payload);
        self.tick();
    }

    /// Write an FW record set.
    pub fn fw_write(&mut self, records: &[granite_proto::Fw<'_>]) {
        let mut buf = [0u8; MAX_PAYLOAD];
        let mut w = RecordWriter::new(&mut buf);
        for record in records {
            record.encode(&mut w).expect("the records fit");
        }
        let payload = w.payload().to_vec();
        self.write_block(BLK_FW, &payload);
        self.tick();
    }

    /// Every event the plane has not had acknowledged.
    pub fn events(&self) -> Vec<granite_proto::Event> {
        self.plane.events().iter().copied().collect()
    }

    /// True while the plane has raised an event of this kind.
    pub fn saw(&self, kind: granite_proto::EventKind) -> bool {
        self.plane.events().iter().any(|e| e.kind == kind)
    }

    /// One unit's fake hardware.
    pub fn unit(&mut self, ch: usize) -> &mut FakeUnit {
        self.dev.bus.unit(ch)
    }

    /// One slot's fake hardware.
    pub fn fake_slot(&mut self, slot: u8) -> &mut FakeSlot {
        let (unit, b) = ((slot / 2) as usize, (slot % 2) as usize);
        &mut self.dev.bus.unit(unit).slots[b]
    }
}
