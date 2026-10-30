//! [`Plane`]: the whole plane as one state machine, driven by
//! [`Plane::boot`], [`Plane::tick`] and [`Plane::handle_frame`].
//!
//! The split follows ADR 0002's component list: the model is
//! [`crate::model`], the poll cycle [`crate::poll`], discovery
//! [`crate::discovery`], the sequences [`crate::sequencer`], the link
//! [`crate::link`] and the update [`crate::update`]. All of them are
//! `impl Plane` blocks over the one struct defined here, because they
//! are one state machine and splitting the state would only hide the
//! coupling.
//!
//! The firmware calls `tick` from its bus task (the only owner of I2C2)
//! and `handle_frame` from its link task; nothing here blocks, sleeps or
//! allocates, and no method takes longer than one poll cycle over the
//! present units.

use granite_proto::{
    Capabilities, EventKind, EventTarget, ImageState, LinkErrors, PlaneType, SLOT_COUNT,
    SEGMENT_CHANNELS, StatusFlags, UNIT_COUNT, UnitStatus, Version,
};

use crate::backup::{BackupSet, Intended};
use crate::config::{PlaneConfig, StoredConfig};
use crate::device::{CorePins, Devices};
use crate::discovery::Channel;
use crate::drivers::{Expander, Mux};
use crate::events::{Events, Results};
use crate::link::PendingWrite;
use crate::log::{Log, level};
use crate::model::{SlotModel, UnitModel};
use crate::sequencer::{Sequence, Stagger, UnitRelease};
use crate::unit_latch::UnitLatch;
use crate::update::Update;

/// How long the plane waits for FW confirm before it resets and lets the
/// bootloader swap back (ADR 0002 decision 9).
pub const PROBATION_MS: u64 = 120_000;

/// How often PC6 PRESS_TRIG is toggled while a press runs.
///
/// The monostable holds PRESS_EN for about 1 s after an edge, so 200 ms
/// leaves four missed toggles of margin and still bounds a hung firmware
/// to about a second of held button (ADR 0002 open questions).
pub const PRESS_TRIG_PERIOD_MS: u64 = 200;

/// What the plane can do (INFO 0x06). Every bit is true for the chained
/// plane: node control, the per-slot power limit, the unit hot-swap, the
/// press deadline hardware and the firmware update.
pub const CAPABILITIES: Capabilities = Capabilities::NODE_CONTROL
    .with(Capabilities::POWER_LIMIT)
    .with(Capabilities::UNIT_HOT_SWAP)
    .with(Capabilities::PRESS_DEADLINE)
    .with(Capabilities::FIRMWARE_UPDATE);

/// What this plane is, as the firmware knows it at build time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Setup {
    /// The plane's 7-bit I2C address: 0x30 plus the straps.
    pub address: u8,
    /// Serial (INFO 0x05): a hash of the STM32 UID.
    pub serial: u32,
    /// The running application's version.
    pub firmware_version: Version,
    /// The bootloader's version.
    pub bootloader_version: Version,
}

impl Setup {
    /// A setup with the address and serial of a board and no version
    /// claim, for a test or a first bring-up.
    pub const fn bare(address: u8) -> Self {
        Setup {
            address,
            serial: 0,
            firmware_version: Version::new(0, 1, 0),
            bootloader_version: Version::new(0, 1, 0),
        }
    }
}

/// What LED_RUN should be doing (ADR 0002 decision 11).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LedPattern {
    /// 1 Hz: the plane is running.
    Heartbeat,
    /// Fast: a firmware update is in progress.
    Fast,
    /// Double blink: the running image is unconfirmed.
    Double,
}

/// Why the plane last reset, as the firmware reads it (STATUS 0x1B).
///
/// ADR 0003 leaves the byte to the plane, so these are the plane's own
/// codes and the controller only logs them.
pub mod reset_cause {
    /// Unknown or not read.
    pub const UNKNOWN: u8 = 0;
    /// Power-on: VDD was lost, and with it the backup registers.
    pub const POWER_ON: u8 = 1;
    /// The NRST pin.
    pub const PIN: u8 = 2;
    /// The independent watchdog.
    pub const WATCHDOG: u8 = 3;
    /// A software reset, which is how a commanded reboot and a firmware
    /// swap arrive.
    pub const SOFTWARE: u8 = 4;
    /// Brown-out.
    pub const BROWNOUT: u8 = 5;
}

