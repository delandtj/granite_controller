//! Plane config: the fields of ADR 0002 decision 10, the two-copy
//! CONFIG page, and the hash the controller compares against.
//!
//! The stored form is granite-proto's own CONFIG records, in a fixed
//! order, wrapped in a small header:
//!
//! ```text
//!  magic u32 | seq u32 | len u16 | records[len] | crc32 u32
//! ```
//!
//! Using the wire records as the storage format means the CONFIG block
//! read, the CONFIG_W write and the page are one encoder, and the config
//! hash (CONFIG 0x80, INFO 0x09) is simply the CRC-32 of the records: it
//! cannot drift from what the controller compares it with.
//!
//! Two copies with a sequence number: the higher valid sequence wins, a
//! write goes to the other copy, so a power loss inside a write leaves
//! the previous set intact.

use granite_proto::{BlinkDetection, Config as CfgRecord, PowerLimit, RecordWriter, RetryPolicy};
use granite_proto::{Records, SLOT_COUNT};

use crate::crc32;
use crate::device::{ConfigStore, DevResult};

/// Bytes of the stored blob: header, the records of eight slots, CRC.
///
/// Eight retry records (7 bytes), eight power limits (5), the blink
/// window (6) is 102 record bytes; the header and CRC add 14. 192 leaves
/// room for a few more records before the page layout has to change.
pub const BLOB_BYTES: usize = 192;

/// Record bytes a config set may use.
pub const RECORD_BYTES: usize = BLOB_BYTES - 14;

/// Copies of the page (ADR 0002 decision 2: 2 x 2 KB).
pub const COPIES: u8 = 2;

/// Blob magic: "PCF1".
const MAGIC: u32 = 0x5043_4631;

/// Default retry maximum: PGD drops in the window a slot may have.
pub const DEFAULT_RETRY_MAX: u8 = 3;
/// Default retry window \[s\].
pub const DEFAULT_RETRY_WINDOW_S: u16 = 60;
/// Default retry action: set the slot OFF and raise an event, the one
/// behaviour ADR 0002 decision 7 describes.
pub const DEFAULT_RETRY_ACTION: u8 = 0;
/// Default per-slot power limit \[0.1 W\]; 0 is "no limit", and ADR 0001
/// leaves the number to the module, so the plane ships without one.
pub const DEFAULT_POWER_LIMIT: u16 = 0;
/// Default shortest blink period \[ms\]: faster than this is noise on
/// the 20 Hz sample.
pub const DEFAULT_BLINK_MIN_MS: u16 = 200;
/// Default longest blink period \[ms\]. ADR 0002 ("What an expert would
/// ask"): a 0.5 Hz breathing LED needs up to 4 s before it is called
/// blinking rather than off.
pub const DEFAULT_BLINK_MAX_MS: u16 = 4000;

/// One slot's retry policy (CONFIG 0x81 / CTRL 0x42 without the slot).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Retry {
    /// PGD drops in the window the slot may have.
    pub max: u8,
    /// The window \[s\].
    pub window_s: u16,
    /// What the plane does past the maximum.
    pub action: u8,
}

impl Default for Retry {
    fn default() -> Self {
        Retry {
            max: DEFAULT_RETRY_MAX,
            window_s: DEFAULT_RETRY_WINDOW_S,
            action: DEFAULT_RETRY_ACTION,
        }
    }
}

/// The whole plane config.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlaneConfig {
    /// Retry policy per slot.
    pub retry: [Retry; SLOT_COUNT],
    /// Power limit per slot \[0.1 W\], 0 for none.
    pub power_limit: [u16; SLOT_COUNT],
    /// The window in which an LED counts as blinking.
    pub blink: BlinkDetection,
}

impl Default for PlaneConfig {
    fn default() -> Self {
        PlaneConfig {
            retry: [Retry::default(); SLOT_COUNT],
            power_limit: [DEFAULT_POWER_LIMIT; SLOT_COUNT],
            blink: BlinkDetection {
                min_period_ms: DEFAULT_BLINK_MIN_MS,
                max_period_ms: DEFAULT_BLINK_MAX_MS,
            },
        }
    }
}

