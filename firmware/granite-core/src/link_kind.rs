//! Which plane link a firmware image is built for, carried inside the
//! image (ADR 0006, architecture component 7: "the image carries its
//! link kind; the OTA upload refuses an image whose kind differs").
//!
//! One image per link (`link-canfd` / `link-i2c` in granite-fw), so an
//! image flashed onto the wrong board comes up with a link it cannot
//! drive: the FD build would talk SPI to an I2C isolator, the I2C build
//! would hold a chip select as if it were SDA. The guard belongs in the
//! upload path, the last moment where a wrong image can be refused
//! without a rollback.
//!
//! This module is the format and the search; the firmware owns the two
//! ends of it (the marker it links into its own image, and the upload
//! that scans a stream). Both are here so they are tested on the host,
//! where the firmware's own tests cannot run.
//!
//! # What the marker is, and why not the app descriptor
//!
//! A 24-byte tagged struct ([`tag_bytes`]) in its own read-only
//! section, found in the uploaded bytes by searching for a 16-byte magic
//! and then checking the structure: a format version, the kind, the
//! kind's one's complement, a zero reserved byte and a CRC-32 over the
//! 20 bytes before it. Four independent checks, so a chance occurrence
//! of the magic in an asset cannot be read as a kind.
//!
//! The obvious alternative is `esp_app_desc_t`, which ESP-IDF puts at a
//! fixed offset (0x20) of every image and whose `version` esp-idf-svc
//! already reads back for the running slot. It was not taken:
//!
//! - `version` is `CONFIG_APP_PROJECT_VER` in `sdkconfig.defaults`, a
//!   file the ESP-IDF build reads before the firmware crate's
//!   `build.rs` runs, so a cargo feature cannot change it; `build.rs`
//!   also pins it to the crate version, because esp-idf-svc's
//!   `FirmwareInfo::version` is 24 bytes and a longer one makes every
//!   slot read fail.
//! - `project_name` is the ESP-IDF project's (`libespidf` for an
//!   esp-idf-sys build) and has no sdkconfig symbol at all.
//! - `secure_version` belongs to anti-rollback, and the descriptor's
//!   reserved fields are ESP-IDF's to use next release.
//!
//! The marker's cost is one pass over the uploaded stream, which the
//! upload is already hashing byte by byte.
//!
//! # The magic appears once
//!
//! An uploaded granite image contains this very module, so a scanner
//! holding the magic as a constant would find *two* copies: the marker
//! and its own comparison value, in whatever order the linker chose. So
//! the constant here is the magic with every byte xored by `OBFUSCATE`
//! and [`magic`] builds the real bytes at run time; the 16 bytes the
//! scanner looks for exist in an image exactly once, in its marker.
//!
//! ```
//! use granite_core::link_kind::{LinkKind, TagScan, tag_bytes};
//!
//! // An image for the CAN FD link, uploaded to a board running it.
//! let mut scan = TagScan::new();
//! scan.feed(&[0u8; 512]);
//! scan.feed(&tag_bytes(LinkKind::CanFd));
//! assert_eq!(scan.verdict(LinkKind::CanFd), Ok(LinkKind::CanFd));
//! assert!(scan.verdict(LinkKind::I2c).is_err());
//! ```

/// Which link an image drives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum LinkKind {
    /// CAN FD through the plane row (ADR 0005), MCP2518FD on the SPI
    /// bus: the FD generation.
    CanFd = 1,
    /// The isolated I2C link of rev D (ADR 0003 / 0004).
    I2c = 2,
}

impl LinkKind {
    /// Lowercase wire name, as the console and the API print it.
    pub const fn as_str(self) -> &'static str {
        match self {
            LinkKind::CanFd => "canfd",
            LinkKind::I2c => "i2c",
        }
    }

    /// The wire byte.
    pub const fn code(self) -> u8 {
        self as u8
    }

    /// The kind a wire byte names.
    pub const fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(LinkKind::CanFd),
            2 => Some(LinkKind::I2c),
            _ => None,
        }
    }

    /// The other one, which an upload must refuse.
    pub const fn other(self) -> Self {
        match self {
            LinkKind::CanFd => LinkKind::I2c,
            LinkKind::I2c => LinkKind::CanFd,
        }
    }
}