/// The plane.
///
/// One of these is the whole application state: no statics, no interior
/// mutability, nothing that needs an allocator. `size_of` is a little
/// over 2 KB, which leaves the 36 KB of the G070 to the stack, the
/// embassy executor and the I2C buffers.
#[derive(Debug)]
pub struct Plane {
    /// Who this plane is.
    pub(crate) setup: Setup,

    /// Per-slot model, index 0-7 (2 x unit + A/B).
    pub(crate) slots: [SlotModel; SLOT_COUNT],
    /// Per-unit model, index 0-3.
    pub(crate) units: [UnitModel; UNIT_COUNT],

    /// The mux, with the channel it believes is selected.
    pub(crate) mux: Mux,
    /// Each unit's expander and its output latch.
    pub(crate) expanders: [Expander; UNIT_COUNT],
    /// Discovery state per mux channel.
    pub(crate) channels: [Channel; SEGMENT_CHANNELS],

    /// What the plane means to be doing, and the backup set it lives in.
    pub(crate) intended: Intended,
    /// The backup set as last written.
    pub(crate) backup: BackupSet,

    /// The config in force.
    pub(crate) config: PlaneConfig,
    /// What is in the CONFIG pages, if anything.
    pub(crate) stored: Option<StoredConfig>,
    /// A CONFIG_W set being collected; applied on its commit record.
    pub(crate) pending_config: Option<PlaneConfig>,

    /// Per-slot sequence state.
    pub(crate) sequences: [Sequence; SLOT_COUNT],
    /// The plane-level stagger, when one runs.
    pub(crate) stagger: Option<Stagger>,
    /// Per-unit hot-swap latch: the firmware half of the LM5069-2
    /// (ADR 0005 decision 3).
    pub(crate) unit_latch: [UnitLatch; UNIT_COUNT],
    /// Per-unit `unit_reset` releases waiting on their verdict
    /// (CTRL 0x4A).
    pub(crate) unit_releases: [Option<UnitRelease>; UNIT_COUNT],
    /// When PC6 was last toggled.
    pub(crate) press_trig_ms: u64,

    /// The event ring and the command results.
    pub(crate) events: Events,
    /// The last eight command results (STATUS 0x16).
    pub(crate) results: Results,
    /// The RAM log ring (LOG block).
    pub(crate) log: Log,

    /// Writes taken from the link and not executed yet.
    pub(crate) writes: heapless::Deque<PendingWrite, 2>,
    /// Frames dropped on the link (STATUS 0x19).
    pub(crate) link_errors: LinkErrors,

    /// The firmware update state machine.
    pub(crate) update: Update,

    /// Monotonic time of the last tick \[ms\].
    pub(crate) now_ms: u64,
    /// The core's 48 V input \[10 mV\].
    pub(crate) core_volts: u16,
    /// The MCU's own temperature \[0.1 C\].
    pub(crate) core_temperature: i16,
    /// Why the plane last reset.
    pub(crate) reset_cause: u8,
    /// The core lost power: OFF slots came back ON and the controller
    /// has to re-apply (STATUS 0x13).
    pub(crate) intended_state_lost: bool,
    /// Set by a full cycle in which the sequencer ran; cleared by
    /// [`Plane::may_feed_watchdog`].
    pub(crate) watchdog_credit: bool,
    /// The plane wants the firmware to reset it (CTRL 0x4C, or a
    /// probation that ran out).
    pub(crate) reset_request: bool,
    /// When the next 20 Hz cycle is due.
    pub(crate) next_poll_ms: u64,
    /// When the next 2 Hz unit-input sample is due.
    pub(crate) next_unit_poll_ms: u64,
    /// When the next 1 Hz ID read is due.
    pub(crate) next_id_poll_ms: u64,
    /// The OVK level of each slot on the last cycle, bit per slot.
    ///
    /// The link has no bit for OVK (it is three trip sources wire-ORed),
    /// but DIAG_ALRT is only worth a read when the node *changed*, so
    /// the last level has to live somewhere.
    pub(crate) ovk_high: u8,
}

