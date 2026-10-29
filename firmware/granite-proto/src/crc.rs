//! CRC-8 as ADR 0003 decision 1 fixes it: polynomial 0x07, init 0x00,
//! no reflection, no final XOR. That is the SMBus PEC, so the same check
//! can move into I2C hardware that computes it where such hardware
//! exists (the plane's STM32 has it, the controller's LP I2C does not).

/// Polynomial in its normal (MSB-first) form.
pub const POLY: u8 = 0x07;

/// Initial remainder.
pub const INIT: u8 = 0x00;

/// CRC-8 of `data`, starting from [`INIT`].
pub const fn crc8(data: &[u8]) -> u8 {
    crc8_update(INIT, data)
}

/// Feed `data` into a running CRC. Framing computes the CRC over the
/// address byte first, then over the frame, without copying either.
pub const fn crc8_update(mut crc: u8, data: &[u8]) -> u8 {
    let mut i = 0;
    while i < data.len() {
        crc ^= data[i];
        let mut bit = 0;
        while bit < 8 {
            // Shift out the top bit; XOR the polynomial back in when it
            // was set. No table: 8 planes at 1 Hz is a few microseconds.
            crc = if crc & 0x80 != 0 {
                (crc << 1) ^ POLY
            } else {
                crc << 1
            };
            bit += 1;
        }
        i += 1;
    }
    crc
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The canonical CRC-8/SMBUS check value.
    #[test]
    fn check_vector() {
        assert_eq!(crc8(b"123456789"), 0xF4);
    }

    #[test]
    fn short_vectors() {
        assert_eq!(crc8(&[]), 0x00);
        assert_eq!(crc8(&[0x00]), 0x00);
        assert_eq!(crc8(&[0x01]), 0x07);
        assert_eq!(crc8(&[0xFF]), 0xF3);
        // A read request to plane 0x30 for INFO: address, block, index.
        assert_eq!(crc8(&[0x30, 0x01, 0xFF]), 0x07);
    }

    #[test]
    fn update_is_split_safe() {
        let all = crc8(b"123456789");
        let split = crc8_update(crc8(b"1234"), b"56789");
        assert_eq!(all, split);
    }

    /// A single bit flip always changes the CRC of a short frame.
    #[test]
    fn detects_single_bit_flips() {
        let frame = [0x30u8, 0x10, 0xFF, 0x04, 0x4F, 0x01, 0x40, 0x02];
        let good = crc8(&frame);
        for byte in 0..frame.len() {
            for bit in 0..8 {
                let mut bad = frame;
                bad[byte] ^= 1 << bit;
                assert_ne!(crc8(&bad), good, "byte {byte} bit {bit}");
            }
        }
    }
}
