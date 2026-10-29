//! The controller link: answer every read from the model, queue every
//! write for the sequencer (ADR 0002 decision 8, ADR 0003 decisions
//! 1-5).
//!
//! A read is answered on the spot out of the model, so the answer is a
//! consistent snapshot and the plane never has to stretch the clock for
//! long. A write is checked, copied into a short queue and executed by
//! the next [`crate::Plane::tick`], because the sequencer owns the
//! segment bus; its outcome is read back under its sequence number in
//! STATUS 0x16.
//!
//! Nothing here touches the segment bus, so the firmware may call
//! [`Plane::handle_frame`] from its link task while the bus task ticks,
//! as long as the two do not run at once on the same `Plane` (one
//! `&mut`, which the firmware's task split already gives).

use granite_proto::{
    AckThrough, BLK_CONFIG, BLK_CONFIG_W, BLK_CTRL, BLK_EVENTS, BLK_EV_ACK, BLK_FW, BLK_INFO,
    BLK_LOG, BLK_SLOT, BLK_STATUS, BLK_SUMMARY, BLK_UNIT, Command, Config as CfgRecord, CtrlWrite,
    Energy, Event, EventKind, EventTarget, EventsLost, Fw, INDEX_NONE, Info, LogLine,
    MAX_PAYLOAD, NodeLed, PowerMinMax, RecordError, RecordWriter, ResultCode, SLOT_COUNT, Slot,
    SlotStatus, SlotSummary, Status, UNIT_COUNT, Unit, UnitStatus, frame,
};

use crate::device::Devices;
use crate::drivers::Ina238;
use crate::log::level;
use crate::plane::{CAPABILITIES, Plane};
use crate::sequencer::Button;

/// A write taken off the link and not executed yet.
#[derive(Debug, Clone, Copy)]
pub struct PendingWrite {
    /// The block it was written to.
    pub block: u8,
    /// Bytes of [`PendingWrite::payload`] in use.
    pub len: u8,
    /// The records, copied out of the frame the link task owns.
    pub payload: [u8; MAX_PAYLOAD],
}

impl PendingWrite {
    /// The records.
    pub fn records(&self) -> &[u8] {
        &self.payload[..self.len as usize]
    }
}

/// How bad a result code is, so a write with several commands can be
/// answered with one code: an error beats "accepted", which beats "ok".
fn severity(code: ResultCode) -> u8 {
    match code {
        ResultCode::Ok => 0,
        ResultCode::Accepted => 1,
        _ => 2,
    }
}