impl Plane {
    /// A plane that has not booted yet.
    ///
    /// Nothing touches hardware here: [`Plane::boot`] does, in the order
    /// ADR 0002 decision 3 fixes.
    pub fn new(setup: Setup) -> Self {
        Plane {
            setup,
            slots: [SlotModel::default(); SLOT_COUNT],
            units: [UnitModel::default(); UNIT_COUNT],
            mux: Mux::new(),
            expanders: [Expander::new(); UNIT_COUNT],
            channels: [Channel::new(); SEGMENT_CHANNELS],
            intended: Intended::hardware_defaults(),
            backup: BackupSet::default(),
            config: PlaneConfig::default(),
            stored: None,
            pending_config: None,
            sequences: [Sequence::idle(); SLOT_COUNT],
            stagger: None,
            unit_latch: [UnitLatch::new(); UNIT_COUNT],
            unit_releases: [None; UNIT_COUNT],
            press_trig_ms: 0,
            events: Events::new(),
            results: Results::new(),
            log: Log::new(),
            writes: heapless::Deque::new(),
            link_errors: LinkErrors::default(),
            update: Update::new(),
            now_ms: 0,
            core_volts: 0,
            core_temperature: 0,
            reset_cause: reset_cause::UNKNOWN,
            intended_state_lost: false,
            watchdog_credit: false,
            reset_request: false,
            next_poll_ms: 0,
            next_unit_poll_ms: 0,
            next_id_poll_ms: 0,
            // Every OVK idles high; a trip pulls it low.
            ovk_high: 0xFF,
        }
    }

    /// The plane's I2C address.
    pub const fn address(&self) -> u8 {
        self.setup.address
    }

    /// Uptime \[s\], the unit STATUS 0x12 and every event carry.
    pub const fn uptime_s(&self) -> u32 {
        (self.now_ms / 1000) as u32
    }

    /// How often this plane has booted (INFO 0x08).
    pub const fn boot_count(&self) -> u32 {
        self.backup.boot_count
    }

    /// The config in force.
    pub const fn config(&self) -> &PlaneConfig {
        &self.config
    }

    /// What the plane means to be doing.
    pub const fn intended(&self) -> Intended {
        self.intended
    }

    /// One slot's model.
    pub fn slot(&self, slot: u8) -> Option<&SlotModel> {
        self.slots.get(slot as usize)
    }

    /// One unit's model.
    pub fn unit(&self, unit: u8) -> Option<&UnitModel> {
        self.units.get(unit as usize)
    }

    /// True while the core holds a unit's hot-swap off after a fault
    /// (ADR 0005 decision 3). Only `unit_reset` or a core power loss
    /// lets go.
    pub fn unit_held_off(&self, unit: u8) -> bool {
        self.unit_latch
            .get(unit as usize)
            .is_some_and(crate::unit_latch::UnitLatch::held)
    }

    /// The event ring.
    pub const fn events(&self) -> &Events {
        &self.events
    }

    /// The log ring.
    pub const fn log(&self) -> &Log {
        &self.log
    }

    /// The command results the plane holds (STATUS 0x16).
    pub fn command_results(&self) -> impl Iterator<Item = &crate::events::Held> {
        self.results.iter()
    }

    /// True while INT_OUT should be high: unacknowledged events exist
    /// (ADR 0002 decision 8).
    pub fn event_pending(&self) -> bool {
        self.events.pending()
    }

    /// True once, after a full poll cycle in which the sequencer also
    /// ran (ADR 0002, "What an expert would ask": the watchdog is fed
    /// only from the bus task, so a hung sequencer stalls the cycle and
    /// the watchdog bites).
    ///
    /// Reading it clears it, so a firmware that calls it twice per cycle
    /// does not feed twice.
    pub fn may_feed_watchdog(&mut self) -> bool {
        core::mem::take(&mut self.watchdog_credit)
    }

