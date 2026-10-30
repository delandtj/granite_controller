//! CTRL block records (tags 0x40-0x51): the commands.
//!
//! Every CTRL write starts with its sequence number (0x4F) and then
//! carries one or more commands. The plane remembers the last
//! [`crate::SEQ_MEMORY`] sequence numbers: a write with a number it has
//! seen is not executed again, only its result kept, so a controller
//! that did not see a result may resend the identical frame
//! (ADR 0003 decisions 3 and 4).
//!
//! Unlike a read, an unknown tag in a CTRL write is an error for that
//! record ([`ResultCode::UnknownTag`]), so [`CtrlWrite::commands`]
//! reports unknown tags instead of skipping them.

use crate::bits::IndexBitmap;
use crate::config::RetryPolicy;
use crate::record::{RecordError, RecordWriter, Records, as_u8, as_u32, fixed, u16_at};
use crate::tags::*;

/// One CTRL record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// 0x4F the write's sequence number: the first record of a CTRL
    /// write, not a command of its own.
    Seq(u8),
    /// 0x40 enable (release OFF) or disable (assert OFF) a slot's rail.
    Enable {
        /// Slot 0-7.
        slot: u8,
        /// True releases OFF.
        on: bool,
    },
    /// 0x41 clear a slot's overvoltage latch. Never automatic.
    ClearOvLatch {
        /// Slot 0-7.
        slot: u8,
    },
    /// 0x42 set a slot's retry policy (the same fields as the config
    /// record, applied at once and not stored).
    RetryPolicy(RetryPolicy),
    /// 0x44 clear a slot's power min / max, or every slot's with
    /// [`SLOT_ALL`].
    ClearMinMax {
        /// Slot 0-7, or [`SLOT_ALL`].
        slot: u8,
    },
    /// 0x45 hold PWR for `ms`.
    PressPwr {
        /// Slot 0-7.
        slot: u8,
        /// Hold time \[ms\], [`PRESS_MS_MIN`] to [`PRESS_MS_MAX`].
        ms: u16,
    },
    /// 0x46 hold RST for `ms`.
    PressRst {
        /// Slot 0-7.
        slot: u8,
        /// Hold time \[ms\], [`PRESS_MS_MIN`] to [`PRESS_MS_MAX`].
        ms: u16,
    },
    /// 0x47 power-cycle a slot: rail off, wait, rail on, and press PWR
    /// if the host stays dark.
    PowerCycle {
        /// Slot 0-7.
        slot: u8,
        /// How long the rail stays off \[s\] (the ADR's default unit).
        off_delay_s: u16,
        /// Press PWR when the host LED stays dark.
        press_if_dark: bool,
        /// How long to wait for the host LED \[s\].
        led_timeout_s: u16,
    },
    /// 0x48 abort the sequence running on a slot.
    AbortSequence {
        /// Slot 0-7.
        slot: u8,
    },
    /// 0x49 module run: release or assert RUNOFF. The plane refuses it
    /// without DISARM ([`ResultCode::Refused`], ADR 0002 decision 6).
    ModuleRun {
        /// Slot 0-7.
        slot: u8,
        /// True runs the module.
        on: bool,
    },
    /// 0x4A let the plane's hold on a unit's hot-swap go.
    ///
    /// The part itself (LM5069-2) has no latch to re-arm and U_OFF is
    /// not pulsed, so a unit that is running keeps its two nodes. The
    /// result is asynchronous: `ok` when the unit carries power again,
    /// `failed` when the fault is still there.
    UnitHotSwapReset {
        /// Unit 0-3.
        unit: u8,
    },
    /// 0x4B switch several slots on, `delay_ms` apart.
    StaggerOn {
        /// Bit n = slot n.
        slots: IndexBitmap,
        /// Delay between slots \[ms\].
        delay_ms: u16,
    },
    /// 0x4C reboot the plane (carries [`REBOOT_MAGIC`]).
    Reboot,
    /// 0x4D pulse EXP_RSTN (carries [`RESET_EXPANDERS_MAGIC`]). The
    /// line is bused: every unit's expander resets at once, so every
    /// slot goes ON before the plane re-applies its state. Recovery for
    /// a wedged expander only.
    ResetExpanders,
    /// 0x50 node on: rail on if it is off, then a PWR press if the host
    /// LED stays dark (controller ADR 0004 verb `on`).
    NodeOn {
        /// Slot 0-7.
        slot: u8,
        /// How long to wait for the host LED \[s\].
        led_timeout_s: u16,
    },
    /// 0x51 node off: a short PWR press, wait for the LED to go dark,
    /// then escalate (controller ADR 0004 verbs `off` and
    /// `force_off`). `timeout_s` 0 skips the short press and escalates
    /// at once.
    NodeOff {
        /// Slot 0-7.
        slot: u8,
        /// How long to wait for the LED to go dark \[s\]; 0 escalates
        /// at once.
        timeout_s: u16,
        /// What to do when the host stays up.
        escalate: Escalate,
    },
}