impl core::fmt::Display for LinkKind {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Format of the marker. A later field layout bumps this; an upload
/// refuses a version it does not know rather than guessing.
pub const TAG_VERSION: u8 = 1;

/// Bytes of the marker.
pub const TAG_LEN: usize = 24;

/// Bytes of the magic.
pub const MAGIC_LEN: usize = 16;

/// Every byte of the magic is stored xored with this, so the magic
/// itself is not a constant in the image (see the module docs).
const OBFUSCATE: u8 = 0xA5;

/// The magic, xored byte by byte with [`OBFUSCATE`].
const MAGIC_OBFUSCATED: [u8; MAGIC_LEN] = [
    0x9C, 0xBC, 0x5E, 0x1B, 0xF0, 0x6E, 0xA9, 0x47, 0x2D, 0x8A, 0x33, 0xD4, 0x61, 0x0F, 0xB2, 0x56,
];

/// The 16 bytes a marker starts with, built rather than stored.
pub const fn magic() -> [u8; MAGIC_LEN] {
    let mut out = [0u8; MAGIC_LEN];
    let mut i = 0;
    while i < MAGIC_LEN {
        out[i] = MAGIC_OBFUSCATED[i] ^ OBFUSCATE;
        i += 1;
    }
    out
}

/// The marker of an image that drives `kind`: the bytes the firmware
/// links into its own read-only data.
///
/// Layout: `magic[16] | version | kind | !kind | 0 | crc32 LE[4]`.
pub const fn tag_bytes(kind: LinkKind) -> [u8; TAG_LEN] {
    let mut out = [0u8; TAG_LEN];
    let m = magic();
    let mut i = 0;
    while i < MAGIC_LEN {
        out[i] = m[i];
        i += 1;
    }
    out[MAGIC_LEN] = TAG_VERSION;
    out[MAGIC_LEN + 1] = kind.code();
    out[MAGIC_LEN + 2] = !kind.code();
    out[MAGIC_LEN + 3] = 0;
    let mut body = [0u8; TAG_LEN - 4];
    let mut i = 0;
    while i < TAG_LEN - 4 {
        body[i] = out[i];
        i += 1;
    }
    let crc = crc32(&body).to_le_bytes();
    out[TAG_LEN - 4] = crc[0];
    out[TAG_LEN - 3] = crc[1];
    out[TAG_LEN - 2] = crc[2];
    out[TAG_LEN - 1] = crc[3];
    out
}

/// CRC-32 (IEEE 802.3, reflected, init 0xFFFFFFFF, final xor): the
/// usual one, bit by bit because it runs over 20 bytes per candidate and
/// a 1 KB table is not worth the flash.
pub const fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    let mut i = 0;
    while i < data.len() {
        crc ^= data[i] as u32;
        let mut bit = 0;
        while bit < 8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
            bit += 1;
        }
        i += 1;
    }
    !crc
}

/// The kind a candidate marker names, if every check passes.
pub fn read_tag(bytes: &[u8]) -> Option<LinkKind> {
    if bytes.len() < TAG_LEN || bytes[..MAGIC_LEN] != magic() {
        return None;
    }
    let version = bytes[MAGIC_LEN];
    let kind = bytes[MAGIC_LEN + 1];
    let kind_not = bytes[MAGIC_LEN + 2];
    let reserved = bytes[MAGIC_LEN + 3];
    if version != TAG_VERSION || kind_not != !kind || reserved != 0 {
        return None;
    }
    let mut crc = [0u8; 4];
    crc.copy_from_slice(&bytes[TAG_LEN - 4..TAG_LEN]);
    if u32::from_le_bytes(crc) != crc32(&bytes[..TAG_LEN - 4]) {
        return None;
    }
    LinkKind::from_code(kind)
}

/// Why an image is not one this board may boot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TagError {
    /// No marker at all: an image from before the marker existed, or
    /// not a granite image. Refused: a wrong-link image from before the
    /// split is exactly the case the marker exists for, and an image
    /// that cannot say what it drives cannot be trusted to drive this
    /// board's link.
    Missing {
        /// What this board runs.
        running: LinkKind,
    },
    /// Two markers that disagree. Nothing builds such an image; it is
    /// refused rather than resolved.
    Conflicting,
    /// A marker for the other link.
    Mismatch {
        /// What the image says it drives.
        image: LinkKind,
        /// What this board runs.
        running: LinkKind,
    },
}

impl core::fmt::Display for TagError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            TagError::Missing { running } => write!(
                f,
                "image carries no plane-link marker (built before the link kinds split, or \
                 not a granite image); this board runs the {running} link"
            ),
            TagError::Conflicting => f.write_str("image carries two different plane-link markers"),
            TagError::Mismatch { image, running } => write!(
                f,
                "image is a {image} plane-link build and this board runs the {running} link"
            ),
        }
    }
}

