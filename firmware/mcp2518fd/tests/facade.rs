//! Byte-exact tests of what this facade itself decides, against a mock
//! SPI device: the padding and DLC of `send`, the `WouldBlock` mapping,
//! the empty-FIFO `None` of `receive`, and the frame it decodes.
//!
//! The register map, the SPI framing and every errata work-around belong
//! to `mcp251xfd` and are tested there (40 test functions in its
//! `tests/driver.rs` alone); nothing here re-tests them. The transaction
//! helpers below mirror its wire format only so the expectations can be
//! written out in full: 16-bit big-endian command word (4-bit opcode,
//! 12-bit address), register payloads little-endian.

use embedded_hal_mock::eh1::spi::{Mock, Transaction};
use mcp2518fd::{Error, Mcp2518fd};

/// `CiFIFOCON` of FIFO 1 (the TX FIFO): 0x05C + 12 * (n - 1).
const TX_CON: u16 = 0x05C;
/// `CiFIFOSTA` of FIFO 1, immediately followed by `CiFIFOUA`.
const TX_STA: u16 = 0x060;
/// `CiFIFOCON` of FIFO 2 (the RX FIFO).
const RX_CON: u16 = 0x068;
/// `CiFIFOSTA` of FIFO 2.
const RX_STA: u16 = 0x06C;
/// First address of the 2 KiB message RAM.
const RAM_START: u16 = 0x400;
/// `TFNRFNIF`: TX FIFO not full / RX FIFO not empty.
const NOT_FULL_OR_NOT_EMPTY: u32 = 1 << 0;
/// `CiFIFOCON` byte 1 with `UINC | TXREQ`, and with `UINC` alone.
const UINC_TXREQ: u8 = 0x03;
const UINC: u8 = 0x01;

fn read_cmd(addr: u16) -> Vec<u8> {
    vec![0x30 | (addr >> 8) as u8, (addr & 0xFF) as u8]
}

fn write_cmd(addr: u16) -> Vec<u8> {
    vec![0x20 | (addr >> 8) as u8, (addr & 0xFF) as u8]
}

/// One READ transaction returning two consecutive 32-bit registers
/// (the status / user-address pair the driver fetches in one go).
fn r32_pair(addr: u16, lo: u32, hi: u32) -> Vec<Transaction<u8>> {
    let mut data = lo.to_le_bytes().to_vec();
    data.extend_from_slice(&hi.to_le_bytes());
    vec![
        Transaction::transaction_start(),
        Transaction::write_vec(read_cmd(addr)),
        Transaction::read_vec(data),
        Transaction::transaction_end(),
    ]
}

/// One message-RAM write transaction.
fn wram(addr: u16, data: Vec<u8>) -> Vec<Transaction<u8>> {
    vec![
        Transaction::transaction_start(),
        Transaction::write_vec(write_cmd(addr)),
        Transaction::write_vec(data),
        Transaction::transaction_end(),
    ]
}

/// One message-RAM read transaction.
fn rram(addr: u16, data: Vec<u8>) -> Vec<Transaction<u8>> {
    vec![
        Transaction::transaction_start(),
        Transaction::write_vec(read_cmd(addr)),
        Transaction::read_vec(data),
        Transaction::transaction_end(),
    ]
}

/// One single-byte register write transaction.
fn w8(addr: u16, val: u8) -> Vec<Transaction<u8>> {
    vec![
        Transaction::transaction_start(),
        Transaction::write_vec(write_cmd(addr)),
        Transaction::write_vec(vec![val]),
        Transaction::transaction_end(),
    ]
}

/// The 8-byte header of a TX or RX message object: T0/R0 is the packed
/// id, T1/R1 carries DLC in bits 3:0, BRS in bit 6, FDF in bit 7 and the
/// TX sequence number from bit 9 up.
fn object_header(id: u16, dlc: u8, brs: bool, fdf: bool, seq: u32) -> Vec<u8> {
    let w0 = id as u32;
    let w1 = (dlc as u32 & 0xF) | ((brs as u32) << 6) | ((fdf as u32) << 7) | (seq << 9);
    let mut out = w0.to_le_bytes().to_vec();
    out.extend_from_slice(&w1.to_le_bytes());
    out
}

/// `send` pads the payload up to the next valid CAN FD length and sets
/// the DLC from the padded length: 10 bytes become 12 (DLC 9), the two
/// pad bytes are zero. ADR 0005 decision 3 ignores the padding because
/// the message carries its own length.
#[test]
fn send_pads_to_the_next_valid_fd_length() {
    let payload: Vec<u8> = (1..=10).collect();
    let mut object = object_header(0x123, 9, true, true, 0);
    object.extend_from_slice(&payload);
    object.extend_from_slice(&[0, 0]); // padded 10 -> 12
    assert_eq!(object.len(), 8 + 12);

    let mut expected = r32_pair(TX_STA, NOT_FULL_OR_NOT_EMPTY, 0);
    expected.extend(wram(RAM_START, object));
    expected.extend(w8(TX_CON + 1, UINC_TXREQ));

    let mut can = Mcp2518fd::new(Mock::new(&expected));
    can.send(0x123, &payload).unwrap();
    can.release().done();
}

