//! Block ids, tag numbers and the small wire enums of ADR 0003
//! decisions 2 and 3.
//!
//! Tags are global: a tag number means one thing on the whole link and
//! never changes meaning. They freeze at the first plane shipped to a
//! site, so a mistake stays here as a reserved number.

/// A value a wire enum has no name for in this build.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnknownValue(pub u16);

/// Define an enum over a wire value with `TryFrom`.
macro_rules! wire_enum {
    (
        $(#[$outer:meta])*
        $name:ident($ty:ty) {
            $( $(#[$inner:meta])* $variant:ident = $value:expr, )*
        }
    ) => {
        $(#[$outer])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        #[repr($ty)]
        pub enum $name {
            $( $(#[$inner])* $variant = $value, )*
        }

        impl $name {
            /// The wire value.
            pub const fn value(self) -> $ty {
                self as $ty
            }

            /// The variant for a wire value, if this build knows it.
            pub const fn from_value(value: $ty) -> Option<Self> {
                match value {
                    $( $value => Some($name::$variant), )*
                    _ => None,
                }
            }
        }

        impl TryFrom<$ty> for $name {
            type Error = UnknownValue;

            fn try_from(value: $ty) -> Result<Self, UnknownValue> {
                Self::from_value(value).ok_or(UnknownValue(value as u16))
            }
        }

        impl From<$name> for $ty {
            fn from(v: $name) -> $ty {
                v.value()
            }
        }
    };
}

// --- Block ids (ADR 0003 decision 2) ---------------------------------

/// INFO: identity, versions, topology, config hash, image state.
pub const BLK_INFO: u8 = 0x01;
/// STATUS: bus, flags, last command results, event head, counters.
pub const BLK_STATUS: u8 = 0x02;
/// SUMMARY: one compact record per present slot (the 1 Hz poll).
pub const BLK_SUMMARY: u8 = 0x03;
/// UNIT: one unit, index 0-3.
pub const BLK_UNIT: u8 = 0x04;
/// SLOT: one slot, index 0-7.
pub const BLK_SLOT: u8 = 0x05;
/// EVENTS: oldest unacknowledged events, up to `L`.
pub const BLK_EVENTS: u8 = 0x06;
/// CONFIG: the current plane config.
pub const BLK_CONFIG: u8 = 0x07;
/// LOG: recent log lines from the RAM ring.
pub const BLK_LOG: u8 = 0x08;
/// CTRL: commands, write only.
pub const BLK_CTRL: u8 = 0x10;
/// EV_ACK: acknowledge events up to a sequence number, write only.
pub const BLK_EV_ACK: u8 = 0x11;
/// CONFIG_W: config records, applied and stored as one set.
pub const BLK_CONFIG_W: u8 = 0x12;
/// FW: firmware update records, write only.
pub const BLK_FW: u8 = 0x13;

/// Index of a block that has only one instance.
pub const INDEX_NONE: u8 = 0xFF;

wire_enum! {
    /// The blocks of decision 2.
    Block(u8) {
        /// [`BLK_INFO`].
        Info = 0x01,
        /// [`BLK_STATUS`].
        Status = 0x02,
        /// [`BLK_SUMMARY`].
        Summary = 0x03,
        /// [`BLK_UNIT`], index 0-3.
        Unit = 0x04,
        /// [`BLK_SLOT`], index 0-7.
        Slot = 0x05,
        /// [`BLK_EVENTS`].
        Events = 0x06,
        /// [`BLK_CONFIG`].
        Config = 0x07,
        /// [`BLK_LOG`].
        Log = 0x08,
        /// [`BLK_CTRL`], write only.
        Ctrl = 0x10,
        /// [`BLK_EV_ACK`], write only.
        EvAck = 0x11,
        /// [`BLK_CONFIG_W`], write only.
        ConfigW = 0x12,
        /// [`BLK_FW`], write only.
        Fw = 0x13,
    }
}

impl Block {
    /// True for a block the controller reads.
    pub const fn is_read(self) -> bool {
        self.value() < BLK_CTRL
    }

    /// True for a block the controller writes.
    pub const fn is_write(self) -> bool {
        !self.is_read()
    }

    /// How many indices the block has: 4 for UNIT, 8 for SLOT, 1 (and
    /// [`INDEX_NONE`] on the wire) for the rest.
    pub const fn index_count(self) -> u8 {
        match self {
            Block::Unit => crate::UNIT_COUNT as u8,
            Block::Slot => crate::SLOT_COUNT as u8,
            _ => 1,
        }
    }

    /// True when `index` is the index this block expects.
    pub const fn accepts_index(self, index: u8) -> bool {
        match self {
            Block::Unit | Block::Slot => index < self.index_count(),
            _ => index == INDEX_NONE,
        }
    }
}

// --- INFO tags (0x01-0x0F) -------------------------------------------

/// Format version of this protocol \[u8\]; see [`crate::FORMAT_VERSION`].
pub const TAG_FORMAT_VERSION: u8 = 0x01;
/// Plane type \[u16\]; see [`PlaneType`].
pub const TAG_PLANE_TYPE: u8 = 0x02;
/// Slot capacity \[u8\] (8 on the chained plane).
pub const TAG_SLOT_CAPACITY: u8 = 0x03;
/// Firmware version \[u8 x3\].
pub const TAG_FIRMWARE_VERSION: u8 = 0x04;
/// Serial \[u32\]: a hash of the STM32 UID.
pub const TAG_SERIAL: u8 = 0x05;
/// Capabilities \[u32\]; see [`crate::Capabilities`].
pub const TAG_CAPABILITIES: u8 = 0x06;
/// Units present \[u8 bitmap\], bit n = unit n.
pub const TAG_UNITS_PRESENT: u8 = 0x07;
/// Boot count \[u32\].
pub const TAG_BOOT_COUNT: u8 = 0x08;
/// Config hash \[u32\] (CRC-32 of the stored set).
pub const TAG_CONFIG_HASH: u8 = 0x09;
/// Image state \[u8\]; see [`ImageState`].
pub const TAG_IMAGE_STATE: u8 = 0x0A;
/// Bootloader version \[u8 x3\].
pub const TAG_BOOTLOADER_VERSION: u8 = 0x0B;

// --- STATUS tags (0x10-0x1B) -----------------------------------------

/// Core input voltage \[u16, 10 mV\].
pub const TAG_CORE_INPUT_V: u8 = 0x10;
/// Core temperature \[i16, 0.1 C\] from the MCU sensor.
pub const TAG_CORE_TEMPERATURE: u8 = 0x11;
/// Uptime \[u32 s\].
pub const TAG_UPTIME: u8 = 0x12;
/// Plane flags \[u16\]; see [`crate::StatusFlags`].
pub const TAG_FLAGS: u8 = 0x13;
/// Reserved: was the aux 12 V monitor, no such sensor exists.
pub const TAG_RESERVED_14: u8 = 0x14;
/// Reserved: was the aux 12 V monitor, no such sensor exists.
pub const TAG_RESERVED_15: u8 = 0x15;
/// Command result \[u8 seq, u8 code\], one per held command.
pub const TAG_COMMAND_RESULT: u8 = 0x16;
/// Event head \[u16\]: the next sequence number the plane will use.
pub const TAG_EVENT_HEAD: u8 = 0x17;
/// Update state \[u8 state, u32 next offset\]; see [`UpdateState`].
pub const TAG_UPDATE_STATE: u8 = 0x18;
/// Link errors \[u16 bad CRC, u16 bad length\].
pub const TAG_LINK_ERRORS: u8 = 0x19;
/// Segment errors \[u16 x4\], one per segment channel.
pub const TAG_SEGMENT_ERRORS: u8 = 0x1A;
/// Last reset cause \[u8\], the plane's own code (ADR 0002: resets by
/// cause); the ADR does not enumerate it, so it stays a raw byte.
pub const TAG_LAST_RESET_CAUSE: u8 = 0x1B;

// --- SUMMARY tag (0x1C) ----------------------------------------------

/// Slot summary \[u8 slot, u16 status, u16 V, u16 I, u16 P, u8 LED\],
/// 10 bytes; see [`crate::SlotSummary`].
pub const TAG_SLOT_SUMMARY: u8 = 0x1C;

// --- SLOT tags (0x20-0x2F, 0x70) -------------------------------------

/// Slot status \[u16\]; see [`crate::SlotStatus`].
pub const TAG_SLOT_STATUS: u8 = 0x20;
/// Retry count \[u16\].
pub const TAG_RETRY_COUNT: u8 = 0x21;
/// Module class \[u8\], the plane's own code; not enumerated by the ADR.
pub const TAG_MODULE_CLASS: u8 = 0x22;
/// Module ID raw \[u16\] as read from the slot.
pub const TAG_MODULE_ID_RAW: u8 = 0x23;
/// Slot voltage \[u16, 10 mV\].
pub const TAG_SLOT_V: u8 = 0x24;
/// Slot current \[u16 mA\].
pub const TAG_SLOT_I: u8 = 0x25;
/// Slot power \[u16, 0.1 W\].
pub const TAG_SLOT_P: u8 = 0x26;
/// Slot power min / max since the last read \[u16, u16, 0.1 W\].
pub const TAG_SLOT_P_MIN_MAX: u8 = 0x27;
/// Slot temperature \[i16, 0.1 C\] from the INA238 die.
pub const TAG_SLOT_TEMPERATURE: u8 = 0x28;
/// Slot energy \[u64 J, u32 boot count\].
pub const TAG_SLOT_ENERGY: u8 = 0x29;
/// Reserved: module output V / I, not measured on this plane.
pub const TAG_RESERVED_2A: u8 = 0x2A;
/// Node LED \[u8 mode, u16 period ms\]; see [`crate::NodeLed`].
pub const TAG_NODE_LED: u8 = 0x2B;
/// Cross-check flags \[u8\]; see [`crate::CrossCheck`].
pub const TAG_CROSS_CHECK: u8 = 0x2C;
/// Sequence state \[u8\]; see [`SequenceState`].
pub const TAG_SEQUENCE_STATE: u8 = 0x2D;
/// Power limit in force \[u16, 0.1 W\].
pub const TAG_POWER_LIMIT_IN_FORCE: u8 = 0x70;

// --- UNIT tags (0x30-0x36) -------------------------------------------

/// Unit status \[u16\]; see [`crate::UnitStatus`].
pub const TAG_UNIT_STATUS: u8 = 0x30;
/// Unit input voltage \[u16, 10 mV\].
pub const TAG_UNIT_V: u8 = 0x31;
/// Unit input current \[u16 mA\].
pub const TAG_UNIT_I: u8 = 0x32;
/// Unit input power \[u16, 0.1 W\].
pub const TAG_UNIT_P: u8 = 0x33;
/// Unit input INA238 temperature \[i16, 0.1 C\].
pub const TAG_UNIT_TEMPERATURE: u8 = 0x34;
/// Unit segment errors \[u16\].
pub const TAG_UNIT_SEGMENT_ERRORS: u8 = 0x35;
/// Times the unit was lost \[u16\].
pub const TAG_UNIT_TIMES_LOST: u8 = 0x36;

// --- CTRL tags (0x40-0x51) -------------------------------------------

/// Enable \[u8 slot, u8 0/1\].
pub const TAG_ENABLE: u8 = 0x40;
/// Clear the OV latch \[u8 slot\].
pub const TAG_CLEAR_OV_LATCH: u8 = 0x41;
/// Retry policy \[u8 slot, u8 max, u16 window, u8 action\].
pub const TAG_RETRY_POLICY: u8 = 0x42;
/// Reserved: was the boot stagger delay, dropped by ADR 0002.
pub const TAG_RESERVED_43: u8 = 0x43;
/// Clear power min / max \[u8 slot or 0xFF for every slot\].
pub const TAG_CLEAR_MIN_MAX: u8 = 0x44;
/// Press PWR \[u8 slot, u16 ms 50-10000\].
pub const TAG_PRESS_PWR: u8 = 0x45;
/// Press RST \[u8 slot, u16 ms 50-10000\].
pub const TAG_PRESS_RST: u8 = 0x46;
/// Power-cycle \[u8 slot, u16 off delay, u8 press-if-dark, u16 LED
/// timeout\].
pub const TAG_POWER_CYCLE: u8 = 0x47;
/// Abort the running sequence \[u8 slot\].
pub const TAG_ABORT_SEQUENCE: u8 = 0x48;
/// Module run \[u8 slot, u8 0 = off / 1 = on\].
pub const TAG_MODULE_RUN: u8 = 0x49;
/// Unit hot-swap reset \[u8 unit\].
pub const TAG_UNIT_HOT_SWAP_RESET: u8 = 0x4A;
/// Stagger on \[u8 slot bitmap, u16 delay ms\].
pub const TAG_STAGGER_ON: u8 = 0x4B;
/// Reboot \[u32 magic\]; see [`REBOOT_MAGIC`].
pub const TAG_REBOOT: u8 = 0x4C;
/// Reset the unit expanders \[u32 magic\]; see
/// [`RESET_EXPANDERS_MAGIC`].
pub const TAG_RESET_EXPANDERS: u8 = 0x4D;
/// Command sequence number \[u8\]: the first record of every CTRL write.
pub const TAG_COMMAND_SEQ: u8 = 0x4F;
/// Node on \[u8 slot, u16 LED timeout s\].
pub const TAG_NODE_ON: u8 = 0x50;
/// Node off \[u8 slot, u16 timeout s, u8 escalate\]; see [`Escalate`].
pub const TAG_NODE_OFF: u8 = 0x51;

/// Magic of [`TAG_REBOOT`]: "RBOT" little endian.
pub const REBOOT_MAGIC: u32 = 0x5242_4F54;
/// Magic of [`TAG_RESET_EXPANDERS`]: "EXRR" little endian.
pub const RESET_EXPANDERS_MAGIC: u32 = 0x4558_5252;

/// Shortest press a slot accepts \[ms\].
pub const PRESS_MS_MIN: u16 = 50;
/// Longest press a slot accepts \[ms\].
pub const PRESS_MS_MAX: u16 = 10_000;

/// Slot value that means every slot (CTRL 0x44).
pub const SLOT_ALL: u8 = 0xFF;

// --- EVENTS tags (0x60-0x62) -----------------------------------------

/// Event \[u16 seq, u8 target, u8 kind, u32 uptime s\]; see
/// [`crate::Event`].
pub const TAG_EVENT: u8 = 0x60;
/// Events lost \[u16 count\]: the plane's event ring overflowed.
pub const TAG_EVENTS_LOST: u8 = 0x61;
/// Acknowledge events through \[u16 seq\] (EV_ACK block).
pub const TAG_ACK_THROUGH: u8 = 0x62;

// --- CONFIG tags (0x80-0x8F) -----------------------------------------

/// Config hash \[u32\], read only (CRC-32 of the stored set).
pub const TAG_CFG_HASH: u8 = 0x80;
/// Retry policy \[u8 slot, u8 max, u16 window, u8 action\].
pub const TAG_CFG_RETRY_POLICY: u8 = 0x81;
/// Power limit \[u8 slot, u16 P\].
pub const TAG_CFG_POWER_LIMIT: u8 = 0x83;
/// LED blink detection \[u16 min period ms, u16 max period ms\].
pub const TAG_CFG_LED_BLINK: u8 = 0x84;
/// Commit \[\]: the last record of a CONFIG_W set. Without it the set
/// is discarded.
pub const TAG_CFG_COMMIT: u8 = 0x8F;

// --- FW tags (0x90-0x94) ---------------------------------------------

/// Begin \[u32 size, u32 CRC-32, u8 x3 version\]: erases DFU.
pub const TAG_FW_BEGIN: u8 = 0x90;
/// Data \[u32 offset, bytes, at most 128\].
pub const TAG_FW_DATA: u8 = 0x91;
/// Commit \[u32 CRC-32\]: verifies DFU and marks the swap.
pub const TAG_FW_COMMIT: u8 = 0x92;
/// Confirm \[\]: the running image is good.
pub const TAG_FW_CONFIRM: u8 = 0x93;
/// Abort \[\].
pub const TAG_FW_ABORT: u8 = 0x94;

/// Most bytes one [`TAG_FW_DATA`] record carries.
pub const FW_DATA_MAX: usize = 128;

// --- LOG tag (0xA0) --------------------------------------------------

/// Log line \[u32 uptime ms, u8 level, text\].
pub const TAG_LOG_LINE: u8 = 0xA0;

/// Longest text one [`TAG_LOG_LINE`] record carries.
pub const LOG_TEXT_MAX: usize = 48;

// --- Wire enums ------------------------------------------------------

wire_enum! {
    /// INFO 0x02 plane type.
    PlaneType(u16) {
        /// PP-48 single board (rev1).
        Pp48Rev1 = 0x0001,
        /// PP-48 chained: core board plus 1-4 unit boards.
        Pp48Chained = 0x0002,
    }
}

wire_enum! {
    /// INFO 0x0A image state, the OTA probation ladder of ADR 0002.
    ImageState(u8) {
        /// The running image is confirmed.
        Confirmed = 0,
        /// The running image is unconfirmed and will be rolled back
        /// unless FW confirm arrives.
        Probation = 1,
        /// The bootloader rolled back to the previous image.
        RolledBack = 2,
    }
}

wire_enum! {
    /// STATUS 0x18 update state.
    ///
    /// The ADR names the field but not its values; these follow the
    /// transfer of ADR 0002 decision 9 (erase, receive, verify, swap
    /// marked) and are the plane's own progress, not a protocol
    /// contract the controller acts on beyond pacing itself.
    UpdateState(u8) {
        /// No update in progress.
        Idle = 0,
        /// Erasing the DFU slot after FW begin.
        Erasing = 1,
        /// Taking FW data records; the offset says where it is.
        Receiving = 2,
        /// Checking the CRC-32 over DFU after FW commit.
        Verifying = 3,
        /// The swap is marked; a reboot performs it.
        Ready = 4,
        /// The update failed (bad CRC, bad offset, write error).
        Failed = 5,
    }
}

wire_enum! {
    /// SLOT 0x2D per-slot sequence state.
    SequenceState(u8) {
        /// No sequence on this slot.
        Idle = 0,
        /// Waiting for the host to go dark.
        OffWait = 1,
        /// Waiting for the host to come up.
        OnWait = 2,
        /// Holding a button.
        Pressing = 3,
        /// The last sequence finished.
        Done = 4,
        /// The last sequence failed.
        Failed = 5,
    }
}

wire_enum! {
    /// SLOT 0x2B and SUMMARY 0x1C LED mode, as the plane reads the
    /// host's power LED.
    LedMode(u8) {
        /// Dark.
        Off = 0,
        /// Steady.
        On = 1,
        /// Blinking (the host sleeps); the period comes with it.
        Blinking = 2,
    }
}

wire_enum! {
    /// Result code of a command (STATUS 0x16).
    ResultCode(u8) {
        /// Done.
        Ok = 0,
        /// Accepted and running; a later result follows.
        Accepted = 1,
        /// The plane cannot take it now.
        Busy = 2,
        /// A field is out of range.
        BadArgument = 3,
        /// A sequence is already running on that slot.
        Conflict = 4,
        /// The slot or unit is not present.
        NotPresent = 5,
        /// A tag the plane does not know (CTRL writes only; a read
        /// skips unknown tags).
        UnknownTag = 6,
        /// Refused for safety (module run off without disarm).
        Refused = 7,
        /// The sequence ended in failure.
        Failed = 8,
    }
}

impl ResultCode {
    /// True while the command is still running: only [`Self::Accepted`]
    /// gets a second result.
    pub const fn is_pending(self) -> bool {
        matches!(self, ResultCode::Accepted)
    }

    /// True for a result that ends the command well.
    pub const fn is_ok(self) -> bool {
        matches!(self, ResultCode::Ok)
    }
}

wire_enum! {
    /// Event kinds (EVENTS 0x60).
    EventKind(u8) {
        /// A slot's rail came on.
        SlotOn = 1,
        /// A slot's rail went off.
        SlotOff = 2,
        /// A slot lost PGD.
        PgdLost = 3,
        /// A slot latched on overvoltage.
        OvLatched = 4,
        /// A slot tripped its power limit.
        PowerLimitTrip = 5,
        /// A slot reached its retry limit and was set OFF.
        RetryLimitReached = 6,
        /// A slot's host LED changed.
        LedChanged = 7,
        /// A sequence finished.
        SequenceDone = 8,
        /// A sequence failed.
        SequenceFailed = 9,
        /// A unit appeared.
        UnitPresent = 16,
        /// A unit stopped answering.
        UnitLost = 17,
        /// A unit came back at hardware defaults.
        UnitResetSeen = 18,
        /// A unit's hot-swap latched off.
        UnitHotSwapLatchedOff = 19,
        /// The plane booted.
        Boot = 32,
        /// A config set was stored.
        ConfigStored = 33,
        /// The update state changed.
        UpdateStateChanged = 34,
        /// The running image was confirmed.
        ImageConfirmed = 35,
        /// The bootloader rolled back.
        RolledBack = 36,
    }
}

wire_enum! {
    /// CTRL 0x51 escalation ladder of `node off` (controller ADR 0004
    /// verbs `off` and `force_off`).
    Escalate(u8) {
        /// Short press only; give up if the host stays up.
        None = 0,
        /// Hold PWR for 5 s.
        HoldPwr = 1,
        /// Cut the rail.
        CutRail = 2,
        /// Hold PWR for 5 s, then cut the rail if the LED is still on.
        HoldPwrThenCutRail = 3,
    }
}

/// What an event is about (EVENTS 0x60 target byte): a slot 0-7, a unit
/// 0x10-0x13, or the plane itself (0xFF).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EventTarget {
    /// Slot 0-7.
    Slot(u8),
    /// Unit 0-3.
    Unit(u8),
    /// The plane as a whole.
    Plane,
}

/// First unit value of an event target byte.
pub const EVENT_TARGET_UNIT_BASE: u8 = 0x10;
/// Event target byte of the plane itself.
pub const EVENT_TARGET_PLANE: u8 = 0xFF;

impl EventTarget {
    /// The target byte.
    pub const fn value(self) -> u8 {
        match self {
            EventTarget::Slot(s) => s,
            EventTarget::Unit(u) => EVENT_TARGET_UNIT_BASE + u,
            EventTarget::Plane => EVENT_TARGET_PLANE,
        }
    }

    /// The target of a byte, if it names one.
    pub const fn from_value(value: u8) -> Option<Self> {
        if value < crate::SLOT_COUNT as u8 {
            Some(EventTarget::Slot(value))
        } else if value >= EVENT_TARGET_UNIT_BASE
            && value < EVENT_TARGET_UNIT_BASE + crate::UNIT_COUNT as u8
        {
            Some(EventTarget::Unit(value - EVENT_TARGET_UNIT_BASE))
        } else if value == EVENT_TARGET_PLANE {
            Some(EventTarget::Plane)
        } else {
            None
        }
    }
}

impl TryFrom<u8> for EventTarget {
    type Error = UnknownValue;

    fn try_from(value: u8) -> Result<Self, UnknownValue> {
        Self::from_value(value).ok_or(UnknownValue(value as u16))
    }
}

impl From<EventTarget> for u8 {
    fn from(t: EventTarget) -> u8 {
        t.value()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_round_trip() {
        for raw in 0..=u8::MAX {
            match Block::try_from(raw) {
                Ok(b) => assert_eq!(b.value(), raw),
                Err(UnknownValue(v)) => assert_eq!(v, raw as u16),
            }
        }
        assert!(Block::Status.is_read() && !Block::Status.is_write());
        assert!(Block::Ctrl.is_write() && !Block::Ctrl.is_read());
        assert!(Block::Slot.accepts_index(7));
        assert!(!Block::Slot.accepts_index(8));
        assert!(!Block::Slot.accepts_index(INDEX_NONE));
        assert!(Block::Unit.accepts_index(3) && !Block::Unit.accepts_index(4));
        assert!(Block::Info.accepts_index(INDEX_NONE) && !Block::Info.accepts_index(0));
    }

    #[test]
    fn enum_values_match_the_adr() {
        assert_eq!(ResultCode::Failed.value(), 8);
        assert_eq!(EventKind::UnitHotSwapLatchedOff.value(), 19);
        assert_eq!(EventKind::RolledBack.value(), 36);
        assert_eq!(Escalate::HoldPwrThenCutRail.value(), 3);
        assert_eq!(PlaneType::Pp48Chained.value(), 2);
        assert_eq!(ImageState::try_from(3), Err(UnknownValue(3)));
        assert_eq!(EventKind::try_from(10), Err(UnknownValue(10)));
    }

    #[test]
    fn event_targets() {
        assert_eq!(EventTarget::Slot(0).value(), 0x00);
        assert_eq!(EventTarget::Slot(7).value(), 0x07);
        assert_eq!(EventTarget::Unit(3).value(), 0x13);
        assert_eq!(EventTarget::Plane.value(), 0xFF);
        for raw in 0..=u8::MAX {
            match EventTarget::from_value(raw) {
                Some(t) => assert_eq!(t.value(), raw),
                None => assert!((8..0x10).contains(&raw) || (0x14..0xFF).contains(&raw)),
            }
        }
    }

    #[test]
    fn magics_are_ascii() {
        assert_eq!(&REBOOT_MAGIC.to_be_bytes(), b"RBOT");
        assert_eq!(&RESET_EXPANDERS_MAGIC.to_be_bytes(), b"EXRR");
    }
}