    /// True while the firmware should reset the plane: a commanded
    /// reboot, or a probation that ran out. The intended state is
    /// already in the backup registers when this turns true.
    pub const fn reset_requested(&self) -> bool {
        self.reset_request
    }

    /// What LED_RUN should be doing.
    pub fn led_pattern(&self) -> LedPattern {
        if self.update.in_progress() {
            LedPattern::Fast
        } else if self.update.unconfirmed() {
            LedPattern::Double
        } else {
            LedPattern::Heartbeat
        }
    }

    /// The plane flags of STATUS 0x13, derived rather than stored so
    /// they cannot disagree with the model.
    pub fn status_flags(&self) -> StatusFlags {
        let mut f = StatusFlags::NONE;
        if self.slots.iter().any(|s| s.in_fault()) {
            f |= StatusFlags::ANY_SLOT_FAULT;
        }
        if self.units.iter().any(|u| u.in_fault()) {
            f |= StatusFlags::ANY_UNIT_FAULT;
        }
        if self.events.pending() {
            f |= StatusFlags::EVENTS_PENDING;
        }
        if self.any_sequence_running() {
            f |= StatusFlags::SEQUENCE_RUNNING;
        }
        if self.config_dirty() {
            f |= StatusFlags::CONFIG_DIRTY;
        }
        if self.update.in_progress() {
            f |= StatusFlags::UPDATE_IN_PROGRESS;
        }
        if self.update.unconfirmed() {
            f |= StatusFlags::UNCONFIRMED_IMAGE;
        }
        if self.intended_state_lost {
            f |= StatusFlags::INTENDED_STATE_LOST;
        }
        f
    }

    /// The image state of INFO 0x0A.
    pub fn image_state(&self) -> ImageState {
        self.update.image_state()
    }

    /// The plane type of INFO 0x02.
    pub const fn plane_type(&self) -> PlaneType {
        PlaneType::Pp48Chained
    }

    /// True while the config in RAM differs from the stored set.
    pub fn config_dirty(&self) -> bool {
        self.stored.is_none_or(|s| s.config != self.config)
    }

    /// The config hash of INFO 0x09 and CONFIG 0x80.
    pub fn config_hash(&self) -> u32 {
        self.config.hash()
    }

    /// Units present, as the bitmap of INFO 0x07.
    pub fn units_present(&self) -> granite_proto::IndexBitmap {
        let mut map = granite_proto::IndexBitmap::NONE;
        for (i, unit) in self.units.iter().enumerate() {
            if unit.present() {
                map = map.plus(i as u8);
            }
        }
        map
    }

    /// Raise an event at the plane's current uptime.
    pub(crate) fn raise(&mut self, target: EventTarget, kind: EventKind) {
        self.events.push(target, kind, self.uptime_s());
    }

    /// Write a log line at the plane's current uptime.
    pub(crate) fn note(&mut self, level: u8, message: &str, n: Option<u32>) {
        self.log
            .push(self.now_ms as u32, level, message, n);
    }

    /// Store the intended state in the backup registers.
    ///
    /// Called on every intended-state change, not only before a
    /// commanded reset (ADR 0002 decision 3).
    pub(crate) fn save_intended<D: Devices>(&mut self, dev: &mut D) {
        self.backup.intended = self.intended;
        self.backup.store(dev.backup());
    }

