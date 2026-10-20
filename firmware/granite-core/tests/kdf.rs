//! The hand-written PBKDF2 and its pause hook. One test function: the
//! hook is process-global, so nothing else in this binary may hash.

use std::sync::atomic::{AtomicU32, Ordering};

use granite_core::api;

static PAUSES: AtomicU32 = AtomicU32::new(0);

fn count_pause() {
    PAUSES.fetch_add(1, Ordering::Relaxed);
}

fn reference(secret: &[u8], salt: &[u8], iters: u32) -> String {
    let mut out = [0u8; api::HASH_BYTES];
    pbkdf2::pbkdf2_hmac::<sha2::Sha256>(secret, salt, iters, &mut out);
    api::hex(&out)
}

#[test]
fn pbkdf2_matches_the_reference_and_pauses_every_chunk() {
    let salt = api::unhex("00112233445566778899aabbccddeeff").unwrap();
    for iters in [1, 2, 499, 500, 501, 1000, 7777] {
        assert_eq!(
            api::pbkdf2_hex(b"correct horse", &salt, iters),
            reference(b"correct horse", &salt, iters),
            "{iters} rounds"
        );
    }

    api::set_kdf_pause(count_pause);
    let hash = api::pbkdf2_hex(b"correct horse", &salt, api::PBKDF2_ITERS);
    assert_eq!(hash, reference(b"correct horse", &salt, api::PBKDF2_ITERS));
    // Rounds 500, 1000, ... 19500: one pause per chunk after the first.
    assert_eq!(
        PAUSES.load(Ordering::Relaxed),
        (api::PBKDF2_ITERS - 1) / api::KDF_CHUNK
    );
}