/// Finds the marker in a stream of image bytes, chunk by chunk.
///
/// The upload path has no idea where in the image the linker put the
/// marker, so the scan is a search for the magic over the whole stream.
/// Chunks arrive in any size, including one chunk of the whole image, so
/// nothing is copied but the few bytes of the seam between two chunks.
#[derive(Debug, Default)]
pub struct TagScan {
    /// The tail of the last chunk, for a marker that straddles two.
    carry: [u8; TAG_LEN - 1],
    /// Bytes of `carry` that are real.
    carry_len: usize,
    /// The kind found so far.
    found: Option<LinkKind>,
    /// A second marker named a different kind.
    conflict: bool,
    /// Markers seen (the same kind twice is not an error).
    tags: u32,
    /// Bytes fed.
    len: u64,
}

impl TagScan {
    /// A scanner that has seen nothing.
    pub const fn new() -> Self {
        TagScan {
            carry: [0u8; TAG_LEN - 1],
            carry_len: 0,
            found: None,
            conflict: false,
            tags: 0,
            len: 0,
        }
    }

    /// Feed the next chunk of the image.
    pub fn feed(&mut self, chunk: &[u8]) {
        self.len = self.len.saturating_add(chunk.len() as u64);
        // The seam first: candidates that start in the carried tail and
        // end in this chunk. Only start positions inside the carry are
        // looked at, so nothing is examined twice.
        if self.carry_len > 0 {
            let mut seam = [0u8; 2 * TAG_LEN];
            let n = self.carry_len;
            seam[..n].copy_from_slice(&self.carry[..n]);
            let take = chunk.len().min(TAG_LEN);
            seam[n..n + take].copy_from_slice(&chunk[..take]);
            self.scan(&seam[..n + take], n);
        }
        self.scan(chunk, chunk.len());

        // Keep the last TAG_LEN - 1 bytes of everything seen so far.
        let keep = TAG_LEN - 1;
        if chunk.len() >= keep {
            self.carry.copy_from_slice(&chunk[chunk.len() - keep..]);
            self.carry_len = keep;
        } else {
            let shift = (self.carry_len + chunk.len()).saturating_sub(keep);
            self.carry.copy_within(shift..self.carry_len, 0);
            let left = self.carry_len - shift;
            self.carry[left..left + chunk.len()].copy_from_slice(chunk);
            self.carry_len = left + chunk.len();
        }
    }

    /// Look at every start position below `starts` in `bytes`.
    fn scan(&mut self, bytes: &[u8], starts: usize) {
        let magic = magic();
        let first = magic[0];
        let limit = starts.min(bytes.len());
        for i in 0..limit {
            if bytes[i] != first || bytes.len() - i < TAG_LEN {
                continue;
            }
            if let Some(kind) = read_tag(&bytes[i..i + TAG_LEN]) {
                self.tags = self.tags.saturating_add(1);
                match self.found {
                    Some(seen) if seen != kind => self.conflict = true,
                    _ => self.found = Some(kind),
                }
            }
        }
    }

    /// The kind the image carries, if one was found.
    pub const fn kind(&self) -> Option<LinkKind> {
        self.found
    }

    /// Markers seen.
    pub const fn tags(&self) -> u32 {
        self.tags
    }

    /// Bytes scanned.
    pub const fn len(&self) -> u64 {
        self.len
    }