impl PlaneConfig {
    /// Encode the set as CONFIG records in their canonical order, so the
    /// hash depends on the values and not on how they arrived.
    ///
    /// The hash record (0x80) is not part of the set: it is the hash.
    pub fn encode(&self, out: &mut [u8]) -> usize {
        let mut w = RecordWriter::new(out);
        for slot in 0..SLOT_COUNT {
            let r = self.retry[slot];
            let _ = RetryPolicy {
                slot: slot as u8,
                max: r.max,
                window_s: r.window_s,
                action: r.action,
            }
            .encode(&mut w);
        }
        for slot in 0..SLOT_COUNT {
            let _ = PowerLimit {
                slot: slot as u8,
                watts: self.power_limit[slot],
            }
            .encode(&mut w);
        }
        let _ = self.blink.encode(&mut w);
        w.len()
    }

    /// The hash of the set: CRC-32 of its canonical records.
    pub fn hash(&self) -> u32 {
        let mut buf = [0u8; RECORD_BYTES];
        let n = self.encode(&mut buf);
        crc32::crc32(&buf[..n])
    }

    /// Apply one CONFIG record, as a CONFIG_W set does.
    ///
    /// [`CfgRecord::Hash`] is read-only and [`CfgRecord::Commit`] is the
    /// set's end, so neither changes a field: both answer `true` (taken)
    /// without touching anything.
    pub fn apply(&mut self, record: CfgRecord) -> bool {
        match record {
            CfgRecord::RetryPolicy(p) => {
                if (p.slot as usize) < SLOT_COUNT {
                    self.retry[p.slot as usize] = Retry {
                        max: p.max,
                        window_s: p.window_s,
                        action: p.action,
                    };
                    true
                } else {
                    false
                }
            }
            CfgRecord::PowerLimit(p) => {
                if (p.slot as usize) < SLOT_COUNT {
                    self.power_limit[p.slot as usize] = p.watts;
                    true
                } else {
                    false
                }
            }
            CfgRecord::BlinkDetection(b) => {
                if b.min_period_ms == 0 || b.max_period_ms < b.min_period_ms {
                    return false;
                }
                self.blink = b;
                true
            }
            CfgRecord::Hash(_) | CfgRecord::Commit => true,
        }
    }

    /// Read a set back out of its records.
    fn from_records(payload: &[u8]) -> Self {
        let mut cfg = PlaneConfig::default();
        for record in Records::new(payload) {
            let Ok((tag, value)) = record else { break };
            if let Ok(r) = CfgRecord::decode(tag, value) {
                cfg.apply(r);
            }
        }
        cfg
    }
}

/// A config set in a page, with the sequence number it was written with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoredConfig {
    /// The set.
    pub config: PlaneConfig,
    /// Its sequence number; the higher valid copy wins.
    pub seq: u32,
}

/// Encode a blob for a page.
fn encode_blob(config: &PlaneConfig, seq: u32, out: &mut [u8; BLOB_BYTES]) -> usize {
    let n = config.encode(&mut out[10..10 + RECORD_BYTES]);
    out[0..4].copy_from_slice(&MAGIC.to_le_bytes());
    out[4..8].copy_from_slice(&seq.to_le_bytes());
    out[8..10].copy_from_slice(&(n as u16).to_le_bytes());
    let crc = crc32::crc32(&out[..10 + n]);
    out[10 + n..14 + n].copy_from_slice(&crc.to_le_bytes());
    14 + n
}

/// Read a blob back, or `None` when it is blank or damaged.
fn decode_blob(blob: &[u8]) -> Option<StoredConfig> {
    if blob.len() < 14 || u32::from_le_bytes(blob[0..4].try_into().ok()?) != MAGIC {
        return None;
    }
    let seq = u32::from_le_bytes(blob[4..8].try_into().ok()?);
    let len = u16::from_le_bytes(blob[8..10].try_into().ok()?) as usize;
    if len > RECORD_BYTES || blob.len() < 14 + len {
        return None;
    }
    let want = u32::from_le_bytes(blob[10 + len..14 + len].try_into().ok()?);
    if crc32::crc32(&blob[..10 + len]) != want {
        return None;
    }
    Some(StoredConfig {
        config: PlaneConfig::from_records(&blob[10..10 + len]),
        seq,
    })
}

