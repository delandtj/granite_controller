//! Framing, both directions (ADR 0003 decision 1).
//!
//! ```text
//!  read:  W: block, index          R: L, records[L], crc
//!  write: W: block, index, L, records[L], crc
//!  crc = CRC-8 over (address, block, index, L, records)
//! ```
//!
//! The CRC covers the 7-bit address and the request bytes, which the
//! plane does not send back in an answer: an answer to a misheard
//! request, or an answer from another plane, fails the check. That is
//! why [`check_answer`] needs the request it answers.
//!
//! Controller side: [`read_request`], [`check_answer`], [`write_frame`].
//! Plane side: [`parse_frame`], [`answer_frame`].

use crate::crc::crc8_update;
use crate::record::{MAX_PAYLOAD, Records};

/// Bytes of a read request: block and index.
pub const REQUEST_LEN: usize = 2;

/// Largest write frame: block, index, L, 254 payload bytes, crc.
pub const MAX_WRITE_FRAME: usize = 3 + MAX_PAYLOAD + 1;

/// Largest read answer: L, 254 payload bytes, crc.
pub const MAX_ANSWER: usize = 1 + MAX_PAYLOAD + 1;

/// Smallest write frame: block, index, L = 0, crc.
pub const MIN_WRITE_FRAME: usize = 4;

/// What can go wrong on a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameError {
    /// Fewer bytes than the frame's own length byte asks for.
    Short,
    /// `L` is above 254, or the frame length does not match `L`, or a
    /// frame is neither a 2-byte request nor a write frame.
    BadLength,
    /// The CRC-8 over (address, block, index, L, records) does not
    /// match the byte on the wire. Counted by the plane (STATUS 0x19).
    BadCrc,
    /// The destination buffer cannot hold the frame.
    BufferTooSmall,
}

/// The two bytes of a read request. The caller keeps them: the answer's
/// CRC covers them ([`check_answer`]).
pub const fn read_request(block: u8, index: u8) -> [u8; REQUEST_LEN] {
    [block, index]
}

/// CRC over a whole transaction: address, block, index, L, records.
fn transaction_crc(addr: u8, block: u8, index: u8, payload: &[u8]) -> u8 {
    let head = [addr, block, index, payload.len() as u8];
    crc8_update(crc8_update(crate::crc::INIT, &head), payload)
}

/// Build a write frame (block, index, L, records, crc) into `buf` and
/// return the bytes to put on the bus.
///
/// `addr` is the plane's 7-bit address: it is hashed into the CRC, not
/// written into the frame (the I2C peripheral sends it).
pub fn write_frame<'a>(
    buf: &'a mut [u8],
    addr: u8,
    block: u8,
    index: u8,
    payload: &[u8],
) -> Result<&'a [u8], FrameError> {
    if payload.len() > MAX_PAYLOAD {
        return Err(FrameError::BadLength);
    }
    let total = 3 + payload.len() + 1;
    if buf.len() < total {
        return Err(FrameError::BufferTooSmall);
    }
    buf[0] = block;
    buf[1] = index;
    buf[2] = payload.len() as u8;
    buf[3..3 + payload.len()].copy_from_slice(payload);
    buf[total - 1] = transaction_crc(addr, block, index, payload);
    Ok(&buf[..total])
}

/// Check a read answer and return its payload.
///
/// `answer` is what came back from the plane: `L, records[L], crc`.
/// `request` is the two request bytes it answers. Bytes past the CRC
/// are ignored, so a caller that reads a fixed-size buffer in one go
/// can pass the whole buffer.
pub fn check_answer<'a>(
    addr: u8,
    request: &[u8],
    answer: &'a [u8],
) -> Result<&'a [u8], FrameError> {
    let [block, index] = *fixed_request(request)?;
    if answer.is_empty() {
        return Err(FrameError::Short);
    }
    let len = answer[0] as usize;
    if len > MAX_PAYLOAD {
        return Err(FrameError::BadLength);
    }
    if answer.len() < len + 2 {
        return Err(FrameError::Short);
    }
    let payload = &answer[1..1 + len];
    if answer[1 + len] != transaction_crc(addr, block, index, payload) {
        return Err(FrameError::BadCrc);
    }
    Ok(payload)
}

/// `request` as the two bytes it must be.
fn fixed_request(request: &[u8]) -> Result<&[u8; REQUEST_LEN], FrameError> {
    request.try_into().map_err(|_| FrameError::BadLength)
}

