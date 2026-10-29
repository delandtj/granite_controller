//! Plane firmware updates, one plane at a time (ADR 0004 decision 8).
//!
//! The image comes from outside the client (the controller's signed
//! OTA bundle carries it, or an API upload of a plane image alone), so
//! the client only wants a reader: [`ImageSource`] gives it the size,
//! the CRC-32, the version and a chunk at an offset.
//!
//! The run, with the plane's side in ADR 0002 decision 9 and the
//! records in ADR 0003 FW 0x90-0x94:
//!
//! 1. **begin**: FW 0x90 with size, CRC-32 and version; the plane
//!    erases its DFU slot.
//! 2. **data**: FW 0x91 records, paced by STATUS 0x18 - the client
//!    writes from the offset the plane asks for next, which makes a
//!    lost or refused record cost one round trip and no state.
//! 3. **commit**: FW 0x92 once the offset reaches the size; the plane
//!    verifies the slot and marks the swap.
//! 4. **reboot**: CTRL 0x4C; the bootloader performs the swap and the
//!    new image comes up on probation.
//! 5. **confirm**: the client waits for the plane to answer INFO with a
//!    new boot count, checks that it sees the units it saw before, and
//!    only then sends FW 0x93.
//!
//! Anything that goes wrong ends the run with an [`UpdateFailure`] and
//! no confirm, which is the safe direction: an unconfirmed image rolls
//! itself back when its probation runs out (ADR 0002), so a controller
//! that dies in the middle of an update still leaves a working plane.
//!
//! The policy of ADR 0004 decision 8 is [`super::UpdatePolicy`]:
//! `manual` (the default) means an update starts only when something
//! asks for it. The client never fetches an image by itself under
//! `auto` either - it has none - so `auto` is the firmware's cue to
//! offer the bundle's image to every plane
//! [`super::PlaneClient::update_candidates`] names.

use alloc::boxed::Box;

use granite_proto::{
    BLK_FW, BLK_INFO, BLK_STATUS, Fw, FwBegin, FwData, INDEX_NONE, ImageState, IndexBitmap, Info,
    MAX_PAYLOAD, RecordWriter, Status, UpdateState, Version, frame,
};

use crate::hal::{HalError, HalResult, PlaneLink};

use super::link::{PlaneLinkError, Transactions};
use super::model::Plane;

/// FW data records one [`super::PlaneClient::poll`] writes: 4 x 128
/// bytes, so a 54 KB image takes about 110 polls (some 10 s at a 100 ms
/// cadence) and no single call holds the link for long.
pub const DATA_RECORDS_PER_STEP: u32 = 4;

/// How long the transfer may take before the client gives up \[ms\].
pub const TRANSFER_TIMEOUT_MS: u64 = 300_000;

/// How long the plane may take to verify the image after a commit
/// \[ms\].
pub const COMMIT_TIMEOUT_MS: u64 = 30_000;

/// How long the client waits for the plane to come back after the
/// reboot that performs the swap \[ms\].
pub const REBOOT_TIMEOUT_MS: u64 = 30_000;

/// How long the client waits for the plane to report a confirmed image
/// after FW confirm \[ms\].
pub const CONFIRM_TIMEOUT_MS: u64 = 10_000;

/// Where a plane image comes from.
///
/// The client reads it in [`granite_proto::FW_DATA_MAX`] pieces while
/// the update runs, so nothing has to hold a whole image in RAM: the
/// firmware's implementation can read straight out of the OTA
/// partition.
///
/// `Send` is required because the client that holds a running update is
/// owned by a task (and, in the simulator, by a shared state an axum
/// handler locks); an image source that cannot move between threads
/// would pin the whole client to one.
pub trait ImageSource: Send {
    /// Bytes in the image.
    fn size(&self) -> u32;

    /// CRC-32 (IEEE) over the whole image, as FW begin and FW commit
    /// carry it.
    fn crc32(&self) -> u32;

    /// The version the image claims.
    fn version(&self) -> Version;

    /// Copy the bytes at `offset` into `out` and say how many came; a
    /// short read is the end of the image.
    fn read(&mut self, offset: u32, out: &mut [u8]) -> HalResult<usize>;
}

/// An image that is already in memory: a `Vec<u8>` a host test or an
/// API upload holds, or a `&'static [u8]` a firmware embedded.
///
/// A plane image is tens of kilobytes, so a board usually implements
/// [`ImageSource`] over its OTA partition instead of buffering one.
#[derive(Debug, Clone, Copy)]
pub struct BufferedImage<B> {
    bytes: B,
    crc32: u32,
    version: Version,
}