/// Load the newer valid copy, or `None` when neither is readable (a
/// blank plane: the defaults apply and the controller writes its set).
pub fn load<S: ConfigStore>(store: &mut S) -> Option<StoredConfig> {
    let mut best: Option<StoredConfig> = None;
    for copy in 0..COPIES {
        let mut blob = [0u8; BLOB_BYTES];
        if store.read(copy, &mut blob).is_err() {
            continue;
        }
        if let Some(found) = decode_blob(&blob)
            && best.is_none_or(|b| found.seq.wrapping_sub(b.seq) < u32::MAX / 2)
        {
            best = Some(found);
        }
    }
    best
}

/// Store `config` in the copy the newer one is not in, one past its
/// sequence number. Returns what is now stored.
pub fn store<S: ConfigStore>(
    store: &mut S,
    current: Option<StoredConfig>,
    config: &PlaneConfig,
) -> DevResult<StoredConfig> {
    let seq = current.map_or(1, |c| c.seq.wrapping_add(1));
    let copy = (seq % COPIES as u32) as u8;
    let mut blob = [0u8; BLOB_BYTES];
    let n = encode_blob(config, seq, &mut blob);
    store.write(copy, &blob[..n])?;
    Ok(StoredConfig {
        config: *config,
        seq,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two pages in RAM.
    struct Pages([[u8; BLOB_BYTES]; COPIES as usize]);

    impl ConfigStore for Pages {
        fn read(&mut self, copy: u8, out: &mut [u8]) -> DevResult<()> {
            let n = out.len().min(BLOB_BYTES);
            out[..n].copy_from_slice(&self.0[copy as usize][..n]);
            Ok(())
        }

        fn write(&mut self, copy: u8, data: &[u8]) -> DevResult<()> {
            self.0[copy as usize] = [0xFF; BLOB_BYTES];
            self.0[copy as usize][..data.len()].copy_from_slice(data);
            Ok(())
        }
    }

    fn pages() -> Pages {
        Pages([[0xFF; BLOB_BYTES]; COPIES as usize])
    }

    #[test]
    fn the_default_set_fits_the_blob() {
        let mut buf = [0u8; RECORD_BYTES];
        let n = PlaneConfig::default().encode(&mut buf);
        assert_eq!(n, 8 * 7 + 8 * 5 + 6);
        assert!(n <= RECORD_BYTES);
    }

    #[test]
    fn the_hash_follows_the_values_only() {
        let a = PlaneConfig::default();
        let mut b = PlaneConfig::default();
        assert_eq!(a.hash(), b.hash());
        b.power_limit[4] = 7500;
        assert_ne!(a.hash(), b.hash());
        let mut c = PlaneConfig::default();
        // Applied in the other order, the hash is the same.
        c.apply(CfgRecord::PowerLimit(PowerLimit {
            slot: 4,
            watts: 7500,
        }));
        assert_eq!(b.hash(), c.hash());
    }

    #[test]
    fn a_blank_store_has_no_set() {
        let mut p = pages();
        assert_eq!(load(&mut p), None);
    }

    #[test]
    fn copies_alternate_and_the_newer_wins() {
        let mut p = pages();
        let mut cfg = PlaneConfig::default();
        cfg.retry[2].max = 9;
        let first = store(&mut p, None, &cfg).unwrap();
        assert_eq!(first.seq, 1);
        assert_eq!(load(&mut p), Some(first));

        cfg.retry[2].max = 1;
        let second = store(&mut p, Some(first), &cfg).unwrap();
        assert_eq!(second.seq, 2);
        assert_eq!(load(&mut p).unwrap().config.retry[2].max, 1);

        // The older copy is still there: damaging the newer one falls
        // back to it.
        p.0[0] = [0xFF; BLOB_BYTES];
        assert_eq!(load(&mut p).unwrap().config.retry[2].max, 9);
    }

    #[test]
    fn a_bad_blink_window_is_refused() {
        let mut cfg = PlaneConfig::default();
        assert!(!cfg.apply(CfgRecord::BlinkDetection(BlinkDetection {
            min_period_ms: 0,
            max_period_ms: 10,
        })));
        assert!(!cfg.apply(CfgRecord::BlinkDetection(BlinkDetection {
            min_period_ms: 900,
            max_period_ms: 100,
        })));
        assert_eq!(cfg.blink.min_period_ms, DEFAULT_BLINK_MIN_MS);
    }
}