impl Command {
    /// The record's tag.
    pub const fn tag(&self) -> u8 {
        match self {
            Command::Seq(_) => TAG_COMMAND_SEQ,
            Command::Enable { .. } => TAG_ENABLE,
            Command::ClearOvLatch { .. } => TAG_CLEAR_OV_LATCH,
            Command::RetryPolicy(_) => TAG_RETRY_POLICY,
            Command::ClearMinMax { .. } => TAG_CLEAR_MIN_MAX,
            Command::PressPwr { .. } => TAG_PRESS_PWR,
            Command::PressRst { .. } => TAG_PRESS_RST,
            Command::PowerCycle { .. } => TAG_POWER_CYCLE,
            Command::AbortSequence { .. } => TAG_ABORT_SEQUENCE,
            Command::ModuleRun { .. } => TAG_MODULE_RUN,
            Command::UnitHotSwapReset { .. } => TAG_UNIT_HOT_SWAP_RESET,
            Command::StaggerOn { .. } => TAG_STAGGER_ON,
            Command::Reboot => TAG_REBOOT,
            Command::ResetExpanders => TAG_RESET_EXPANDERS,
            Command::NodeOn { .. } => TAG_NODE_ON,
            Command::NodeOff { .. } => TAG_NODE_OFF,
        }
    }

    /// The slot this command acts on, if it names one ([`SLOT_ALL`]
    /// included).
    pub const fn slot(&self) -> Option<u8> {
        match self {
            Command::Enable { slot, .. }
            | Command::ClearOvLatch { slot }
            | Command::ClearMinMax { slot }
            | Command::PressPwr { slot, .. }
            | Command::PressRst { slot, .. }
            | Command::PowerCycle { slot, .. }
            | Command::AbortSequence { slot }
            | Command::ModuleRun { slot, .. }
            | Command::NodeOn { slot, .. }
            | Command::NodeOff { slot, .. } => Some(*slot),
            Command::RetryPolicy(p) => Some(p.slot),
            _ => None,
        }
    }

    /// True for a command the plane runs as a sequence: it answers
    /// [`ResultCode::Accepted`] first and a second result later.
    pub const fn is_sequence(&self) -> bool {
        matches!(
            self,
            Command::PressPwr { .. }
                | Command::PressRst { .. }
                | Command::PowerCycle { .. }
                | Command::StaggerOn { .. }
                | Command::NodeOn { .. }
                | Command::NodeOff { .. }
        )
    }

