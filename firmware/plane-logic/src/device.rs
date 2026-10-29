//! The device traits: everything the plane logic uses to touch a board
//! (ADR 0002 decision 1, "device traits ... so the same code runs on the
//! host").
//!
//! Every trait is synchronous and takes `&mut self`. There is no `async`
//! and no clock trait: the firmware's bus task calls
//! [`crate::Plane::tick`] with a monotonic millisecond count, and the
//! host tests call it from a loop with a fake one. Nothing here knows
//! about embassy, STM32 peripherals or I2C peripherals.
//!
//! [`SegmentBus`] is the raw I2C2 side: byte transfers at a 7-bit
//! address, plus the two recovery lines that are not on the bus
//! (MUX_RST and clocking out a stuck slave). The mux itself is a driver
//! ([`crate::drivers::Mux`]) over this trait, not a trait method, so a
//! plane with a different mux is a driver change and not a firmware
//! change.

/// What a device operation can fail with.
///
/// The variants are the ones the logic branches on: a NAK means "not
/// there" (discovery), a stuck bus means "recover it" (ADR 0002 decision
/// 4), everything else is counted and retried on the next cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceError {
    /// The addressed device did not acknowledge. In discovery this is
    /// absence, not an error.
    Nak,
    /// Arbitration lost, a bus error, a timeout: counted per channel
    /// (STATUS 0x1A).
    Bus,
    /// SDA stayed low: the channel needs nine SCL clocks and a MUX_RST
    /// pulse before it is usable again.
    Stuck,
    /// Flash or backup storage refused the operation.
    Storage,
    /// An offset or length outside the region.
    OutOfRange,
}

/// Result of a device operation.
pub type DevResult<T> = Result<T, DeviceError>;

/// The plane's own I2C2 segment bus, behind the TCA9548A.
///
/// `write` and `read` are one I2C transaction each; `read` writes the
/// command bytes first and then reads with a repeated start, which is
/// what every device on a segment wants (TCA6424A command byte, INA238
/// register pointer, ADC121C027 register address). A device that answers
/// nothing takes an empty `cmd`.
pub trait SegmentBus {
    /// Write `bytes` to `addr`.
    fn write(&mut self, addr: u8, bytes: &[u8]) -> DevResult<()>;

    /// Write `cmd`, then read `out.len()` bytes from `addr`.
    fn read(&mut self, addr: u8, cmd: &[u8], out: &mut [u8]) -> DevResult<()>;

    /// Pulse MUX_RST: every mux channel goes off (ADR 0002: the core pin
    /// PB12 through Q6, so the polarity is the glue's business).
    fn reset_mux(&mut self) -> DevResult<()>;

    /// Clock out nine SCL pulses to free a slave that holds SDA low
    /// (ADR 0002 decision 4, stuck-bus recovery).
    fn unstick(&mut self) -> DevResult<()>;
}

/// The core pins and sensors that are not on the segment bus.
pub trait CorePins {
    /// Toggle PC6 PRESS_TRIG.
    ///
    /// Every edge retriggers the ~1 s monostable that feeds PRESS_EN to
    /// every unit, and without PRESS_EN no photoMOS LED can light. The
    /// plane logic calls this from the same code that owns the press
    /// deadline, about every 200 ms while a press runs, and never
    /// otherwise (ADR 0002 open questions, "press deadline line": never
    /// from a timer peripheral or DMA, those keep running in a hung
    /// firmware and would defeat the deadline).
    fn toggle_press_trigger(&mut self);

    /// Pulse PC7 EXP_RSTN (open drain, bused to every unit's TCA6424A
    /// /RESET). CTRL 0x4D only.
    fn pulse_expander_reset(&mut self);

    /// The core's 48 V input through the 220k / 10k divider \[10 mV\],
    /// or `None` while no reading is ready.
    fn core_input_10mv(&mut self) -> Option<u16>;

    /// The MCU's own temperature sensor \[0.1 C\], or `None`.
    fn core_temperature_decidegrees(&mut self) -> Option<i16>;
}

/// The TAMP backup registers: a handful of `u32` that survive any reset
/// that keeps VDD (ADR 0002 decision 3).
///
/// The logic writes the whole set on every intended-state change and
/// checks it with a magic word and a CRC-32; a core power loss wipes
/// them and the plane starts from hardware defaults with the "intended
/// state lost" flag set.
pub trait BackupRegs {
    /// Read word `index` (below [`crate::backup::BACKUP_WORDS`]).
    fn read(&self, index: usize) -> u32;

    /// Write word `index`.
    fn write(&mut self, index: usize, value: u32);
}