impl<B: AsRef<[u8]>> BufferedImage<B> {
    /// An image over `bytes`. The CRC-32 is the caller's: granite-core
    /// has no CRC-32 of its own and the OTA bundle carries one anyway.
    pub const fn new(bytes: B, crc32: u32, version: Version) -> Self {
        BufferedImage {
            bytes,
            crc32,
            version,
        }
    }
}

impl<B: AsRef<[u8]> + Send> ImageSource for BufferedImage<B> {
    fn size(&self) -> u32 {
        self.bytes.as_ref().len() as u32
    }

    fn crc32(&self) -> u32 {
        self.crc32
    }

    fn version(&self) -> Version {
        self.version
    }

    fn read(&mut self, offset: u32, out: &mut [u8]) -> HalResult<usize> {
        let bytes = self.bytes.as_ref();
        let at = offset as usize;
        if at > bytes.len() {
            return Err(HalError::OutOfRange);
        }
        let n = out.len().min(bytes.len() - at);
        out[..n].copy_from_slice(&bytes[at..at + n]);
        Ok(n)
    }
}

/// Where an update run is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateStage {
    /// FW begin has to go out.
    Begin,
    /// Writing FW data, paced by the plane's next offset.
    Sending,
    /// FW commit is out; the plane verifies the slot.
    Committing,
    /// The reboot that performs the swap has to go out.
    Rebooting,
    /// Waiting for the plane to answer with a new boot count.
    WaitingBack,
    /// FW confirm is out; waiting for the plane to call its image
    /// confirmed.
    Confirming,
}

impl UpdateStage {
    /// Lowercase wire name.
    pub const fn as_str(self) -> &'static str {
        match self {
            UpdateStage::Begin => "begin",
            UpdateStage::Sending => "sending",
            UpdateStage::Committing => "committing",
            UpdateStage::Rebooting => "rebooting",
            UpdateStage::WaitingBack => "waiting_back",
            UpdateStage::Confirming => "confirming",
        }
    }
}

/// Why an update did not end with a confirmed image.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateFailure {
    /// The image could not be read.
    Image,
    /// The transfer broke: the plane reported `failed`, or refused
    /// every offset, or the link would not carry it.
    Transfer,
    /// The plane did not verify the image after the commit.
    Commit,
    /// The plane did not come back after the reboot.
    NoReturn,
    /// The plane came back without the units it had before, so the
    /// client did not confirm (ADR 0004 decision 8).
    UnitsChanged,
    /// The plane rolled the image back: its probation ran out, or the
    /// bootloader refused the swap.
    RolledBack,
    /// The plane came back but never called its image confirmed.
    NotConfirmed,
    /// Something asked for the run to stop.
    Aborted,
}

impl UpdateFailure {
    /// Lowercase wire name.
    pub const fn as_str(self) -> &'static str {
        match self {
            UpdateFailure::Image => "image",
            UpdateFailure::Transfer => "transfer",
            UpdateFailure::Commit => "commit",
            UpdateFailure::NoReturn => "no_return",
            UpdateFailure::UnitsChanged => "units_changed",
            UpdateFailure::RolledBack => "rolled_back",
            UpdateFailure::NotConfirmed => "not_confirmed",
            UpdateFailure::Aborted => "aborted",
        }
    }
}

impl core::fmt::Display for UpdateFailure {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Why the client would not start an update.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateRefused {
    /// No plane answers at that address.
    PlaneAbsent,
    /// An update is already running (one plane at a time).
    Busy,
    /// A node sequence is running on that plane (ADR 0004 decision 8:
    /// never while one does).
    SequenceRunning,
    /// Commands are still in flight on that plane.
    CommandsInFlight,
    /// The image is empty, or bigger than the plane can take.
    BadImage,
    /// The plane does not report the firmware update capability.
    NotCapable,
}

impl UpdateRefused {
    /// Lowercase wire name.
    pub const fn as_str(self) -> &'static str {
        match self {
            UpdateRefused::PlaneAbsent => "plane_absent",
            UpdateRefused::Busy => "busy",
            UpdateRefused::SequenceRunning => "sequence_running",
            UpdateRefused::CommandsInFlight => "commands_in_flight",
            UpdateRefused::BadImage => "bad_image",
            UpdateRefused::NotCapable => "not_capable",
        }
    }
}