    /// True while nothing has been fed.
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Whether a board running `running` may boot the scanned image.
    pub fn verdict(&self, running: LinkKind) -> Result<LinkKind, TagError> {
        if self.conflict {
            return Err(TagError::Conflicting);
        }
        match self.found {
            None => Err(TagError::Missing { running }),
            Some(kind) if kind != running => Err(TagError::Mismatch {
                image: kind,
                running,
            }),
            Some(kind) => Ok(kind),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use alloc::vec::Vec;

    #[test]
    fn a_marker_reads_back_as_the_kind_it_was_built_for() {
        for kind in [LinkKind::CanFd, LinkKind::I2c] {
            let tag = tag_bytes(kind);
            assert_eq!(read_tag(&tag), Some(kind));
            assert_eq!(&tag[..MAGIC_LEN], &magic());
            assert_eq!(tag[MAGIC_LEN], TAG_VERSION);
            assert_eq!(tag[MAGIC_LEN + 1], kind.code());
            assert_eq!(tag[MAGIC_LEN + 2], !kind.code());
            assert_eq!(kind.other().other(), kind);
        }
        // The magic is not a constant in the image.
        assert_ne!(magic(), MAGIC_OBFUSCATED);
        // CRC-32 against the known value of the ASCII digits.
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn an_image_for_the_other_link_is_refused() {
        let mut scan = TagScan::new();
        scan.feed(&[0x5A; 1000]);
        scan.feed(&tag_bytes(LinkKind::I2c));
        scan.feed(&[0u8; 500]);
        assert_eq!(scan.verdict(LinkKind::I2c), Ok(LinkKind::I2c));
        assert_eq!(
            scan.verdict(LinkKind::CanFd),
            Err(TagError::Mismatch {
                image: LinkKind::I2c,
                running: LinkKind::CanFd
            })
        );
        assert_eq!(scan.tags(), 1);
        assert_eq!(scan.len(), 1524);
    }

    #[test]
    fn an_image_without_a_marker_is_refused() {
        let mut scan = TagScan::new();
        assert!(scan.is_empty());
        scan.feed(&[0xFF; 4096]);
        assert_eq!(
            scan.verdict(LinkKind::CanFd),
            Err(TagError::Missing {
                running: LinkKind::CanFd
            })
        );
        assert_eq!(scan.kind(), None);

        // So is one whose marker is damaged, in every field that guards
        // it.
        for broken in [0, MAGIC_LEN, MAGIC_LEN + 1, MAGIC_LEN + 2, MAGIC_LEN + 3, TAG_LEN - 1] {
            let mut tag = tag_bytes(LinkKind::CanFd);
            tag[broken] ^= 0x01;
            let mut scan = TagScan::new();
            scan.feed(&tag);
            assert!(scan.verdict(LinkKind::CanFd).is_err(), "byte {broken}");
        }
    }

    /// The scan must find a marker however the upload is cut up: one
    /// byte at a time, in the seam between two chunks, or in one chunk
    /// of the whole image.
    #[test]
    fn a_marker_is_found_at_any_chunk_boundary() {
        let tag = tag_bytes(LinkKind::CanFd);
        for split in 0..=TAG_LEN {
            let mut image = vec![0x11u8; 70];
            image.extend_from_slice(&tag);
            image.extend_from_slice(&[0x22; 70]);
            let cut = 70 + split;
            let mut scan = TagScan::new();
            scan.feed(&image[..cut]);
            scan.feed(&image[cut..]);
            assert_eq!(
                scan.verdict(LinkKind::CanFd),
                Ok(LinkKind::CanFd),
                "split at {split}"
            );
            assert_eq!(scan.tags(), 1, "counted twice at {split}");
        }

        // Byte by byte, and with chunks shorter than the marker.
        for chunk in [1usize, 2, 7, TAG_LEN - 1] {
            let mut image = vec![0u8; 13];
            image.extend_from_slice(&tag);
            image.extend_from_slice(&[9; 5]);
            let mut scan = TagScan::new();
            for part in image.chunks(chunk) {
                scan.feed(part);
            }
            assert_eq!(
                scan.verdict(LinkKind::CanFd),
                Ok(LinkKind::CanFd),
                "chunks of {chunk}"
            );
            assert_eq!(scan.tags(), 1, "counted twice at chunks of {chunk}");
        }

        // Empty chunks change nothing.
        let mut scan = TagScan::new();
        scan.feed(&[]);
        scan.feed(&tag);
        scan.feed(&[]);
        assert_eq!(scan.verdict(LinkKind::CanFd), Ok(LinkKind::CanFd));
    }

    #[test]
    fn two_markers_that_disagree_are_refused() {
        let mut scan = TagScan::new();
        scan.feed(&tag_bytes(LinkKind::CanFd));
        scan.feed(&[0; 9]);
        scan.feed(&tag_bytes(LinkKind::I2c));
        assert_eq!(scan.verdict(LinkKind::CanFd), Err(TagError::Conflicting));
        assert_eq!(scan.verdict(LinkKind::I2c), Err(TagError::Conflicting));

        // The same marker twice is not a conflict: an image may well
        // carry the bytes twice (a copy in an asset, a duplicated
        // section).
        let mut scan = TagScan::new();
        scan.feed(&tag_bytes(LinkKind::CanFd));
        scan.feed(&tag_bytes(LinkKind::CanFd));
        assert_eq!(scan.verdict(LinkKind::CanFd), Ok(LinkKind::CanFd));
        assert_eq!(scan.tags(), 2);
    }

    /// A stream of noise must not produce a kind, however much of it
    /// there is: this is the check that the four guards are enough.
    #[test]
    fn noise_never_names_a_kind() {
        let mut state = 0x9E37_79B9u32;
        let mut rng = move || {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state
        };
        let mut scan = TagScan::new();
        let m = magic();
        for _ in 0..2_000 {
            let mut chunk: Vec<u8> = (0..64).map(|_| rng() as u8).collect();
            // Sprinkle in near misses: the magic followed by noise, and
            // the magic with one byte wrong.
            chunk.extend_from_slice(&m);
            chunk.extend_from_slice(&[0u8; 8]);
            let mut almost = m;
            almost[3] ^= 0x80;
            chunk.extend_from_slice(&almost);
            scan.feed(&chunk);
        }
        assert_eq!(scan.kind(), None);
        assert_eq!(scan.tags(), 0);
    }
}