/// A full 63-byte link segment (1 index byte + 63 payload bytes is the
/// 64-byte frame of ADR 0005 decision 3) pads to 64, DLC 15.
#[test]
fn send_pads_a_63_byte_segment_to_64() {
    let payload = vec![0xA5u8; 63];
    let mut object = object_header(0x033, 15, true, true, 0);
    object.extend_from_slice(&payload);
    object.push(0);
    assert_eq!(object.len(), 8 + 64);

    let mut expected = r32_pair(TX_STA, NOT_FULL_OR_NOT_EMPTY, 0);
    expected.extend(wram(RAM_START, object));
    expected.extend(w8(TX_CON + 1, UINC_TXREQ));

    let mut can = Mcp2518fd::new(Mock::new(&expected));
    can.send(0x033, &payload).unwrap();
    can.release().done();
}

/// An 8-byte payload is already a valid length: no padding, DLC 8.
#[test]
fn send_does_not_pad_a_valid_length() {
    let payload = vec![0xFFu8; 8];
    let mut object = object_header(0x020, 8, true, true, 0);
    object.extend_from_slice(&payload);
    assert_eq!(object.len(), 8 + 8);

    let mut expected = r32_pair(TX_STA, NOT_FULL_OR_NOT_EMPTY, 0);
    expected.extend(wram(RAM_START, object));
    expected.extend(w8(TX_CON + 1, UINC_TXREQ));

    let mut can = Mcp2518fd::new(Mock::new(&expected));
    can.send(0x020, &payload).unwrap();
    can.release().done();
}

/// A full transmit FIFO becomes `WouldBlock`, and nothing is written:
/// the status read is the only transaction.
#[test]
fn send_maps_a_full_tx_fifo_to_would_block() {
    let expected = r32_pair(TX_STA, 0, 0);
    let mut can = Mcp2518fd::new(Mock::new(&expected));
    assert_eq!(can.send(0x123, &[1, 2, 3]), Err(Error::WouldBlock));
    can.release().done();
}

/// An id that does not fit 11 bits, and a payload over 64 bytes, are
/// refused before any SPI traffic.
#[test]
fn send_rejects_bad_arguments_without_touching_the_bus() {
    let mut can = Mcp2518fd::new(Mock::new(&[]));
    assert_eq!(can.send(0x800, &[0]), Err(Error::InvalidId));
    assert_eq!(can.send(0x123, &[0u8; 65]), Err(Error::PayloadTooLong));
    can.release().done();
}

/// An empty receive FIFO is `Ok(None)`, not an error: the status read is
/// the only transaction and the FIFO is not advanced.
#[test]
fn receive_returns_none_on_an_empty_fifo() {
    let expected = r32_pair(RX_STA, 0, 0);
    let mut can = Mcp2518fd::new(Mock::new(&expected));
    assert_eq!(can.receive(), Ok(None));
    can.release().done();
}

/// A received FD frame becomes a `Frame` with the 11-bit id and the
/// decoded length, and the FIFO tail advances by one (`UINC`).
#[test]
fn receive_decodes_an_fd_frame_and_advances_the_fifo() {
    // DLC 9 = 12 payload bytes; the last two are padding the sender added.
    let mut payload: Vec<u8> = (1..=10).collect();
    payload.extend_from_slice(&[0, 0]);

    let mut expected = r32_pair(RX_STA, NOT_FULL_OR_NOT_EMPTY, 0);
    expected.extend(rram(RAM_START, object_header(0x033, 9, true, true, 0)));
    expected.extend(rram(RAM_START + 8, payload.clone()));
    expected.extend(w8(RX_CON + 1, UINC));

    let mut can = Mcp2518fd::new(Mock::new(&expected));
    let frame = can.receive().unwrap().expect("a frame");
    assert_eq!(frame.id, 0x033);
    assert_eq!(frame.len, 12);
    assert_eq!(frame.payload(), payload.as_slice());
    // Beyond the length the buffer stays zero.
    assert!(frame.data[12..].iter().all(|&b| b == 0));
    can.release().done();
}

/// The DLC codes `send` can produce, against the length table of
/// DS20006027B Table 3-6: every input length maps to the padded length
/// whose DLC the TX object carries.
///
/// Two paddings stack here, and they are not the same thing. The CAN one
/// is the DLC: 9 bytes on the wire become 12, because CAN FD has no DLC
/// for 9. The SPI one is the message-RAM write: RAM accesses are sized
/// in 32-bit words, so a 1-byte payload (a valid DLC of 1) still travels
/// as 4 bytes. Only the DLC reaches the bus.
#[test]
fn dlc_and_length_table() {
    let cases: [(usize, u8, usize); 14] = [
        // input length, DLC written, padded payload length
        (0, 0, 0),
        (1, 1, 1),
        (5, 5, 5),
        (8, 8, 8),
        (9, 9, 12),
        (12, 9, 12),
        (13, 10, 16),
        (16, 10, 16),
        (20, 11, 20),
        (24, 12, 24),
        (32, 13, 32),
        (33, 14, 48),
        (48, 14, 48),
        (64, 15, 64),
    ];
    for (len, dlc, padded) in cases {
        let payload = vec![0x5Au8; len];
        let mut object = object_header(0x010, dlc, true, true, 0);
        // The RAM write rounds the padded payload up to a 32-bit word.
        object.resize(8 + padded.div_ceil(4) * 4, 0);
        object[8..8 + len].copy_from_slice(&payload);

        let mut expected = r32_pair(TX_STA, NOT_FULL_OR_NOT_EMPTY, 0);
        expected.extend(wram(RAM_START, object));
        expected.extend(w8(TX_CON + 1, UINC_TXREQ));

        let mut can = Mcp2518fd::new(Mock::new(&expected));
        can.send(0x010, &payload)
            .unwrap_or_else(|_| panic!("send of {len} bytes"));
        can.release().done();
    }
}