    /// Append the record to a payload.
    pub fn encode(&self, w: &mut RecordWriter) -> Result<(), RecordError> {
        let tag = self.tag();
        match self {
            Command::Seq(seq) => w.push(tag, &[*seq]),
            Command::Enable { slot, on } | Command::ModuleRun { slot, on } => {
                crate::check_slot(tag, *slot)?;
                w.push(tag, &[*slot, u8::from(*on)])
            }
            Command::ClearOvLatch { slot } | Command::AbortSequence { slot } => {
                crate::check_slot(tag, *slot)?;
                w.push(tag, &[*slot])
            }
            Command::RetryPolicy(p) => p.encode_as(tag, w),
            Command::ClearMinMax { slot } => {
                if *slot != SLOT_ALL {
                    crate::check_slot(tag, *slot)?;
                }
                w.push(tag, &[*slot])
            }
            Command::PressPwr { slot, ms } | Command::PressRst { slot, ms } => {
                crate::check_slot(tag, *slot)?;
                check_press(tag, *ms)?;
                let m = ms.to_le_bytes();
                w.push_parts(tag, &[&[*slot], &m])
            }
            Command::PowerCycle {
                slot,
                off_delay_s,
                press_if_dark,
                led_timeout_s,
            } => {
                crate::check_slot(tag, *slot)?;
                let mut v = [0u8; 6];
                v[0] = *slot;
                v[1..3].copy_from_slice(&off_delay_s.to_le_bytes());
                v[3] = u8::from(*press_if_dark);
                v[4..6].copy_from_slice(&led_timeout_s.to_le_bytes());
                w.push(tag, &v)
            }
            Command::UnitHotSwapReset { unit } => {
                crate::check_unit(tag, *unit)?;
                w.push(tag, &[*unit])
            }
            Command::StaggerOn { slots, delay_ms } => {
                let d = delay_ms.to_le_bytes();
                w.push_parts(tag, &[&[slots.bits()], &d])
            }
            Command::Reboot => w.push(tag, &REBOOT_MAGIC.to_le_bytes()),
            Command::ResetExpanders => w.push(tag, &RESET_EXPANDERS_MAGIC.to_le_bytes()),
            Command::NodeOn {
                slot,
                led_timeout_s,
            } => {
                crate::check_slot(tag, *slot)?;
                let t = led_timeout_s.to_le_bytes();
                w.push_parts(tag, &[&[*slot], &t])
            }
            Command::NodeOff {
                slot,
                timeout_s,
                escalate,
            } => {
                crate::check_slot(tag, *slot)?;
                let t = timeout_s.to_le_bytes();
                w.push_parts(tag, &[&[*slot], &t, &[escalate.value()]])
            }
        }
    }

    /// Read one command record.
    ///
    /// A tag outside the CTRL table gives [`RecordError::Unknown`]
    /// (answer [`ResultCode::UnknownTag`]); a field out of range gives
    /// [`RecordError::BadValue`] (answer [`ResultCode::BadArgument`]).
    pub fn decode(tag: u8, value: &[u8]) -> Result<Self, RecordError> {
        Ok(match tag {
            TAG_COMMAND_SEQ => Command::Seq(as_u8(tag, value)?),
            TAG_ENABLE => {
                let (slot, on) = slot_flag(tag, value)?;
                Command::Enable { slot, on }
            }
            TAG_CLEAR_OV_LATCH => Command::ClearOvLatch {
                slot: slot_only(tag, value)?,
            },
            TAG_RETRY_POLICY => Command::RetryPolicy(RetryPolicy::decode(value)?),
            TAG_CLEAR_MIN_MAX => {
                let slot = as_u8(tag, value)?;
                if slot != SLOT_ALL {
                    crate::check_slot(tag, slot)?;
                }
                Command::ClearMinMax { slot }
            }
            TAG_PRESS_PWR | TAG_PRESS_RST => {
                let v = fixed::<3>(tag, value)?;
                crate::check_slot(tag, v[0])?;
                let ms = u16_at(&v, 1);
                check_press(tag, ms)?;
                if tag == TAG_PRESS_PWR {
                    Command::PressPwr { slot: v[0], ms }
                } else {
                    Command::PressRst { slot: v[0], ms }
                }
            }
            TAG_POWER_CYCLE => {
                let v = fixed::<6>(tag, value)?;
                crate::check_slot(tag, v[0])?;
                Command::PowerCycle {
                    slot: v[0],
                    off_delay_s: u16_at(&v, 1),
                    press_if_dark: flag(tag, v[3])?,
                    led_timeout_s: u16_at(&v, 4),
                }
            }
            TAG_ABORT_SEQUENCE => Command::AbortSequence {
                slot: slot_only(tag, value)?,
            },
            TAG_MODULE_RUN => {
                let (slot, on) = slot_flag(tag, value)?;
                Command::ModuleRun { slot, on }
            }
            TAG_UNIT_HOT_SWAP_RESET => {
                let unit = as_u8(tag, value)?;
                crate::check_unit(tag, unit)?;
                Command::UnitHotSwapReset { unit }
            }
            TAG_STAGGER_ON => {
                let v = fixed::<3>(tag, value)?;
                Command::StaggerOn {
                    slots: IndexBitmap::from_bits(v[0]),
                    delay_ms: u16_at(&v, 1),
                }
            }
            TAG_REBOOT => {
                check_magic(tag, value, REBOOT_MAGIC)?;
                Command::Reboot
            }
            TAG_RESET_EXPANDERS => {
                check_magic(tag, value, RESET_EXPANDERS_MAGIC)?;
                Command::ResetExpanders
            }
            TAG_NODE_ON => {
                let v = fixed::<3>(tag, value)?;
                crate::check_slot(tag, v[0])?;
                Command::NodeOn {
                    slot: v[0],
                    led_timeout_s: u16_at(&v, 1),
                }
            }
            TAG_NODE_OFF => {
                let v = fixed::<4>(tag, value)?;
                crate::check_slot(tag, v[0])?;
                Command::NodeOff {
                    slot: v[0],
                    timeout_s: u16_at(&v, 1),
                    escalate: Escalate::try_from(v[3])
                        .map_err(|_| RecordError::BadValue { tag })?,
                }
            }
            _ => return Err(RecordError::Unknown { tag }),
        })
    }