impl core::fmt::Display for UpdateRefused {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// How a run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateEnd {
    /// The new image runs and is confirmed.
    Confirmed,
    /// It did not get there.
    Failed(UpdateFailure),
}

/// One update run.
pub struct UpdateJob {
    /// The plane being updated.
    pub address: u8,
    /// Where the run is.
    pub stage: UpdateStage,
    /// The version the image claims.
    pub version: Version,
    /// Bytes in the image.
    pub size: u32,
    /// The offset the plane last asked for.
    pub offset: u32,
    /// When this stage runs out of time \[ms\].
    pub deadline_ms: u64,
    /// When the run started \[ms\].
    pub started_ms: u64,
    /// The units the plane had before the reboot: the client confirms
    /// only when it sees them again (ADR 0004 decision 8).
    units_before: IndexBitmap,
    /// The boot count before the reboot.
    boot_before: u32,
    /// Where the bytes come from.
    image: Box<dyn ImageSource>,
    /// The sequence number the reboot went out under.
    reboot_seq: u8,
}

impl core::fmt::Debug for UpdateJob {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("UpdateJob")
            .field("address", &self.address)
            .field("stage", &self.stage)
            .field("version", &self.version)
            .field("size", &self.size)
            .field("offset", &self.offset)
            .finish_non_exhaustive()
    }
}

impl UpdateJob {
    /// A run against the plane at `address`.
    pub fn new(now_ms: u64, address: u8, plane: &Plane, image: Box<dyn ImageSource>) -> Self {
        UpdateJob {
            address,
            stage: UpdateStage::Begin,
            version: image.version(),
            size: image.size(),
            offset: 0,
            deadline_ms: now_ms + TRANSFER_TIMEOUT_MS,
            started_ms: now_ms,
            units_before: plane.units_present,
            boot_before: plane.boot_count,
            image,
            reboot_seq: 0,
        }
    }

    /// How far the transfer has got, 0-100.
    pub const fn percent(&self) -> u32 {
        if self.size == 0 {
            return 0;
        }
        (self.offset as u64 * 100 / self.size as u64) as u32
    }

    /// Do the next piece of work. `None` while the run goes on.
    ///
    /// `reboot_seq` is a sequence number the client lends for the CTRL
    /// reboot write; the run does not wait for its result, because the
    /// plane resets before it can be read.
    pub fn step<L: PlaneLink>(
        &mut self,
        now_ms: u64,
        tx: &mut Transactions,
        link: &mut L,
        next_seq: &mut dyn FnMut() -> u8,
    ) -> Option<UpdateEnd> {
        if now_ms > self.deadline_ms {
            return Some(UpdateEnd::Failed(match self.stage {
                UpdateStage::Begin | UpdateStage::Sending => UpdateFailure::Transfer,
                UpdateStage::Committing => UpdateFailure::Commit,
                UpdateStage::Rebooting | UpdateStage::WaitingBack => UpdateFailure::NoReturn,
                UpdateStage::Confirming => UpdateFailure::NotConfirmed,
            }));
        }
        match self.stage {
            UpdateStage::Begin => self.begin(now_ms, tx, link),
            UpdateStage::Sending => self.send(now_ms, tx, link),
            UpdateStage::Committing => self.commit(now_ms, tx, link),
            UpdateStage::Rebooting => self.reboot(now_ms, tx, link, next_seq),
            UpdateStage::WaitingBack => self.waiting_back(now_ms, tx, link),
            UpdateStage::Confirming => self.confirming(tx, link),
        }
    }

    /// Write FW abort, for a run that is being given up on while the
    /// plane still holds a transfer.
    pub fn abort<L: PlaneLink>(&self, tx: &mut Transactions, link: &mut L) {
        if matches!(
            self.stage,
            UpdateStage::Begin | UpdateStage::Sending | UpdateStage::Committing
        ) {
            let _ = write_fw(tx, link, self.address, &[Fw::Abort]);
        }
    }

    fn begin<L: PlaneLink>(
        &mut self,
        now_ms: u64,
        tx: &mut Transactions,
        link: &mut L,
    ) -> Option<UpdateEnd> {
        let begin = FwBegin {
            size: self.size,
            crc32: self.image.crc32(),
            version: self.version,
        };
        if write_fw(tx, link, self.address, &[Fw::Begin(begin)]).is_err() {
            // The link will come back or the plane is gone; the stage
            // deadline decides.
            return None;
        }
        self.stage = UpdateStage::Sending;
        self.deadline_ms = now_ms + TRANSFER_TIMEOUT_MS;
        None
    }