/// What the plane received from the controller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Frame<'a> {
    /// A read request: answer it with [`answer_frame`].
    Read {
        /// Block id (see [`crate::Block`]).
        block: u8,
        /// Block index (0xFF where a block has only one).
        index: u8,
    },
    /// A write: the payload holds the records.
    Write {
        /// Block id (see [`crate::Block`]).
        block: u8,
        /// Block index (0xFF where a block has only one).
        index: u8,
        /// The `L` record bytes, CRC already checked.
        payload: &'a [u8],
    },
}

impl<'a> Frame<'a> {
    /// Block id of either kind of frame.
    pub const fn block(&self) -> u8 {
        match self {
            Frame::Read { block, .. } | Frame::Write { block, .. } => *block,
        }
    }

    /// Block index of either kind of frame.
    pub const fn index(&self) -> u8 {
        match self {
            Frame::Read { index, .. } | Frame::Write { index, .. } => *index,
        }
    }

    /// The records of a write (an empty iterator for a read).
    pub fn records(&self) -> Records<'a> {
        match self {
            Frame::Write { payload, .. } => Records::new(payload),
            Frame::Read { .. } => Records::new(&[]),
        }
    }
}

/// Plane side: parse what the controller wrote.
///
/// Two bytes are a read request; four or more are a write frame, whose
/// CRC is checked here. A write with a bad CRC or a bad length is
/// dropped and counted (ADR 0003 decision 1), never acted on.
pub fn parse_frame<'a>(addr: u8, frame: &'a [u8]) -> Result<Frame<'a>, FrameError> {
    match frame.len() {
        0 | 1 => Err(FrameError::Short),
        REQUEST_LEN => Ok(Frame::Read {
            block: frame[0],
            index: frame[1],
        }),
        // 3 bytes is neither: a request with a stray byte, or a write
        // frame that lost its CRC.
        3 => Err(FrameError::BadLength),
        n => {
            let block = frame[0];
            let index = frame[1];
            let len = frame[2] as usize;
            if len > MAX_PAYLOAD || n != len + 4 {
                return Err(FrameError::BadLength);
            }
            let payload = &frame[3..3 + len];
            if frame[n - 1] != transaction_crc(addr, block, index, payload) {
                return Err(FrameError::BadCrc);
            }
            Ok(Frame::Write {
                block,
                index,
                payload,
            })
        }
    }
}

