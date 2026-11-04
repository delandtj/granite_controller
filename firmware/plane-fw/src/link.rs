//! The controller link: I2C1 as a slave at 0x30 + straps (ADR 0002
//! decision 8, ADR 0003 decisions 1-4).
//!
//! embassy-stm32 0.6 does have an I2C slave on the G0's i2c v2
//! (`into_slave_multimaster`, `listen`, `respond_to_write`,
//! `respond_to_read`), including the repeated-start detection a
//! write-then-read transaction needs, so there is no PAC-level slave
//! here. The ADR's fallback ("a small interrupt-driven slave on the
//! PAC") stays unused.
//!
//! One transaction, both shapes:
//!
//! ```text
//!  read:  listen -> Write, 2 bytes (block, index), RESTART
//!         listen -> Read, answer L, records, crc
//!  write: listen -> Write, 4+ bytes (block, index, L, records, crc), STOP
//! ```
//!
//! The answer is encoded the moment the request arrives, out of the
//! model, and clocked out on the read that follows: the plane stretches
//! the clock only for as long as the encode takes, plus however long
//! the bus task holds the plane (ADR 0002 decision 8 wants the answer
//! ready before the read starts). An answer is kept after it has been
//! read, so a controller that puts a STOP between the request and the
//! read, or that retries a read whose CRC failed, gets the same
//! snapshot back (ADR 0003 "Known unknowns": the request stays valid
//! until the next one).

use embassy_stm32::i2c::{I2c, MultiMaster, SlaveCommandKind};
use embassy_stm32::mode::Async;
use granite_proto::frame;

use crate::PlaneMutex;
use crate::status;

/// Bytes the slave takes in one write.
///
/// The peripheral's slave transfer is set up in chunks below 256 bytes,
/// so this stays at 255: the longest frame ADR 0003 defines is a FW
/// data record (~137 bytes) and the largest possible is 258, which
/// would need a reload mid-transfer. A longer write is truncated here
/// and then fails its length check, which counts as a bad frame
/// (STATUS 0x19) and is never acted on.
const RX_BYTES: usize = 255;

/// The link peripheral: I2C1 in slave (multimaster) mode, async so the
/// task sleeps between transactions.
pub type LinkI2c = I2c<'static, Async, MultiMaster>;

/// Answer the controller.
#[embassy_executor::task]
pub async fn task(plane: &'static PlaneMutex, mut i2c: LinkI2c) {
    let mut rx = [0u8; RX_BYTES];
    let mut answer = [0u8; frame::MAX_ANSWER];
    let mut answer_len = 0usize;

    loop {
        let command = match i2c.listen().await {
            Ok(command) => command,
            Err(e) => {
                defmt::warn!("link listen: {}", e);
                continue;
            }
        };

        match command.kind {
            SlaveCommandKind::Write => {
                let received = match i2c.respond_to_write(&mut rx).await {
                    Ok(n) => n,
                    Err(e) => {
                        defmt::warn!("link write: {}", e);
                        continue;
                    }
                };
                let mut plane = plane.lock().await;
                answer_len = plane
                    .handle_frame(&rx[..received], &mut answer)
                    .map_or(0, <[u8]>::len);
                // A write can have acknowledged events (EV_ACK), so
                // INT_OUT may have to drop.
                status::publish(&plane);
            }
            SlaveCommandKind::Read => {
                // No request, or a request the plane could not answer:
                // one zero byte, which fails the controller's CRC check
                // and is counted there. NAKing is not an option - a NAK
                // means "the plane is gone" (ADR 0003 decision 2).
                let bytes = if answer_len == 0 {
                    &[0u8][..]
                } else {
                    &answer[..answer_len]
                };
                if let Err(e) = i2c.respond_to_read(bytes).await {
                    defmt::warn!("link read: {}", e);
                }
            }
        }
    }
}