    fn send<L: PlaneLink>(
        &mut self,
        now_ms: u64,
        tx: &mut Transactions,
        link: &mut L,
    ) -> Option<UpdateEnd> {
        let progress = match read_progress(tx, link, self.address) {
            Ok(p) => p,
            Err(_) => return None,
        };
        self.offset = progress.next_offset;
        match progress.state {
            UpdateState::Failed => Some(UpdateEnd::Failed(UpdateFailure::Transfer)),
            // The plane is still erasing, or it has already taken
            // everything and is waiting for the commit.
            UpdateState::Erasing | UpdateState::Verifying => None,
            UpdateState::Ready => {
                self.stage = UpdateStage::Rebooting;
                None
            }
            UpdateState::Idle => {
                // The plane forgot the transfer (a reboot of its own):
                // start again from FW begin.
                self.stage = UpdateStage::Begin;
                None
            }
            UpdateState::Receiving => {
                if self.offset >= self.size {
                    if write_fw(
                        tx,
                        link,
                        self.address,
                        &[Fw::Commit {
                            crc32: self.image.crc32(),
                        }],
                    )
                    .is_ok()
                    {
                        self.stage = UpdateStage::Committing;
                        self.deadline_ms = now_ms + COMMIT_TIMEOUT_MS;
                    }
                    return None;
                }
                let mut chunk = [0u8; granite_proto::FW_DATA_MAX];
                for _ in 0..DATA_RECORDS_PER_STEP {
                    if self.offset >= self.size {
                        break;
                    }
                    let want = (self.size - self.offset).min(chunk.len() as u32) as usize;
                    let n = match self.image.read(self.offset, &mut chunk[..want]) {
                        Ok(0) | Err(_) => return Some(UpdateEnd::Failed(UpdateFailure::Image)),
                        Ok(n) => n,
                    };
                    let record = Fw::Data(FwData {
                        offset: self.offset,
                        data: &chunk[..n],
                    });
                    if write_fw(tx, link, self.address, &[record]).is_err() {
                        // Nothing is lost: the next STATUS read says
                        // where the plane wants to go on from.
                        break;
                    }
                    self.offset += n as u32;
                }
                None
            }
        }
    }

    fn commit<L: PlaneLink>(
        &mut self,
        now_ms: u64,
        tx: &mut Transactions,
        link: &mut L,
    ) -> Option<UpdateEnd> {
        let progress = match read_progress(tx, link, self.address) {
            Ok(p) => p,
            Err(_) => return None,
        };
        match progress.state {
            UpdateState::Ready => {
                self.stage = UpdateStage::Rebooting;
                self.deadline_ms = now_ms + REBOOT_TIMEOUT_MS;
                None
            }
            UpdateState::Failed | UpdateState::Idle => {
                Some(UpdateEnd::Failed(UpdateFailure::Commit))
            }
            _ => None,
        }
    }

    fn reboot<L: PlaneLink>(
        &mut self,
        now_ms: u64,
        tx: &mut Transactions,
        link: &mut L,
        next_seq: &mut dyn FnMut() -> u8,
    ) -> Option<UpdateEnd> {
        self.reboot_seq = next_seq();
        let mut buf = [0u8; MAX_PAYLOAD];
        let mut w = RecordWriter::new(&mut buf);
        if granite_proto::Command::write_ctrl(
            &mut w,
            self.reboot_seq,
            &[granite_proto::Command::Reboot],
        )
        .is_err()
        {
            return Some(UpdateEnd::Failed(UpdateFailure::Aborted));
        }
        let payload = w.payload();
        if tx
            .write(link, self.address, granite_proto::BLK_CTRL, payload)
            .is_err()
        {
            return None;
        }
        self.stage = UpdateStage::WaitingBack;
        self.deadline_ms = now_ms + REBOOT_TIMEOUT_MS;
        None
    }

    fn waiting_back<L: PlaneLink>(
        &mut self,
        now_ms: u64,
        tx: &mut Transactions,
        link: &mut L,
    ) -> Option<UpdateEnd> {
        let info = match read_info(tx, link, self.address) {
            Ok(i) => i,
            // Still down: the stage deadline is what gives up.
            Err(_) => return None,
        };
        if info.boot_count == self.boot_before {
            return None;
        }
        if matches!(info.image_state, ImageState::RolledBack) {
            return Some(UpdateEnd::Failed(UpdateFailure::RolledBack));
        }
        if info.units_present.bits() != self.units_before.bits() {
            // ADR 0004 decision 8: the controller confirms an image
            // after reading INFO *and* seeing the units it saw before.
            // Not confirming is what rolls the image back.
            return Some(UpdateEnd::Failed(UpdateFailure::UnitsChanged));
        }
        if write_fw(tx, link, self.address, &[Fw::Confirm]).is_err() {
            return None;
        }
        self.stage = UpdateStage::Confirming;
        self.deadline_ms = now_ms + CONFIRM_TIMEOUT_MS;
        None
    }