/// The two CONFIG pages (ADR 0002 decision 2: 2 x 2 KB, two copies with
/// a sequence number and a CRC).
///
/// The logic hands over a blob of at most [`crate::config::BLOB_BYTES`]
/// bytes and does the sequencing and the CRC itself; the store only has
/// to erase and write a page.
pub trait ConfigStore {
    /// Read copy `copy` (0 or 1) into `out`.
    fn read(&mut self, copy: u8, out: &mut [u8]) -> DevResult<()>;

    /// Erase copy `copy` and write `data` at its start.
    fn write(&mut self, copy: u8, data: &[u8]) -> DevResult<()>;
}

/// The DFU slot and the A/B swap markers (ADR 0002 decision 9).
///
/// This is embassy-boot's `FirmwareUpdater` on the board and a byte
/// buffer in the simulator. The logic never reads the image back except
/// through [`UpdateStore::crc32_dfu`], so a store may compute that in
/// hardware.
pub trait UpdateStore {
    /// Bytes the DFU slot holds.
    fn dfu_capacity(&self) -> u32;

    /// Erase the whole DFU slot.
    fn erase_dfu(&mut self) -> DevResult<()>;

    /// Write `data` at `offset` in the DFU slot.
    fn write_dfu(&mut self, offset: u32, data: &[u8]) -> DevResult<()>;

    /// CRC-32 (IEEE, as [`crate::crc32::crc32`]) over the first `len`
    /// bytes of the DFU slot.
    fn crc32_dfu(&mut self, len: u32) -> DevResult<u32>;

    /// Mark the swap: the next boot runs the DFU image unconfirmed.
    fn mark_swap(&mut self) -> DevResult<()>;

    /// Mark the running image booted: its probation ends.
    fn mark_booted(&mut self) -> DevResult<()>;

    /// True while the running image is unconfirmed (INFO 0x0A
    /// probation).
    fn image_unconfirmed(&self) -> bool;

    /// True when the bootloader rolled back to the previous image.
    fn rolled_back(&self) -> bool;
}

/// Everything the plane logic needs, in one place.
///
/// The firmware builds one of these around its peripherals (see
/// [`DeviceSet`]) and hands it to [`crate::Plane::boot`] and
/// [`crate::Plane::tick`]. Passing it per call rather than owning it
/// keeps `Plane` free of the peripheral types, so `size_of::<Plane>()`
/// is the model and nothing else.
pub trait Devices {
    /// The segment bus behind the mux.
    type Bus: SegmentBus;
    /// The core pins and sensors.
    type Pins: CorePins;
    /// The TAMP backup registers.
    type Backup: BackupRegs;
    /// The CONFIG pages.
    type Config: ConfigStore;
    /// The DFU slot.
    type Update: UpdateStore;

    /// The segment bus.
    fn bus(&mut self) -> &mut Self::Bus;
    /// The core pins.
    fn pins(&mut self) -> &mut Self::Pins;
    /// The backup registers.
    fn backup(&mut self) -> &mut Self::Backup;
    /// The config store.
    fn config_store(&mut self) -> &mut Self::Config;
    /// The firmware update store.
    fn update_store(&mut self) -> &mut Self::Update;
}

/// A [`Devices`] made of five owned parts, so the firmware glue is a
/// constructor and nothing else.
#[derive(Debug)]
pub struct DeviceSet<B, P, K, C, U> {
    /// The segment bus.
    pub bus: B,
    /// The core pins.
    pub pins: P,
    /// The backup registers.
    pub backup: K,
    /// The config store.
    pub config: C,
    /// The firmware update store.
    pub update: U,
}

impl<B, P, K, C, U> DeviceSet<B, P, K, C, U> {
    /// Collect five devices.
    pub const fn new(bus: B, pins: P, backup: K, config: C, update: U) -> Self {
        DeviceSet {
            bus,
            pins,
            backup,
            config,
            update,
        }
    }
}

impl<B, P, K, C, U> Devices for DeviceSet<B, P, K, C, U>
where
    B: SegmentBus,
    P: CorePins,
    K: BackupRegs,
    C: ConfigStore,
    U: UpdateStore,
{
    type Bus = B;
    type Pins = P;
    type Backup = K;
    type Config = C;
    type Update = U;

    fn bus(&mut self) -> &mut B {
        &mut self.bus
    }

    fn pins(&mut self) -> &mut P {
        &mut self.pins
    }

    fn backup(&mut self) -> &mut K {
        &mut self.backup
    }

    fn config_store(&mut self) -> &mut C {
        &mut self.config
    }

    fn update_store(&mut self) -> &mut U {
        &mut self.update
    }
}
