//! MD5, because the switch's session cookie is `admin=md5(user + pass)`
//! (`docs/sodola.md`, "Protocol").
//!
//! Not a security primitive here and not used as one: the scheme it
//! serves is CVE-2026-27754/27755 (a static cookie that never expires),
//! and the controller only has to reproduce what the web UI computes.
//! Written out rather than pulled in so the token is built and tested in
//! `granite-core` like every other payload (ADR 0006 component 15), with
//! no crate that would also land in the firmware's flash budget.

use alloc::string::String;

/// MD5 of `data`, as the 16 raw bytes.
pub fn md5(data: &[u8]) -> [u8; 16] {
    const S: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5,
        9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10,
        15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
    ];
    // floor(abs(sin(i + 1)) * 2^32), RFC 1321's table.
    const K: [u32; 64] = [
        0xd76a_a478, 0xe8c7_b756, 0x2420_70db, 0xc1bd_ceee, 0xf57c_0faf, 0x4787_c62a, 0xa830_4613,
        0xfd46_9501, 0x6980_98d8, 0x8b44_f7af, 0xffff_5bb1, 0x895c_d7be, 0x6b90_1122, 0xfd98_7193,
        0xa679_438e, 0x49b4_0821, 0xf61e_2562, 0xc040_b340, 0x265e_5a51, 0xe9b6_c7aa, 0xd62f_105d,
        0x0244_1453, 0xd8a1_e681, 0xe7d3_fbc8, 0x21e1_cde6, 0xc337_07d6, 0xf4d5_0d87, 0x455a_14ed,
        0xa9e3_e905, 0xfcef_a3f8, 0x676f_02d9, 0x8d2a_4c8a, 0xfffa_3942, 0x8771_f681, 0x6d9d_6122,
        0xfde5_380c, 0xa4be_ea44, 0x4bde_cfa9, 0xf6bb_4b60, 0xbebf_bc70, 0x289b_7ec6, 0xeaa1_27fa,
        0xd4ef_3085, 0x0488_1d05, 0xd9d4_d039, 0xe6db_99e5, 0x1fa2_7cf8, 0xc4ac_5665, 0xf429_2244,
        0x432a_ff97, 0xab94_23a7, 0xfc93_a039, 0x655b_59c3, 0x8f0c_cc92, 0xffef_f47d, 0x8584_5dd1,
        0x6fa8_7e4f, 0xfe2c_e6e0, 0xa301_4314, 0x4e08_11a1, 0xf753_7e82, 0xbd3a_f235, 0x2ad7_d2bb,
        0xeb86_d391,
    ];

    let mut h: [u32; 4] = [0x6745_2301, 0xefcd_ab89, 0x98ba_dcfe, 0x1032_5476];
    let bits = (data.len() as u64).wrapping_mul(8);

    // The message, padded to a multiple of 64 bytes: 0x80, zeroes, then
    // the length in bits little-endian. Walked block by block so only
    // the tail is ever copied.
    let tail_start = data.len() & !63;
    let mut block = [0u8; 64];
    let mut offset = 0usize;
    while offset < tail_start {
        block.copy_from_slice(&data[offset..offset + 64]);
        compress(&mut h, &block, &S, &K);
        offset += 64;
    }
    let rest = &data[tail_start..];
    let mut tail = [0u8; 128];
    tail[..rest.len()].copy_from_slice(rest);
    tail[rest.len()] = 0x80;
    let total = if rest.len() + 9 <= 64 { 64 } else { 128 };
    tail[total - 8..total].copy_from_slice(&bits.to_le_bytes());
    for chunk in tail[..total].as_chunks::<64>().0 {
        block.copy_from_slice(chunk);
        compress(&mut h, &block, &S, &K);
    }

    let mut out = [0u8; 16];
    for (i, word) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_le_bytes());
    }
    out
}

fn compress(h: &mut [u32; 4], block: &[u8; 64], s: &[u32; 64], k: &[u32; 64]) {
    let mut m = [0u32; 16];
    for (i, word) in m.iter_mut().enumerate() {
        *word = u32::from_le_bytes([
            block[i * 4],
            block[i * 4 + 1],
            block[i * 4 + 2],
            block[i * 4 + 3],
        ]);
    }
    let (mut a, mut b, mut c, mut d) = (h[0], h[1], h[2], h[3]);
    for i in 0..64 {
        let (f, g) = match i / 16 {
            0 => ((b & c) | (!b & d), i),
            1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
            2 => (b ^ c ^ d, (3 * i + 5) % 16),
            _ => (c ^ (b | !d), (7 * i) % 16),
        };
        let tmp = d;
        d = c;
        c = b;
        let sum = a
            .wrapping_add(f)
            .wrapping_add(k[i])
            .wrapping_add(m[g]);
        b = b.wrapping_add(sum.rotate_left(s[i]));
        a = tmp;
    }
    h[0] = h[0].wrapping_add(a);
    h[1] = h[1].wrapping_add(b);
    h[2] = h[2].wrapping_add(c);
    h[3] = h[3].wrapping_add(d);
}

/// MD5 of `data` as 32 lowercase hex digits.
pub fn md5_hex(data: &[u8]) -> String {
    let digest = md5(data);
    let mut out = String::with_capacity(32);
    for byte in digest {
        out.push(nibble(byte >> 4));
        out.push(nibble(byte & 0x0f));
    }
    out
}

const fn nibble(n: u8) -> char {
    match n {
        0..=9 => (b'0' + n) as char,
        _ => (b'a' + n - 10) as char,
    }
}

/// The session cookie value the switch expects: `md5(user + password)`.
pub fn session_token(user: &str, password: &str) -> String {
    let mut buf = alloc::vec::Vec::with_capacity(user.len() + password.len());
    buf.extend_from_slice(user.as_bytes());
    buf.extend_from_slice(password.as_bytes());
    md5_hex(&buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc_1321_test_suite() {
        for (input, want) in [
            ("", "d41d8cd98f00b204e9800998ecf8427e"),
            ("a", "0cc175b9c0f1b6a831c399e269772661"),
            ("abc", "900150983cd24fb0d6963f7d28e17f72"),
            ("message digest", "f96b697d7cb7938d525a2f31aaf161d0"),
            (
                "abcdefghijklmnopqrstuvwxyz",
                "c3fcd3d76192e4007dfb496cca67e13b",
            ),
            (
                "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789",
                "d174ab98d277d9f5a5611c2c9f419d9f",
            ),
            (
                "12345678901234567890123456789012345678901234567890123456789012345678901234567890",
                "57edf4a22be3c955ac49da2e2107b67a",
            ),
        ] {
            assert_eq!(md5_hex(input.as_bytes()), want, "md5({input:?})");
        }
    }

    #[test]
    fn long_inputs_cross_the_block_boundaries() {
        // 64 and 119 bytes: the two padding cases, one block and two.
        let a = "a".repeat(64);
        assert_eq!(md5_hex(a.as_bytes()), "014842d480b571495a4a0363793f7367");
        let b = "b".repeat(119);
        assert_eq!(md5_hex(b.as_bytes()), "b4e5105d0b3acae7feda8ec2acbdad57");
    }

    #[test]
    fn the_session_token_is_the_cookie_the_web_ui_sends() {
        // What tools/sodola.py computes for the factory credentials.
        assert_eq!(
            session_token("admin", "admin"),
            "f6fdffe48c908deb0f4c3bd36c032e72"
        );
    }
}