    fn confirming<L: PlaneLink>(
        &mut self,
        tx: &mut Transactions,
        link: &mut L,
    ) -> Option<UpdateEnd> {
        let info = match read_info(tx, link, self.address) {
            Ok(i) => i,
            Err(_) => return None,
        };
        match info.image_state {
            ImageState::Confirmed => Some(UpdateEnd::Confirmed),
            ImageState::RolledBack => Some(UpdateEnd::Failed(UpdateFailure::RolledBack)),
            ImageState::Probation => None,
        }
    }
}

/// Write a set of FW records to a plane.
fn write_fw<L: PlaneLink>(
    tx: &mut Transactions,
    link: &mut L,
    address: u8,
    records: &[Fw<'_>],
) -> Result<(), PlaneLinkError> {
    let mut buf = [0u8; MAX_PAYLOAD];
    let mut w = RecordWriter::new(&mut buf);
    for record in records {
        record
            .encode(&mut w)
            .map_err(|_| PlaneLinkError::Frame(frame::FrameError::BadLength))?;
    }
    let payload = w.payload();
    tx.write(link, address, BLK_FW, payload)
}

/// The update progress a plane reports (STATUS 0x18).
fn read_progress<L: PlaneLink>(
    tx: &mut Transactions,
    link: &mut L,
    address: u8,
) -> Result<granite_proto::UpdateProgress, PlaneLinkError> {
    let mut buf = [0u8; frame::MAX_ANSWER];
    let n = tx.read(link, address, BLK_STATUS, INDEX_NONE, &mut buf)?;
    Status::records(&buf[..n])
        .flatten()
        .find_map(|r| match r {
            Status::UpdateProgress(p) => Some(p),
            _ => None,
        })
        .ok_or(PlaneLinkError::Frame(frame::FrameError::BadLength))
}

/// The three INFO fields the run cares about.
struct UpdateInfo {
    boot_count: u32,
    image_state: ImageState,
    units_present: IndexBitmap,
}

/// Read INFO and pick out what the run needs.
fn read_info<L: PlaneLink>(
    tx: &mut Transactions,
    link: &mut L,
    address: u8,
) -> Result<UpdateInfo, PlaneLinkError> {
    let mut buf = [0u8; frame::MAX_ANSWER];
    let n = tx.read(link, address, BLK_INFO, INDEX_NONE, &mut buf)?;
    let mut out = UpdateInfo {
        boot_count: 0,
        image_state: ImageState::Confirmed,
        units_present: IndexBitmap::NONE,
    };
    let mut seen = false;
    for record in Info::records(&buf[..n]).flatten() {
        match record {
            Info::BootCount(n) => {
                out.boot_count = n;
                seen = true;
            }
            Info::ImageState(s) => out.image_state = s,
            Info::UnitsPresent(m) => out.units_present = m,
            _ => {}
        }
    }
    if seen {
        Ok(out)
    } else {
        Err(PlaneLinkError::Frame(frame::FrameError::BadLength))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_buffered_image_reads_its_pieces() {
        let bytes: alloc::vec::Vec<u8> = (0..300u32).map(|i| i as u8).collect();
        let mut image = BufferedImage::new(bytes, 0x1234, Version::new(0, 2, 0));
        assert_eq!(image.size(), 300);
        assert_eq!(image.crc32(), 0x1234);
        let mut out = [0u8; 128];
        assert_eq!(image.read(0, &mut out), Ok(128));
        assert_eq!(out[0], 0);
        assert_eq!(image.read(256, &mut out), Ok(44));
        assert_eq!(out[0], 0);
        assert_eq!(image.read(300, &mut out), Ok(0));
        assert_eq!(image.read(301, &mut out), Err(HalError::OutOfRange));
    }

    #[test]
    fn the_names_are_the_wire_names() {
        assert_eq!(UpdateStage::WaitingBack.as_str(), "waiting_back");
        assert_eq!(UpdateFailure::UnitsChanged.as_str(), "units_changed");
        assert_eq!(UpdateRefused::SequenceRunning.as_str(), "sequence_running");
    }
}