impl Plane {
    /// Answer or take one link frame.
    ///
    /// `frame` is what the controller wrote: two bytes are a read
    /// request, four or more a write. `out` must hold
    /// [`frame::MAX_ANSWER`] bytes. A read gives the bytes to clock back;
    /// a write gives `None`, and its outcome is read from STATUS.
    pub fn handle_frame<'o>(&mut self, bytes: &[u8], out: &'o mut [u8]) -> Option<&'o [u8]> {
        let addr = self.setup.address;
        match frame::parse_frame(addr, bytes) {
            Ok(frame::Frame::Read { block, index }) => {
                let mut payload = [0u8; MAX_PAYLOAD];
                let n = self.encode_block(block, index, &mut payload);
                frame::answer_frame(out, addr, block, index, &payload[..n]).ok()
            }
            Ok(frame::Frame::Write {
                block,
                index,
                payload,
            }) => {
                let _ = index;
                self.take_write(block, payload);
                None
            }
            Err(frame::FrameError::BadCrc) => {
                self.link_errors.bad_crc = self.link_errors.bad_crc.saturating_add(1);
                None
            }
            Err(_) => {
                self.link_errors.bad_length = self.link_errors.bad_length.saturating_add(1);
                None
            }
        }
    }

    /// Copy a write into the queue, or answer its sequence number with
    /// [`ResultCode::Busy`] when there is no room.
    fn take_write(&mut self, block: u8, payload: &[u8]) {
        if self.writes.is_full() {
            // Not executed, so a resend with the same number runs: the
            // seq memory only deduplicates writes that ran.
            if block == BLK_CTRL
                && let Ok(w) = CtrlWrite::parse(payload)
            {
                self.results.record(w.seq, ResultCode::Busy, false);
            }
            return;
        }
        let mut pending = PendingWrite {
            block,
            len: payload.len() as u8,
            payload: [0u8; MAX_PAYLOAD],
        };
        pending.payload[..payload.len()].copy_from_slice(payload);
        let _ = self.writes.push_back(pending);
    }

    /// Execute the writes the link queued (called from `tick`).
    pub(crate) fn run_pending_writes<D: Devices>(&mut self, dev: &mut D) {
        while let Some(write) = self.writes.pop_front() {
            match write.block {
                BLK_CTRL => self.exec_ctrl(dev, write.records()),
                BLK_EV_ACK => self.exec_ev_ack(write.records()),
                BLK_CONFIG_W => self.exec_config_write(dev, write.records()),
                BLK_FW => self.exec_fw(dev, write.records()),
                _ => {
                    self.link_errors.bad_length = self.link_errors.bad_length.saturating_add(1);
                }
            }
        }
    }

    // --- writes -------------------------------------------------------

    /// A CTRL write: one sequence number, one or more commands.
    fn exec_ctrl<D: Devices>(&mut self, dev: &mut D, payload: &[u8]) {
        let Ok(write) = CtrlWrite::parse(payload) else {
            // A write the plane cannot name in a result is dropped and
            // counted (ADR 0003 decision 1).
            self.link_errors.bad_length = self.link_errors.bad_length.saturating_add(1);
            return;
        };
        if self.results.is_duplicate(write.seq) {
            // Already executed: keep the result, do not act again.
            return;
        }
        let mut worst = ResultCode::Ok;
        let mut any = false;
        for record in write.commands() {
            any = true;
            let code = match record {
                Ok(command) => self.exec_command(dev, command, write.seq),
                Err(RecordError::Unknown { .. }) => ResultCode::UnknownTag,
                Err(_) => ResultCode::BadArgument,
            };
            if severity(code) >= severity(worst) {
                worst = code;
            }
        }
        if !any {
            worst = ResultCode::Ok;
        }
        // `record` keeps a result a finished sequence already wrote when
        // that one is the more final of the two.
        if !(worst == ResultCode::Accepted
            && self.results.get(write.seq).is_some_and(|h| h.executed))
        {
            self.results.record(write.seq, worst, true);
        }
    }

    /// One CTRL command.
    fn exec_command<D: Devices>(
        &mut self,
        dev: &mut D,
        command: Command,
        cmd_seq: u8,
    ) -> ResultCode {
        match command {
            Command::Seq(_) => ResultCode::Ok,
            Command::Enable { slot, on } => self.cmd_enable(dev, slot, on),
            Command::ClearOvLatch { slot } => self.cmd_clear_ov(dev, slot, cmd_seq),
            Command::RetryPolicy(p) => {
                if (p.slot as usize) >= SLOT_COUNT {
                    return ResultCode::BadArgument;
                }
                self.config.retry[p.slot as usize] = crate::config::Retry {
                    max: p.max,
                    window_s: p.window_s,
                    action: p.action,
                };
                ResultCode::Ok
            }
            Command::ClearMinMax { slot } => self.cmd_clear_min_max(slot),
            Command::PressPwr { slot, ms } => {
                self.cmd_press(dev, slot, Button::Pwr, ms, cmd_seq)
            }
            Command::PressRst { slot, ms } => {
                self.cmd_press(dev, slot, Button::Rst, ms, cmd_seq)
            }
            Command::PowerCycle {
                slot,
                off_delay_s,
                press_if_dark,
                led_timeout_s,
            } => self.cmd_power_cycle(dev, slot, off_delay_s, press_if_dark, led_timeout_s, cmd_seq),
            Command::AbortSequence { slot } => self.cmd_abort(dev, slot),
            Command::ModuleRun { slot, on } => self.cmd_module_run(dev, slot, on),
            Command::UnitHotSwapReset { unit } => self.cmd_unit_reset(dev, unit, cmd_seq),
            Command::StaggerOn { slots, delay_ms } => {
                self.cmd_stagger(dev, slots, delay_ms, cmd_seq)
            }
            Command::Reboot => self.cmd_reboot(dev),
            Command::ResetExpanders => self.cmd_reset_expanders(dev),
            Command::NodeOn {
                slot,
                led_timeout_s,
            } => self.cmd_node_on(dev, slot, led_timeout_s, cmd_seq),
            Command::NodeOff {
                slot,
                timeout_s,
                escalate,
            } => self.cmd_node_off(dev, slot, timeout_s, escalate, cmd_seq),
        }
    }

    /// An EV_ACK write.
    fn exec_ev_ack(&mut self, payload: &[u8]) {
        for record in granite_proto::Records::new(payload) {
            let Ok((tag, value)) = record else { break };
            if tag == AckThrough::TAG
                && let Ok(ack) = AckThrough::decode(value)
            {
                self.events.ack_through(ack.seq);
            }
        }
    }

    /// A CONFIG_W write: one set, applied and stored only on its commit
    /// record (ADR 0003 CONFIG 0x8F).
    ///
    /// The ADR gives CONFIG_W no sequence record, so there is no result
    /// code to answer with: a stored set raises
    /// [`EventKind::ConfigStored`] and shows up as a new config hash,
    /// and a set the plane refused simply never changes the hash.
    fn exec_config_write<D: Devices>(&mut self, dev: &mut D, payload: &[u8]) {
        let mut set = self.pending_config.unwrap_or(self.config);
        let mut commit = false;
        let mut bad = false;
        for record in CfgRecord::written(payload) {
            match record {
                Ok(CfgRecord::Commit) => commit = true,
                Ok(other) => {
                    if !set.apply(other) {
                        bad = true;
                    }
                }
                Err(_) => bad = true,
            }
        }
        if bad {
            // A set with a record the plane cannot take is discarded
            // whole: half a config is worse than none.
            self.pending_config = None;
            self.note(level::WARN, "config set refused", None);
            return;
        }
        if !commit {
            self.pending_config = Some(set);
            return;
        }
        self.pending_config = None;
        self.config = set;
        match crate::config::store(dev.config_store(), self.stored, &self.config) {
            Ok(stored) => {
                self.stored = Some(stored);
                self.apply_power_limits(dev);
                self.note(level::INFO, "config stored", Some(stored.seq));
                self.raise(EventTarget::Plane, EventKind::ConfigStored);
            }
            Err(_) => self.note(level::ERROR, "config store failed", None),
        }
    }

    /// Push the config's power limits into the present units' INA238s.
    fn apply_power_limits<D: Devices>(&mut self, dev: &mut D) {
        for unit in 0..UNIT_COUNT {
            if !self.unit_present(unit) {
                continue;
            }
            if let Err(e) = self.mux.select(dev.bus(), unit as u8) {
                self.segment_error(dev, unit, e);
                continue;
            }
            for half in 0..2usize {
                let slot = 2 * unit + half;
                let limit = self.config.power_limit[slot];
                if Ina238::slot(half == 1)
                    .set_power_limit(dev.bus(), limit)
                    .is_ok()
                {
                    self.slots[slot].power_limit = limit;
                }
            }
        }
    }

    /// An FW write.
    fn exec_fw<D: Devices>(&mut self, dev: &mut D, payload: &[u8]) {
        let before = self.update.state();
        for record in Fw::records(payload) {
            match record {
                Ok(Fw::Begin(begin)) => {
                    self.note(level::INFO, "fw begin", Some(begin.size));
                    self.update.begin(dev.update_store(), begin);
                }
                Ok(Fw::Data(data)) => {
                    self.update.data(dev.update_store(), data);
                }
                Ok(Fw::Commit { crc32 }) => {
                    if self.update.commit(dev.update_store(), crc32) {
                        self.note(level::INFO, "fw ready", None);
                    } else {
                        self.note(level::ERROR, "fw commit failed", None);
                    }
                }
                Ok(Fw::Confirm) => {
                    if self.update.confirm(dev.update_store()) {
                        self.note(level::INFO, "image confirmed", None);
                        self.raise(EventTarget::Plane, EventKind::ImageConfirmed);
                    }
                }
                Ok(Fw::Abort) => self.update.abort(),
                Err(_) => {
                    self.link_errors.bad_length =
                        self.link_errors.bad_length.saturating_add(1);
                }
            }
        }
        if self.update.state() != before {
            self.raise(EventTarget::Plane, EventKind::UpdateStateChanged);
        }
    }

    // --- reads --------------------------------------------------------

    /// Encode one block into `payload` and return its length.
    ///
    /// An unknown block or a bad index answers with nothing (L = 0): the
    /// CRC still covers the request, so the controller can tell this
    /// from a lost answer.
    pub(crate) fn encode_block(&self, block: u8, index: u8, payload: &mut [u8]) -> usize {
        let mut w = RecordWriter::new(payload);
        match (block, index) {
            (BLK_INFO, INDEX_NONE) => self.encode_info(&mut w),
            (BLK_STATUS, INDEX_NONE) => self.encode_status(&mut w),
            (BLK_SUMMARY, INDEX_NONE) => self.encode_summary(&mut w),
            (BLK_UNIT, i) if (i as usize) < UNIT_COUNT => self.encode_unit(&mut w, i as usize),
            (BLK_SLOT, i) if (i as usize) < SLOT_COUNT => self.encode_slot(&mut w, i as usize),
            (BLK_EVENTS, INDEX_NONE) => self.encode_events(&mut w),
            (BLK_CONFIG, INDEX_NONE) => self.encode_config(&mut w),
            (BLK_LOG, INDEX_NONE) => self.encode_log(&mut w),
            _ => {}
        }
        w.len()
    }

    fn encode_info(&self, w: &mut RecordWriter) {
        let _ = Info::FormatVersion(granite_proto::FORMAT_VERSION).encode(w);
        let _ = Info::PlaneType(self.plane_type()).encode(w);
        let _ = Info::SlotCapacity(SLOT_COUNT as u8).encode(w);
        let _ = Info::FirmwareVersion(self.setup.firmware_version).encode(w);
        let _ = Info::Serial(self.setup.serial).encode(w);
        let _ = Info::Capabilities(CAPABILITIES).encode(w);
        let _ = Info::UnitsPresent(self.units_present()).encode(w);
        let _ = Info::BootCount(self.boot_count()).encode(w);
        let _ = Info::ConfigHash(self.config_hash()).encode(w);
        let _ = Info::ImageState(self.image_state()).encode(w);
        let _ = Info::BootloaderVersion(self.setup.bootloader_version).encode(w);
    }

    fn encode_status(&self, w: &mut RecordWriter) {
        let _ = Status::CoreInputVoltage(self.core_volts).encode(w);
        let _ = Status::CoreTemperature(self.core_temperature).encode(w);
        let _ = Status::Uptime(self.uptime_s()).encode(w);
        let _ = Status::Flags(self.status_flags()).encode(w);
        for held in self.results.iter() {
            let _ = Status::CommandResult(granite_proto::CommandResult::new(held.seq, held.code))
                .encode(w);
        }
        let _ = Status::EventHead(self.events.next_seq()).encode(w);
        let _ = Status::UpdateProgress(self.update.progress()).encode(w);
        let _ = Status::LinkErrors(self.link_errors).encode(w);
        let mut per_channel = [0u16; granite_proto::SEGMENT_CHANNELS];
        for (i, unit) in self.units.iter().enumerate() {
            per_channel[i] = unit.segment_errors;
        }
        let _ = Status::SegmentErrors(granite_proto::SegmentErrors { per_channel }).encode(w);
        let _ = Status::LastResetCause(self.reset_cause).encode(w);
    }

    fn encode_summary(&self, w: &mut RecordWriter) {
        for (i, slot) in self.slots.iter().enumerate() {
            if !self.unit_present(i / 2) {
                continue;
            }
            let _ = SlotSummary {
                slot: i as u8,
                status: slot.status,
                volts: slot.volts,
                amps: slot.amps,
                watts: slot.watts,
                led: slot.led.mode,
            }
            .encode(w);
        }
    }

    fn encode_unit(&self, w: &mut RecordWriter, unit: usize) {
        let model = &self.units[unit];
        if !model.present() {
            // One status record with the present bit clear, never a NAK
            // (ADR 0003 decision 2).
            let _ = Unit::Status(model.status.without(UnitStatus::PRESENT)).encode(w);
            return;
        }
        let _ = Unit::Status(model.status).encode(w);
        let _ = Unit::InputVolts(model.volts).encode(w);
        let _ = Unit::InputAmps(model.amps).encode(w);
        let _ = Unit::InputWatts(model.watts).encode(w);
        let _ = Unit::InputTemperature(model.temperature).encode(w);
        let _ = Unit::SegmentErrors(model.segment_errors).encode(w);
        let _ = Unit::TimesLost(model.times_lost).encode(w);
    }

    fn encode_slot(&self, w: &mut RecordWriter, slot: usize) {
        let model = &self.slots[slot];
        if !self.unit_present(slot / 2) {
            let _ = Slot::Status(model.status.without(SlotStatus::PRESENT)).encode(w);
            return;
        }
        let _ = Slot::Status(model.status).encode(w);
        let _ = Slot::RetryCount(model.retry_count).encode(w);
        let _ = Slot::ModuleClass(model.module_class).encode(w);
        let _ = Slot::ModuleIdRaw(model.module_id_raw).encode(w);
        let _ = Slot::Volts(model.volts).encode(w);
        let _ = Slot::Amps(model.amps).encode(w);
        let _ = Slot::Watts(model.watts).encode(w);
        let _ = Slot::PowerMinMax(PowerMinMax {
            min: model.reported_min(),
            max: model.watts_max,
        })
        .encode(w);
        let _ = Slot::Temperature(model.temperature).encode(w);
        let _ = Slot::Energy(Energy {
            joules: model.energy_joules(),
            boot_count: self.boot_count(),
        })
        .encode(w);
        let _ = Slot::NodeLed(NodeLed {
            mode: model.led.mode,
            period_ms: model.led.period_ms,
        })
        .encode(w);
        let _ = Slot::CrossCheck(model.cross).encode(w);
        let _ = Slot::SequenceState(self.sequences[slot].state()).encode(w);
        let _ = Slot::PowerLimit(model.power_limit).encode(w);
    }

    fn encode_events(&self, w: &mut RecordWriter) {
        for event in self.events.iter() {
            if w.remaining() < Event::LEN {
                break;
            }
            let _ = event.encode(w);
        }
        if self.events.lost() > 0 {
            let _ = EventsLost {
                count: self.events.lost(),
            }
            .encode(w);
        }
    }

    fn encode_config(&self, w: &mut RecordWriter) {
        let _ = CfgRecord::Hash(self.config_hash()).encode(w);
        for slot in 0..SLOT_COUNT {
            let r = self.config.retry[slot];
            let _ = CfgRecord::RetryPolicy(granite_proto::RetryPolicy {
                slot: slot as u8,
                max: r.max,
                window_s: r.window_s,
                action: r.action,
            })
            .encode(w);
        }
        for slot in 0..SLOT_COUNT {
            let _ = CfgRecord::PowerLimit(granite_proto::PowerLimit {
                slot: slot as u8,
                watts: self.config.power_limit[slot],
            })
            .encode(w);
        }
        let _ = CfgRecord::BlinkDetection(self.config.blink).encode(w);
    }

    fn encode_log(&self, w: &mut RecordWriter) {
        for line in self.log.iter() {
            if w.remaining() < 5 + line.text().len() {
                break;
            }
            let _ = LogLine {
                uptime_ms: line.uptime_ms,
                level: line.level,
                text: line.text(),
            }
            .encode(w);
        }
    }

    /// Encode a block without a frame around it, for the simulator and
    /// for tests that want the records and not the CRC.
    pub fn block_records(&self, block: u8, index: u8, out: &mut [u8]) -> usize {
        self.encode_block(block, index, out)
    }
}