    /// Build a whole CTRL payload: the 0x4F sequence record, then the
    /// commands in order.
    pub fn write_ctrl(
        w: &mut RecordWriter,
        seq: u8,
        commands: &[Command],
    ) -> Result<(), RecordError> {
        Command::Seq(seq).encode(w)?;
        for c in commands {
            if let Command::Seq(_) = c {
                continue;
            }
            c.encode(w)?;
        }
        Ok(())
    }
}

/// A CTRL payload the plane received: its sequence number and the
/// commands after it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CtrlWrite<'a> {
    /// The write's sequence number (its 0x4F record).
    pub seq: u8,
    /// The payload bytes after the sequence record.
    rest: &'a [u8],
}

impl<'a> CtrlWrite<'a> {
    /// Read the sequence record a CTRL write starts with.
    ///
    /// Gives [`RecordError::NoSeq`] when the first record is not 0x4F:
    /// the plane cannot answer a write it cannot name, so it drops it
    /// and counts it.
    pub fn parse(payload: &'a [u8]) -> Result<Self, RecordError> {
        let mut records = Records::new(payload);
        match records.next() {
            Some(Ok((TAG_COMMAND_SEQ, value))) => Ok(CtrlWrite {
                seq: as_u8(TAG_COMMAND_SEQ, value)?,
                rest: records.rest(),
            }),
            Some(Err(e)) => Err(e),
            _ => Err(RecordError::NoSeq),
        }
    }

    /// The commands after the sequence record, in wire order. Unknown
    /// tags are reported, not skipped.
    pub fn commands(&self) -> impl Iterator<Item = Result<Command, RecordError>> + 'a {
        Records::new(self.rest).map(|r| match r {
            Ok((tag, value)) => Command::decode(tag, value),
            Err(e) => Err(e),
        })
    }
}

/// Press duration inside the ADR's 50-10000 ms window.
fn check_press(tag: u8, ms: u16) -> Result<(), RecordError> {
    if (PRESS_MS_MIN..=PRESS_MS_MAX).contains(&ms) {
        Ok(())
    } else {
        Err(RecordError::BadValue { tag })
    }
}

/// A 0/1 flag byte.
fn flag(tag: u8, byte: u8) -> Result<bool, RecordError> {
    match byte {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(RecordError::BadValue { tag }),
    }
}

/// A \[slot\] value.
fn slot_only(tag: u8, value: &[u8]) -> Result<u8, RecordError> {
    let slot = as_u8(tag, value)?;
    crate::check_slot(tag, slot)?;
    Ok(slot)
}

/// A \[slot, 0/1\] value.
fn slot_flag(tag: u8, value: &[u8]) -> Result<(u8, bool), RecordError> {
    let v = fixed::<2>(tag, value)?;
    crate::check_slot(tag, v[0])?;
    Ok((v[0], flag(tag, v[1])?))
}

