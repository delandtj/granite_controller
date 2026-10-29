//! The config the controller wants a plane to have.
//!
//! ADR 0004 decision 5: "config push (compare the config hash, write
//! on mismatch)". The hash is the plane's own CRC-32 over its stored
//! set (ADR 0003 CONFIG 0x80) and its layout is the plane's business,
//! so the controller cannot compute it: it learns the hash a set
//! produces by writing the set once and reading INFO back.
//!
//! So a desired config carries the records and, optionally, the hash
//! the plane reported the last time this exact set was stored. The
//! firmware may persist that hash with its own config, which saves one
//! write per plane per controller boot; without it the client pushes
//! once after it finds the plane, learns the hash, and then leaves the
//! plane alone.
//!
//! A CONFIG_W write has no sequence number and therefore no result
//! code (ADR 0003 decision 3 gives the block none): the plane applies
//! and stores the set only when it ends with the commit record, raises
//! `config stored` and shows a new hash. The client re-reads INFO
//! after a push and compares; [`PUSH_TRIES`] pushes without a new hash
//! mean the plane refused the set.

use alloc::vec::Vec;

use granite_proto::Config as ConfigRecord;

/// How often the client writes the same set before it calls the plane's
/// silence a refusal.
pub const PUSH_TRIES: u32 = 3;

/// The config set the controller wants on one plane.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DesiredConfig {
    /// The records, without the commit record (the client appends it).
    records: Vec<ConfigRecord>,
    /// The hash the plane reported after it stored this set, when the
    /// client (or the firmware) has seen it.
    stored_hash: Option<u32>,
    /// Pushes since the hash last matched.
    tries: u32,
    /// The client has given up on this set.
    refused: bool,
}

impl DesiredConfig {
    /// A desired set. `stored_hash` is the hash the plane is expected
    /// to report once it holds this set, if the firmware kept it from
    /// an earlier run; `None` means "push it once and find out".
    pub fn new(records: Vec<ConfigRecord>, stored_hash: Option<u32>) -> Self {
        DesiredConfig {
            records: records
                .into_iter()
                .filter(|r| !matches!(r, ConfigRecord::Commit | ConfigRecord::Hash(_)))
                .collect(),
            stored_hash,
            tries: 0,
            refused: false,
        }
    }

    /// The records, without the commit.
    pub fn records(&self) -> &[ConfigRecord] {
        &self.records
    }

    /// The hash the plane reported for this set, if it is known.
    pub const fn stored_hash(&self) -> Option<u32> {
        self.stored_hash
    }

    /// True when the plane's hash does not match what this set
    /// produces, so the set has to be written.
    pub const fn needs_push(&self, plane_hash: u32) -> bool {
        if self.refused || self.records.is_empty() {
            return false;
        }
        match self.stored_hash {
            Some(h) => h != plane_hash,
            None => true,
        }
    }

    /// True when the client has stopped trying.
    pub const fn is_refused(&self) -> bool {
        self.refused
    }

    /// Count a push. True while there are tries left.
    pub fn note_push(&mut self) -> bool {
        self.tries = self.tries.saturating_add(1);
        if self.tries >= PUSH_TRIES {
            self.refused = true;
        }
        !self.refused
    }

    /// The plane reports `hash` now: remember it as this set's hash and
    /// start the try counter over.
    pub fn note_stored(&mut self, hash: u32) {
        self.stored_hash = Some(hash);
        self.tries = 0;
        self.refused = false;
    }

    /// The whole write, commit record last. Without the commit the
    /// plane discards the set (ADR 0003 CONFIG 0x8F).
    pub fn write_set(&self) -> Vec<ConfigRecord> {
        let mut out = self.records.clone();
        out.push(ConfigRecord::Commit);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use granite_proto::{BlinkDetection, PowerLimit};

    fn set() -> DesiredConfig {
        DesiredConfig::new(
            alloc::vec![
                ConfigRecord::PowerLimit(PowerLimit {
                    slot: 0,
                    watts: 7_500
                }),
                ConfigRecord::BlinkDetection(BlinkDetection {
                    min_period_ms: 300,
                    max_period_ms: 3_000
                }),
                // A commit the caller slipped in is dropped: the client
                // owns where the set ends.
                ConfigRecord::Commit,
            ],
            None,
        )
    }

    #[test]
    fn a_set_is_pushed_until_the_hash_matches() {
        let mut config = set();
        assert_eq!(config.records().len(), 2);
        assert!(config.needs_push(0x1234), "no hash is known yet");
        assert!(matches!(
            config.write_set().last(),
            Some(ConfigRecord::Commit)
        ));

        assert!(config.note_push());
        config.note_stored(0xABCD);
        assert!(!config.needs_push(0xABCD));
        assert!(config.needs_push(0x1111), "the plane lost the set");
    }

    #[test]
    fn a_plane_that_never_takes_the_set_is_given_up_on() {
        let mut config = set();
        for _ in 0..PUSH_TRIES - 1 {
            assert!(config.note_push());
        }
        assert!(!config.note_push());
        assert!(config.is_refused());
        assert!(!config.needs_push(0x1234));
    }

    #[test]
    fn an_empty_set_is_never_pushed() {
        let config = DesiredConfig::new(Vec::new(), None);
        assert!(!config.needs_push(0));
    }
}
