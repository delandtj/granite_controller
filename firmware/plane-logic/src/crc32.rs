//! CRC-32 (IEEE 802.3), the one the link asks for in three places: the
//! config hash (ADR 0003 CONFIG 0x80), the firmware image (FW 0x90 /
//! 0x92) and the backup-register set.
//!
//! The link's own frame CRC is CRC-8 and lives in granite-proto; this is
//! the wide one and nothing on the wire shares it, so it stays here.
//! Reflected polynomial 0xEDB88320, init 0xFFFF_FFFF, final XOR
//! 0xFFFF_FFFF: the same CRC-32 `crc32fast`, `zlib` and `gzip` compute,
//! so the host tool that pushes an image needs no special code.
//!
//! Bitwise, no table: a 1 KB table costs more flash than the time it
//! saves on a 55 KB image checked once per update.

/// Starting value of a running CRC-32.
pub const INIT: u32 = 0xFFFF_FFFF;

/// Reflected polynomial.
const POLY: u32 = 0xEDB8_8320;

/// Fold `data` into the running CRC `crc`, which starts at [`INIT`].
///
/// The result is still inverted: pass it to [`finish`] when the last
/// chunk is in.
pub const fn update(mut crc: u32, data: &[u8]) -> u32 {
    let mut i = 0;
    while i < data.len() {
        crc ^= data[i] as u32;
        let mut bit = 0;
        while bit < 8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ POLY
            } else {
                crc >> 1
            };
            bit += 1;
        }
        i += 1;
    }
    crc
}

/// The final value of a running CRC.
pub const fn finish(crc: u32) -> u32 {
    crc ^ 0xFFFF_FFFF
}

/// CRC-32 of one slice.
pub const fn crc32(data: &[u8]) -> u32 {
    finish(update(INIT, data))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_vectors() {
        // The vectors every CRC-32 implementation agrees on.
        assert_eq!(crc32(b""), 0x0000_0000);
        assert_eq!(crc32(b"a"), 0xE8B7_BE43);
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b"The quick brown fox jumps over the lazy dog"), 0x414F_A339);
    }

    #[test]
    fn chunked_equals_whole() {
        let data: [u8; 64] = core::array::from_fn(|i| (i * 7) as u8);
        let whole = crc32(&data);
        let mut running = INIT;
        for chunk in data.chunks(7) {
            running = update(running, chunk);
        }
        assert_eq!(finish(running), whole);
    }
}