    /// Bring the plane up (ADR 0002 decision 3).
    ///
    /// In order: adopt or lose the intended state, load the config,
    /// reset the mux, then discover the units. Discovery is what
    /// releases the photoMOS outputs on every present unit before it
    /// reads anything, and what puts an OFF slot back OFF.
    ///
    /// The caller has already started the IWDG and read `cause`.
    pub fn boot<D: Devices>(&mut self, now_ms: u64, dev: &mut D, cause: u8) {
        self.now_ms = now_ms;
        self.reset_cause = cause;

        match BackupSet::load(dev.backup()) {
            Some(mut set) => {
                // A reset that kept VDD: the expanders held their
                // outputs through it, so an OFF slot never came on and
                // the set is the truth.
                set.boot_count = set.boot_count.wrapping_add(1);
                set.set_commanded_reboot(false);
                self.intended = set.intended;
                self.backup = set;
                self.intended_state_lost = false;
            }
            None => {
                // The core lost power: hardware defaults, every slot ON,
                // and the controller re-applies what it wanted.
                self.intended = Intended::hardware_defaults();
                self.backup = BackupSet {
                    intended: self.intended,
                    boot_count: 1,
                    flags: 0,
                };
                self.intended_state_lost = true;
                self.note(level::WARN, "intended state lost", None);
            }
        }
        self.backup.store(dev.backup());

        // U_OFF in the intended state is this latch and nothing else
        // (ADR 0005 decision 3): a reset that kept VDD comes back
        // holding the unit it was holding, and a core power loss - which
        // wiped the set above - comes back holding nothing, which is the
        // hardware default of a unit that retries for ever.
        for unit in 0..UNIT_COUNT {
            self.unit_latch[unit].restore(self.intended.u_off(unit as u8));
        }

        if let Some(found) = crate::config::load(dev.config_store()) {
            self.config = found.config;
            self.stored = Some(found);
        }
        for slot in 0..SLOT_COUNT {
            self.slots[slot].power_limit = self.config.power_limit[slot];
        }

        self.update.boot(now_ms, dev.update_store());
        if self.update.rolled_back() {
            self.raise(EventTarget::Plane, EventKind::RolledBack);
        }

        // Every channel off, then look for units.
        let _ = self.mux.reset(dev.bus());
        for ch in 0..SEGMENT_CHANNELS {
            self.channels[ch] = Channel::new();
        }
        self.scan_channels(dev, true);

        self.raise(EventTarget::Plane, EventKind::Boot);
        self.note(level::INFO, "boot", Some(self.backup.boot_count));
        self.next_poll_ms = now_ms;
        self.next_unit_poll_ms = now_ms;
        self.next_id_poll_ms = now_ms;
    }

    /// One step of the plane: the poll cycle, the queued writes, the
    /// sequences and the housekeeping (ADR 0002 decision 5).
    ///
    /// The firmware calls this from its bus task about every 50 ms. The
    /// order matters: samples first, so a sequence decides on fresh
    /// readings; then the writes the link queued; then the sequences,
    /// which is also what releases a press whose deadline has passed.
    pub fn tick<D: Devices>(&mut self, now_ms: u64, dev: &mut D) {
        self.now_ms = now_ms;

        if let Some(v) = dev.pins().core_input_10mv() {
            self.core_volts = v;
        }
        if let Some(t) = dev.pins().core_temperature_decidegrees() {
            self.core_temperature = t;
        }

        // Re-scan the channels that have no unit, and recover the ones
        // whose bus is stuck.
        self.scan_channels(dev, false);

        let polled = if now_ms >= self.next_poll_ms {
            self.next_poll_ms = now_ms + crate::model::POLL_PERIOD_MS as u64;
            self.poll_cycle(dev);
            true
        } else {
            false
        };

        self.run_pending_writes(dev);
        self.run_sequences(dev);
        self.drive_press_trigger(dev);
        self.update.tick(now_ms, dev.update_store(), &mut self.reset_request);

        // ADR 0002: the watchdog is fed only after a full cycle in which
        // the sequencer also ran.
        if polled {
            self.watchdog_credit = true;
        }
    }

    /// Toggle PC6 while any press runs, and never otherwise.
    fn drive_press_trigger<D: Devices>(&mut self, dev: &mut D) {
        if !self.any_press_running() {
            return;
        }
        if self.now_ms.saturating_sub(self.press_trig_ms) >= PRESS_TRIG_PERIOD_MS {
            dev.pins().toggle_press_trigger();
            self.press_trig_ms = self.now_ms;
        }
    }

    /// True while a unit is present and its slots are worth polling.
    pub(crate) fn unit_present(&self, unit: usize) -> bool {
        self.units[unit].status.contains(UnitStatus::PRESENT)
    }
}