/// Plane side: encode an answer (L, records, crc) to the request it
/// answers, into `buf`.
pub fn answer_frame<'a>(
    buf: &'a mut [u8],
    addr: u8,
    block: u8,
    index: u8,
    payload: &[u8],
) -> Result<&'a [u8], FrameError> {
    if payload.len() > MAX_PAYLOAD {
        return Err(FrameError::BadLength);
    }
    let total = 1 + payload.len() + 1;
    if buf.len() < total {
        return Err(FrameError::BufferTooSmall);
    }
    buf[0] = payload.len() as u8;
    buf[1..1 + payload.len()].copy_from_slice(payload);
    buf[total - 1] = transaction_crc(addr, block, index, payload);
    Ok(&buf[..total])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::RecordWriter;

    #[test]
    fn read_transaction_round_trip() {
        let addr = 0x33;
        let request = read_request(crate::BLK_STATUS, crate::INDEX_NONE);
        let frame = parse_frame(addr, &request).unwrap();
        assert_eq!(
            frame,
            Frame::Read {
                block: crate::BLK_STATUS,
                index: crate::INDEX_NONE
            }
        );

        let mut payload = [0u8; 16];
        let mut w = RecordWriter::new(&mut payload);
        w.push(0x12, &1234u32.to_le_bytes()).unwrap();
        let mut out = [0u8; MAX_ANSWER];
        let answer = answer_frame(
            &mut out,
            addr,
            crate::BLK_STATUS,
            crate::INDEX_NONE,
            w.payload(),
        )
        .unwrap()
        .to_vec();

        let got = check_answer(addr, &request, &answer).unwrap();
        assert_eq!(got, w.payload());
    }

    #[test]
    fn answer_checked_against_the_wrong_address_fails() {
        let request = read_request(crate::BLK_INFO, crate::INDEX_NONE);
        let mut out = [0u8; MAX_ANSWER];
        let answer = answer_frame(
            &mut out,
            0x30,
            crate::BLK_INFO,
            crate::INDEX_NONE,
            &[0x01, 1, 1],
        )
        .unwrap()
        .to_vec();
        assert!(check_answer(0x30, &request, &answer).is_ok());
        assert_eq!(
            check_answer(0x31, &request, &answer),
            Err(FrameError::BadCrc)
        );
    }

    #[test]
    fn answer_to_another_request_fails() {
        let mut out = [0u8; MAX_ANSWER];
        let answer = answer_frame(&mut out, 0x30, crate::BLK_SLOT, 2, &[0x20, 2, 0x07, 0x00])
            .unwrap()
            .to_vec();
        let wrong = read_request(crate::BLK_SLOT, 3);
        assert_eq!(check_answer(0x30, &wrong, &answer), Err(FrameError::BadCrc));
    }

    #[test]
    fn trailing_bytes_after_the_crc_are_ignored() {
        let request = read_request(crate::BLK_INFO, crate::INDEX_NONE);
        let mut out = [0u8; MAX_ANSWER];
        let n = answer_frame(
            &mut out,
            0x30,
            crate::BLK_INFO,
            crate::INDEX_NONE,
            &[0x01, 1, 1],
        )
        .unwrap()
        .len();
        out[n] = 0xAA;
        assert_eq!(
            check_answer(0x30, &request, &out[..n + 1]).unwrap(),
            &[0x01, 1, 1]
        );
    }

    #[test]
    fn long_payloads_are_rejected() {
        let mut out = [0u8; 300];
        let payload = [0u8; MAX_PAYLOAD + 1];
        assert_eq!(
            write_frame(&mut out, 0x30, crate::BLK_CTRL, crate::INDEX_NONE, &payload),
            Err(FrameError::BadLength)
        );
        assert_eq!(
            answer_frame(&mut out, 0x30, crate::BLK_INFO, crate::INDEX_NONE, &payload),
            Err(FrameError::BadLength)
        );
        // L above 254 on the wire.
        let mut answer = [0u8; 258];
        answer[0] = 255;
        let request = read_request(crate::BLK_INFO, crate::INDEX_NONE);
        assert_eq!(
            check_answer(0x30, &request, &answer),
            Err(FrameError::BadLength)
        );
        let mut frame = [0u8; 260];
        frame[0] = crate::BLK_CTRL;
        frame[1] = crate::INDEX_NONE;
        frame[2] = 255;
        assert_eq!(parse_frame(0x30, &frame), Err(FrameError::BadLength));
    }

    #[test]
    fn small_buffers_are_an_error() {
        let mut out = [0u8; 3];
        assert_eq!(
            write_frame(
                &mut out,
                0x30,
                crate::BLK_CTRL,
                crate::INDEX_NONE,
                &[0x4F, 1, 1]
            ),
            Err(FrameError::BufferTooSmall)
        );
        assert_eq!(
            answer_frame(
                &mut out,
                0x30,
                crate::BLK_INFO,
                crate::INDEX_NONE,
                &[0x01, 1, 1, 0]
            ),
            Err(FrameError::BufferTooSmall)
        );
    }

    #[test]
    fn write_frame_round_trip() {
        let mut out = [0u8; MAX_WRITE_FRAME];
        let payload = [0x4F, 1, 0x07, 0x45, 3, 0x03, 0xC8, 0x00];
        let frame = write_frame(&mut out, 0x35, crate::BLK_CTRL, crate::INDEX_NONE, &payload)
            .unwrap()
            .to_vec();
        match parse_frame(0x35, &frame).unwrap() {
            Frame::Write {
                block,
                index,
                payload: got,
            } => {
                assert_eq!((block, index), (crate::BLK_CTRL, crate::INDEX_NONE));
                assert_eq!(got, &payload);
            }
            other => panic!("not a write: {other:?}"),
        }
        // One flipped bit anywhere is caught.
        for i in 0..frame.len() {
            let mut bad = frame.clone();
            bad[i] ^= 0x01;
            assert!(
                !matches!(parse_frame(0x35, &bad), Ok(Frame::Write { .. })),
                "bit flip at {i} passed"
            );
        }
    }

    #[test]
    fn stray_lengths_are_rejected() {
        assert_eq!(parse_frame(0x30, &[]), Err(FrameError::Short));
        assert_eq!(parse_frame(0x30, &[0x01]), Err(FrameError::Short));
        assert_eq!(
            parse_frame(0x30, &[0x10, 0xFF, 0x00]),
            Err(FrameError::BadLength)
        );
        // L = 0 write: four bytes, CRC over the head only.
        let mut out = [0u8; 8];
        let frame = write_frame(&mut out, 0x30, crate::BLK_EV_ACK, crate::INDEX_NONE, &[])
            .unwrap()
            .to_vec();
        assert_eq!(frame.len(), MIN_WRITE_FRAME);
        assert!(matches!(
            parse_frame(0x30, &frame),
            Ok(Frame::Write { payload, .. }) if payload.is_empty()
        ));
    }
}