/// A \[u32 magic\] value that must be exactly `want`.
fn check_magic(tag: u8, value: &[u8], want: u32) -> Result<(), RecordError> {
    if as_u32(tag, value)? == want {
        Ok(())
    } else {
        Err(RecordError::BadValue { tag })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ctrl_write_round_trip() {
        let commands = [
            Command::NodeOff {
                slot: 3,
                timeout_s: 30,
                escalate: Escalate::HoldPwrThenCutRail,
            },
            Command::Enable { slot: 4, on: true },
        ];
        let mut buf = [0u8; 64];
        let mut w = RecordWriter::new(&mut buf);
        Command::write_ctrl(&mut w, 0x11, &commands).unwrap();
        let parsed = CtrlWrite::parse(w.payload()).unwrap();
        assert_eq!(parsed.seq, 0x11);
        let mut n = 0;
        for (got, want) in parsed.commands().zip(commands.iter()) {
            assert_eq!(got, Ok(*want));
            n += 1;
        }
        assert_eq!(n, commands.len());
    }

    #[test]
    fn a_ctrl_write_without_a_seq_is_refused() {
        let mut buf = [0u8; 16];
        let mut w = RecordWriter::new(&mut buf);
        Command::Enable { slot: 0, on: true }
            .encode(&mut w)
            .unwrap();
        assert_eq!(CtrlWrite::parse(w.payload()), Err(RecordError::NoSeq));
        assert_eq!(CtrlWrite::parse(&[]), Err(RecordError::NoSeq));
    }

    #[test]
    fn unknown_ctrl_tags_are_reported() {
        let mut buf = [0u8; 16];
        let mut w = RecordWriter::new(&mut buf);
        Command::Seq(1).encode(&mut w).unwrap();
        w.push(0x5E, &[0x00]).unwrap();
        let parsed = CtrlWrite::parse(w.payload()).unwrap();
        assert_eq!(
            parsed.commands().next(),
            Some(Err(RecordError::Unknown { tag: 0x5E }))
        );
    }

    #[test]
    fn out_of_range_fields_are_refused() {
        let mut buf = [0u8; 16];
        let mut w = RecordWriter::new(&mut buf);
        assert_eq!(
            Command::PressPwr { slot: 0, ms: 10 }.encode(&mut w),
            Err(RecordError::BadValue { tag: TAG_PRESS_PWR })
        );
        assert_eq!(
            Command::PressPwr {
                slot: 0,
                ms: 20_000
            }
            .encode(&mut w),
            Err(RecordError::BadValue { tag: TAG_PRESS_PWR })
        );
        assert_eq!(
            Command::Enable { slot: 8, on: true }.encode(&mut w),
            Err(RecordError::BadValue { tag: TAG_ENABLE })
        );
        assert_eq!(
            Command::UnitHotSwapReset { unit: 4 }.encode(&mut w),
            Err(RecordError::BadValue {
                tag: TAG_UNIT_HOT_SWAP_RESET
            })
        );
        assert!(w.is_empty());
        assert_eq!(
            Command::decode(TAG_ENABLE, &[0, 2]),
            Err(RecordError::BadValue { tag: TAG_ENABLE })
        );
        assert_eq!(
            Command::decode(TAG_REBOOT, &0u32.to_le_bytes()),
            Err(RecordError::BadValue { tag: TAG_REBOOT })
        );
    }

    #[test]
    fn clear_min_max_takes_every_slot() {
        let mut buf = [0u8; 8];
        let mut w = RecordWriter::new(&mut buf);
        Command::ClearMinMax { slot: SLOT_ALL }
            .encode(&mut w)
            .unwrap();
        assert_eq!(w.payload(), &[0x44, 1, 0xFF]);
        assert_eq!(
            Command::decode(TAG_CLEAR_MIN_MAX, &[0xFF]),
            Ok(Command::ClearMinMax { slot: SLOT_ALL })
        );
        assert_eq!(
            Command::decode(TAG_CLEAR_MIN_MAX, &[8]),
            Err(RecordError::BadValue {
                tag: TAG_CLEAR_MIN_MAX
            })
        );
    }

    #[test]
    fn sequences_are_marked() {
        assert!(
            Command::NodeOn {
                slot: 0,
                led_timeout_s: 30
            }
            .is_sequence()
        );
        assert!(!Command::Enable { slot: 0, on: true }.is_sequence());
        assert_eq!(
            Command::NodeOff {
                slot: 6,
                timeout_s: 0,
                escalate: Escalate::None
            }
            .slot(),
            Some(6)
        );
        assert_eq!(Command::Reboot.slot(), None);
    }
}
