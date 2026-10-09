//! Identity and recovery (ADR 0001 component 13).
//!
//! Everything a sealed board can be found and rescued by:
//!
//! - the device id `granite-<mac6>`, derived from the eFuse **Ethernet**
//!   MAC, which is the address the user notes before the board goes into
//!   the fluid,
//! - a per-device recovery token (20 random bytes, base32, grouped), made
//!   at first boot and at every factory reset, kept in the `factory`
//!   namespace so a factory reset does not destroy the way back in, and
//!   printed on the USB console at generation and on request,
//! - the fleet recovery public key (ECDSA P-256, PEM): optionally baked in
//!   at build time from `FLEET_RECOVERY_PUBKEY`, otherwise set at
//!   commissioning. [`issue_nonce`] and [`verify_fleet_recovery`] are the
//!   `/recover` half of it,
//! - the device TLS certificate: ECDSA P-256, self-signed, generated on
//!   first boot with mbedTLS, stored in the `secrets` namespace, with its
//!   SHA-256 fingerprint on the console and the `/id` endpoint.
//!
//! All crypto here goes through mbedTLS via `esp-idf-sys`. ESP-IDF v5.5.5
//! defines `MBEDTLS_X509_CREATE_C` and `MBEDTLS_X509_CRT_WRITE_C`
//! unconditionally in `components/mbedtls/port/include/mbedtls/esp_config.h`
//! and `CONFIG_MBEDTLS_PEM_WRITE_C` is on by default, so certificate
//! writing needs no extra Kconfig symbol; `sdkconfig.defaults` only has to
//! keep the P-256 curve and ECDSA enabled, which it already does.

use std::ffi::CString;

use esp_idf_svc::sys::{
    EspError, MBEDTLS_X509_CRT_VERSION_3, MBEDTLS_X509_KU_DIGITAL_SIGNATURE,
    MBEDTLS_X509_KU_KEY_AGREEMENT, esp, esp_fill_random, esp_mac_type_t_ESP_MAC_ETH, esp_read_mac,
    mbedtls_ecp_gen_key, mbedtls_ecp_group_id_MBEDTLS_ECP_DP_SECP256R1, mbedtls_ecp_keypair,
    mbedtls_md_type_t_MBEDTLS_MD_SHA256, mbedtls_pk_context, mbedtls_pk_free,
    mbedtls_pk_info_from_type, mbedtls_pk_init, mbedtls_pk_parse_public_key, mbedtls_pk_setup,
    mbedtls_pk_type_t_MBEDTLS_PK_ECKEY, mbedtls_pk_verify, mbedtls_pk_write_key_pem,
    mbedtls_sha256, mbedtls_x509write_cert, mbedtls_x509write_crt_der, mbedtls_x509write_crt_free,
    mbedtls_x509write_crt_init, mbedtls_x509write_crt_pem,
    mbedtls_x509write_crt_set_authority_key_identifier,
    mbedtls_x509write_crt_set_basic_constraints, mbedtls_x509write_crt_set_issuer_key,
    mbedtls_x509write_crt_set_issuer_name, mbedtls_x509write_crt_set_key_usage,
    mbedtls_x509write_crt_set_md_alg, mbedtls_x509write_crt_set_serial_raw,
    mbedtls_x509write_crt_set_subject_key, mbedtls_x509write_crt_set_subject_key_identifier,
    mbedtls_x509write_crt_set_subject_name, mbedtls_x509write_crt_set_validity,
    mbedtls_x509write_crt_set_version,
};

use super::store::{FLEET_PUBKEY_KEY, RECOVERY_TOKEN_KEY, Store};

/// Length of the recovery token in random bytes. 20 bytes = 32 base32
/// characters = 160 bits.
pub const RECOVERY_TOKEN_BYTES: usize = 20;

/// Certificate lifetime in years (ADR: 10).
const CERT_YEARS: i64 = 10;

/// Fleet recovery public key baked in at build time, if the build set
/// `FLEET_RECOVERY_PUBKEY` (PEM). `build.rs` asks cargo to rerun when the
/// variable changes.
pub const BUILT_IN_FLEET_PUBKEY: Option<&str> = option_env!("FLEET_RECOVERY_PUBKEY");

/// Build timestamp, Unix seconds, from `build.rs`.
pub const BUILD_UNIX: u64 = match u64::from_str_radix(env!("GRANITE_BUILD_UNIX"), 10) {
    Ok(v) => v,
    Err(_) => 0,
};

/// Build timestamp as mbedTLS wants it in a certificate: `YYYYMMDDHHMMSS`.
pub const BUILD_X509_TIME: &str = env!("GRANITE_BUILD_X509");

// ---------------------------------------------------------------------------
// Small encodings. Hand-written to keep the dependency list short.
// ---------------------------------------------------------------------------

/// RFC 4648 base32 alphabet.
const BASE32: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

/// Base32 of `bytes`, no padding, in groups of 4 separated by `-`.
///
/// 20 bytes come out as `XXXX-XXXX-XXXX-XXXX-XXXX-XXXX-XXXX-XXXX`, which is
/// what the user writes down next to the MAC.
pub fn base32_grouped(bytes: &[u8]) -> String {
    let mut out = String::new();
    let mut acc: u32 = 0;
    let mut bits = 0u32;
    let mut chars = 0usize;
    for b in bytes {
        acc = (acc << 8) | u32::from(*b);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            let idx = ((acc >> bits) & 0x1f) as usize;
            if chars > 0 && chars.is_multiple_of(4) {
                out.push('-');
            }
            out.push(BASE32[idx] as char);
            chars += 1;
        }
    }
    if bits > 0 {
        let idx = ((acc << (5 - bits)) & 0x1f) as usize;
        if chars > 0 && chars.is_multiple_of(4) {
            out.push('-');
        }
        out.push(BASE32[idx] as char);
    }
    out
}

/// Lowercase hex.
pub fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// Parse lowercase or uppercase hex. `None` on any non-hex character or an
/// odd length.
pub fn from_hex(s: &str) -> Option<Vec<u8>> {
    let s = s.trim();
    if !s.len().is_multiple_of(2) {
        return None;
    }
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(s.len() / 2);
    for pair in bytes.chunks(2) {
        let hi = (pair[0] as char).to_digit(16)?;
        let lo = (pair[1] as char).to_digit(16)?;
        out.push((hi * 16 + lo) as u8);
    }
    Some(out)
}

/// `len` bytes from the hardware RNG.
pub fn random_bytes(len: usize) -> Vec<u8> {
    let mut buf = vec![0u8; len];
    // esp_fill_random is the ESP-IDF entropy source; it is seeded by the
    // RF/ADC noise sources and is safe to call this early.
    unsafe { esp_fill_random(buf.as_mut_ptr().cast(), buf.len()) };
    buf
}

/// SHA-256 through mbedTLS (hardware accelerated on the C6).
pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    let rc = unsafe { mbedtls_sha256(data.as_ptr(), data.len(), out.as_mut_ptr(), 0) };
    if rc != 0 {
        log::error!("mbedtls_sha256 failed with {rc}");
    }
    out
}

/// mbedTLS RNG callback backed by `esp_fill_random`.
///
/// ESP-IDF's own TLS code uses the same source; a CTR-DRBG on top would
/// add an entropy context to carry around for no extra strength here.
unsafe extern "C" fn rng(
    _ctx: *mut core::ffi::c_void,
    out: *mut core::ffi::c_uchar,
    len: usize,
) -> core::ffi::c_int {
    unsafe { esp_fill_random(out.cast(), len) };
    0
}

// ---------------------------------------------------------------------------
// Device id
// ---------------------------------------------------------------------------

/// The eFuse Ethernet MAC. Every identity in the firmware derives from it,
/// and it is what the user notes before a board is sealed.
pub fn eth_mac() -> Result<[u8; 6], EspError> {
    let mut mac = [0u8; 6];
    esp!(unsafe { esp_read_mac(mac.as_mut_ptr(), esp_mac_type_t_ESP_MAC_ETH) })?;
    Ok(mac)
}

/// `granite-<last 3 MAC bytes in hex>`.
pub fn default_device_id(mac: &[u8; 6]) -> String {
    format!("granite-{:02x}{:02x}{:02x}", mac[3], mac[4], mac[5])
}

/// `aa:bb:cc:dd:ee:ff`.
pub fn mac_string(mac: &[u8; 6]) -> String {
    format!(
        "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
    )
}

// ---------------------------------------------------------------------------
// Identity
// ---------------------------------------------------------------------------

/// Everything derived once at boot and then read-only.
#[derive(Debug, Clone)]
pub struct Identity {
    /// eFuse Ethernet MAC.
    pub mac: [u8; 6],
    /// Device id: `sys.device_id` if set, else `granite-<mac6>`.
    pub device_id: String,
    /// Recovery token, base32, grouped.
    pub recovery_token: String,
    /// True when this boot generated the token (first boot or after a
    /// factory reset). The console prints it once in that case.
    pub recovery_token_fresh: bool,
    /// Fleet recovery public key, PEM, if one is known.
    pub fleet_pubkey_pem: Option<String>,
    /// Device certificate, PEM.
    pub cert_pem: String,
    /// Device certificate private key, PEM. Never leaves the device.
    pub key_pem: String,
    /// SHA-256 of the certificate DER, lowercase hex.
    pub cert_sha256: String,
    /// True when this boot generated the certificate.
    pub cert_fresh: bool,
}

impl Identity {
    /// The fingerprint in the `aa:bb:...` form people compare by eye.
    pub fn cert_fingerprint_colons(&self) -> String {
        self.cert_sha256
            .as_bytes()
            .chunks(2)
            .map(|c| String::from_utf8_lossy(c).to_string())
            .collect::<Vec<_>>()
            .join(":")
    }

    /// Fleet key fingerprint (SHA-256 of the PEM body), for `fleet-key`.
    pub fn fleet_key_fingerprint(&self) -> Option<String> {
        self.fleet_pubkey_pem
            .as_ref()
            .map(|pem| hex(&sha256(pem.trim().as_bytes())))
    }
}

/// Bring the identity up, generating what is missing.
///
/// `device_id_override` is `sys.device_id` from the config; empty means
/// "derive it from the MAC".
pub fn init(store: &mut Store, device_id_override: &str) -> Result<Identity, EspError> {
    let mac = eth_mac()?;
    let device_id = if device_id_override.trim().is_empty() {
        default_device_id(&mac)
    } else {
        device_id_override.trim().to_string()
    };

    log::info!("device id {device_id}, ethernet mac {}", mac_string(&mac));

    // -- recovery token --------------------------------------------------
    let existing = store.factory_get(RECOVERY_TOKEN_KEY).unwrap_or(None);
    let (recovery_token, recovery_token_fresh) = match existing {
        Some(t) if !t.trim().is_empty() => (t, false),
        _ => {
            let token = base32_grouped(&random_bytes(RECOVERY_TOKEN_BYTES));
            match store.factory_set(RECOVERY_TOKEN_KEY, &token) {
                Ok(()) => log::warn!(
                    "recovery token generated and stored in the factory namespace: {token}"
                ),
                Err(e) => log::error!(
                    "recovery token generated but could not be stored ({e}); it will change on \
                     the next boot: {token}"
                ),
            }
            log::warn!("write it down together with the MAC; it is the way back into this board");
            (token, true)
        }
    };

    // -- fleet recovery public key ---------------------------------------
    let stored_fleet = store
        .factory_get(FLEET_PUBKEY_KEY)
        .unwrap_or(None)
        .filter(|p| !p.trim().is_empty());
    let fleet_pubkey_pem = match stored_fleet {
        Some(pem) => Some(pem),
        None => match BUILT_IN_FLEET_PUBKEY {
            Some(pem) if !pem.trim().is_empty() => {
                // A build-time key is trusted by a freshly flashed board;
                // persist it so replacing it later is one code path.
                if let Err(e) = store.factory_set(FLEET_PUBKEY_KEY, pem) {
                    log::error!("fleet recovery key from the build could not be stored: {e}");
                }
                log::info!("fleet recovery key taken from the build (FLEET_RECOVERY_PUBKEY)");
                Some(pem.to_string())
            }
            _ => {
                log::info!("no fleet recovery key; set one on the Security page before sealing");
                None
            }
        },
    };

    // -- device certificate ----------------------------------------------
    let mut secrets = store.load_secrets();
    let stored_cert = store.device_cert().unwrap_or(None);
    let (cert_pem, key_pem, cert_fresh) = match (stored_cert, secrets.device_key_pem.clone()) {
        (Some(cert), key) if !cert.trim().is_empty() && !key.trim().is_empty() => {
            log::info!("device certificate loaded from nvs");
            (cert, key, false)
        }
        _ => {
            log::info!("generating the device certificate (ECDSA P-256, self-signed)");
            let generated = generate_self_signed(&device_id)?;
            if let Err(e) = store.save_device_cert(&generated.cert_pem) {
                log::error!("device certificate could not be stored: {e}");
            }
            secrets.device_key_pem = generated.key_pem.clone();
            if let Err(e) = store.save_secrets(&secrets) {
                log::error!("device key could not be stored: {e}");
            }
            (generated.cert_pem, generated.key_pem, true)
        }
    };

    let cert_sha256 = cert_fingerprint(&cert_pem);
    log::info!("device cert sha256 {cert_sha256}");

    Ok(Identity {
        mac,
        device_id,
        recovery_token,
        recovery_token_fresh,
        fleet_pubkey_pem,
        cert_pem,
        key_pem,
        cert_sha256,
        cert_fresh,
    })
}

/// SHA-256 of a certificate's DER, taken from its PEM. This is the number
/// the status page and `/id` publish.
pub fn cert_fingerprint(pem: &str) -> String {
    match pem_body(pem) {
        Some(der) => hex(&sha256(&der)),
        None => String::new(),
    }
}

/// Decode the base64 body of the first PEM block.
fn pem_body(pem: &str) -> Option<Vec<u8>> {
    let mut b64 = String::new();
    let mut inside = false;
    for line in pem.lines() {
        let line = line.trim();
        if line.starts_with("-----BEGIN") {
            inside = true;
            continue;
        }
        if line.starts_with("-----END") {
            break;
        }
        if inside {
            b64.push_str(line);
        }
    }
    if b64.is_empty() {
        return None;
    }
    base64_decode(&b64)
}

pub fn base64_decode(s: &str) -> Option<Vec<u8>> {
    fn val(c: u8) -> Option<u32> {
        match c {
            b'A'..=b'Z' => Some(u32::from(c - b'A')),
            b'a'..=b'z' => Some(u32::from(c - b'a') + 26),
            b'0'..=b'9' => Some(u32::from(c - b'0') + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    let mut acc = 0u32;
    let mut bits = 0u32;
    for c in s.bytes() {
        if c == b'=' {
            break;
        }
        let v = val(c)?;
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((acc >> bits) & 0xff) as u8);
        }
    }
    Some(out)
}

// ---------------------------------------------------------------------------
// Certificate generation
// ---------------------------------------------------------------------------

/// A freshly made key pair and certificate.
pub struct GeneratedCert {
    /// Certificate, PEM.
    pub cert_pem: String,
    /// Private key, PEM (PKCS#8).
    pub key_pem: String,
}

/// RAII wrapper so every error path frees the mbedTLS contexts.
struct Pk(mbedtls_pk_context);

impl Pk {
    fn new() -> Self {
        let mut ctx: mbedtls_pk_context = unsafe { core::mem::zeroed() };
        unsafe { mbedtls_pk_init(&mut ctx) };
        Pk(ctx)
    }
}

impl Drop for Pk {
    fn drop(&mut self) {
        unsafe { mbedtls_pk_free(&mut self.0) };
    }
}

struct Crt(mbedtls_x509write_cert);

impl Crt {
    fn new() -> Self {
        let mut ctx: mbedtls_x509write_cert = unsafe { core::mem::zeroed() };
        unsafe { mbedtls_x509write_crt_init(&mut ctx) };
        Crt(ctx)
    }
}

impl Drop for Crt {
    fn drop(&mut self) {
        unsafe { mbedtls_x509write_crt_free(&mut self.0) };
    }
}

fn mbed_err(what: &str, rc: core::ffi::c_int) -> EspError {
    log::error!("{what} failed with mbedtls error {rc} (-0x{:04x})", -rc);
    EspError::from_infallible::<{ esp_idf_svc::sys::ESP_FAIL }>()
}

/// Generate an ECDSA P-256 key and a self-signed certificate for
/// `device_id`: CN = the device id, 10 years, `notBefore` = build time.
pub fn generate_self_signed(device_id: &str) -> Result<GeneratedCert, EspError> {
    let mut key = Pk::new();
    unsafe {
        let info = mbedtls_pk_info_from_type(mbedtls_pk_type_t_MBEDTLS_PK_ECKEY);
        let rc = mbedtls_pk_setup(&mut key.0, info);
        if rc != 0 {
            return Err(mbed_err("mbedtls_pk_setup", rc));
        }
        let keypair = key.0.private_pk_ctx as *mut mbedtls_ecp_keypair;
        let rc = mbedtls_ecp_gen_key(
            mbedtls_ecp_group_id_MBEDTLS_ECP_DP_SECP256R1,
            keypair,
            Some(rng),
            core::ptr::null_mut(),
        );
        if rc != 0 {
            return Err(mbed_err("mbedtls_ecp_gen_key", rc));
        }
    }

    // The key first: if PEM writing is going to fail, fail before spending
    // time on the certificate.
    let mut key_buf = vec![0u8; 2048];
    unsafe {
        let rc = mbedtls_pk_write_key_pem(&key.0, key_buf.as_mut_ptr(), key_buf.len());
        if rc != 0 {
            return Err(mbed_err("mbedtls_pk_write_key_pem", rc));
        }
    }
    let key_pem = cstr_from_buf(&key_buf);

    let subject = CString::new(format!("CN={device_id}"))
        .map_err(|_| EspError::from_infallible::<{ esp_idf_svc::sys::ESP_ERR_INVALID_ARG }>())?;
    let (not_before, not_after) = validity_window();
    let not_before_c = CString::new(not_before.clone()).expect("ascii");
    let not_after_c = CString::new(not_after.clone()).expect("ascii");

    // 20 byte serial, positive and without a leading zero byte.
    let mut serial = random_bytes(20);
    serial[0] = (serial[0] & 0x7f) | 0x01;

    let mut crt = Crt::new();
    unsafe {
        mbedtls_x509write_crt_set_version(&mut crt.0, MBEDTLS_X509_CRT_VERSION_3 as i32);
        mbedtls_x509write_crt_set_md_alg(&mut crt.0, mbedtls_md_type_t_MBEDTLS_MD_SHA256);
        mbedtls_x509write_crt_set_subject_key(&mut crt.0, &mut key.0);
        mbedtls_x509write_crt_set_issuer_key(&mut crt.0, &mut key.0);

        let rc = mbedtls_x509write_crt_set_subject_name(&mut crt.0, subject.as_ptr());
        if rc != 0 {
            return Err(mbed_err("mbedtls_x509write_crt_set_subject_name", rc));
        }
        // Self-signed: issuer == subject.
        let rc = mbedtls_x509write_crt_set_issuer_name(&mut crt.0, subject.as_ptr());
        if rc != 0 {
            return Err(mbed_err("mbedtls_x509write_crt_set_issuer_name", rc));
        }
        let rc = mbedtls_x509write_crt_set_serial_raw(&mut crt.0, serial.as_mut_ptr(), serial.len());
        if rc != 0 {
            return Err(mbed_err("mbedtls_x509write_crt_set_serial_raw", rc));
        }
        let rc = mbedtls_x509write_crt_set_validity(
            &mut crt.0,
            not_before_c.as_ptr(),
            not_after_c.as_ptr(),
        );
        if rc != 0 {
            return Err(mbed_err("mbedtls_x509write_crt_set_validity", rc));
        }
        // A leaf that is also its own issuer: CA true would be a lie no
        // client checks, CA false is what a self-signed leaf should say.
        let rc = mbedtls_x509write_crt_set_basic_constraints(&mut crt.0, 0, 0);
        if rc != 0 {
            return Err(mbed_err("mbedtls_x509write_crt_set_basic_constraints", rc));
        }
        let rc = mbedtls_x509write_crt_set_key_usage(
            &mut crt.0,
            MBEDTLS_X509_KU_DIGITAL_SIGNATURE | MBEDTLS_X509_KU_KEY_AGREEMENT,
        );
        if rc != 0 {
            return Err(mbed_err("mbedtls_x509write_crt_set_key_usage", rc));
        }
        let rc = mbedtls_x509write_crt_set_subject_key_identifier(&mut crt.0);
        if rc != 0 {
            return Err(mbed_err("mbedtls_x509write_crt_set_subject_key_identifier", rc));
        }
        let rc = mbedtls_x509write_crt_set_authority_key_identifier(&mut crt.0);
        if rc != 0 {
            return Err(mbed_err(
                "mbedtls_x509write_crt_set_authority_key_identifier",
                rc,
            ));
        }
        // TODO(ADR 0001 8): a subjectAltName with the hostname and the IP
        // would stop one browser warning out of several on a self-signed
        // cert; mbedtls_x509write_crt_set_subject_alternative_name needs an
        // mbedtls_x509_san_list built by hand and the IP is not known at
        // generation time. The fingerprint is the trust anchor here.
    }

    let mut cert_buf = vec![0u8; 4096];
    unsafe {
        let rc = mbedtls_x509write_crt_pem(
            &mut crt.0,
            cert_buf.as_mut_ptr(),
            cert_buf.len(),
            Some(rng),
            core::ptr::null_mut(),
        );
        if rc != 0 {
            return Err(mbed_err("mbedtls_x509write_crt_pem", rc));
        }
    }
    let cert_pem = cstr_from_buf(&cert_buf);

    Ok(GeneratedCert { cert_pem, key_pem })
}

/// `notBefore` = build time, `notAfter` = build time + 10 years.
fn validity_window() -> (String, String) {
    let before = BUILD_X509_TIME.to_string();
    let after = match before.get(0..4).and_then(|y| y.parse::<i64>().ok()) {
        Some(year) => format!("{:04}{}", year + CERT_YEARS, &before[4..]),
        // No usable build stamp: fall back to a window that is certainly
        // valid rather than refusing to make a certificate at all.
        None => String::from("20360101000000"),
    };
    let before = if before.len() == 14 {
        before
    } else {
        String::from("20260101000000")
    };
    (before, after)
}

/// Read a NUL-terminated string out of a buffer mbedTLS wrote into.
fn cstr_from_buf(buf: &[u8]) -> String {
    let end = buf.iter().position(|b| *b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..end]).into_owned()
}

/// DER of a certificate written from a PEM, for a fingerprint over a
/// freshly built cert without a PEM round trip. Kept for the OTA and
/// status paths that already hold DER.
pub fn der_fingerprint(der: &[u8]) -> String {
    hex(&sha256(der))
}

/// Write the DER of a certificate under construction. Only used by tests
/// and by [`generate_self_signed`]'s diagnostics; mbedTLS writes DER at the
/// **end** of the buffer and returns its length.
#[allow(dead_code)]
fn crt_der(crt: &mut Crt) -> Option<Vec<u8>> {
    let mut buf = vec![0u8; 2048];
    let len = unsafe {
        mbedtls_x509write_crt_der(
            &mut crt.0,
            buf.as_mut_ptr(),
            buf.len(),
            Some(rng),
            core::ptr::null_mut(),
        )
    };
    if len <= 0 {
        return None;
    }
    let len = len as usize;
    let start = buf.len() - len;
    Some(buf[start..].to_vec())
}

// ---------------------------------------------------------------------------
// Fleet recovery
// ---------------------------------------------------------------------------
//
// The nonce for `POST /recover` is issued and consumed by
// `granite_core::api::AuthState` (fresh per `/id`, 5 min, single use) and
// the API composes the signed message as
// `device || nonce || "factory-reset"`. This module therefore only has to
// check a signature over a message it is handed; an own nonce table here
// would be a second, disagreeing authority on the same question.

/// Check a fleet-key signature over `message`.
///
/// `sig` is what the host tool produced: base64 of a DER ECDSA
/// signature. Returns false when no fleet key is configured, when the key
/// does not parse, when `sig` is not base64 DER, or when the signature
/// does not verify.
pub fn verify_fleet_sig(fleet_pubkey_pem: Option<&str>, message: &[u8], sig: &str) -> bool {
    let Some(pem) = fleet_pubkey_pem.map(str::trim).filter(|p| !p.is_empty()) else {
        log::warn!("fleet recovery attempt but no fleet key is configured");
        return false;
    };
    let Some(der) = base64_decode(sig.trim()) else {
        log::warn!("fleet recovery signature is not base64");
        return false;
    };
    if der.len() < 8 || der[0] != 0x30 {
        log::warn!("fleet recovery signature is not DER");
        return false;
    }
    let digest = sha256(message);

    let mut key = Pk::new();
    let mut pem_z = pem.as_bytes().to_vec();
    pem_z.push(0);
    let rc = unsafe { mbedtls_pk_parse_public_key(&mut key.0, pem_z.as_ptr(), pem_z.len()) };
    if rc != 0 {
        log::error!("fleet recovery key does not parse (mbedtls {rc})");
        return false;
    }
    let rc = unsafe {
        mbedtls_pk_verify(
            &mut key.0,
            mbedtls_md_type_t_MBEDTLS_MD_SHA256,
            digest.as_ptr(),
            digest.len(),
            der.as_ptr(),
            der.len(),
        )
    };
    if rc == 0 {
        log::warn!("fleet recovery signature accepted");
        true
    } else {
        log::warn!("fleet recovery signature rejected (mbedtls {rc})");
        false
    }
}

/// Compare a typed recovery token with the stored one, ignoring the
/// grouping dashes, spaces and case the user may or may not reproduce.
pub fn recovery_token_matches(stored: &str, typed: &str) -> bool {
    fn canon(s: &str) -> String {
        s.chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .map(|c| c.to_ascii_uppercase())
            .collect()
    }
    let a = canon(stored);
    let b = canon(typed);
    !a.is_empty() && super::constant_time_eq(a.as_bytes(), b.as_bytes())
}

/// Replace the fleet recovery key. The caller is responsible for the
/// authorisation (admin password or the old fleet key, per the ADR).
pub fn set_fleet_pubkey(store: &mut Store, pem: &str) -> Result<(), String> {
    let trimmed = pem.trim();
    if trimmed.is_empty() {
        store
            .factory_set(FLEET_PUBKEY_KEY, "")
            .map_err(|e| e.to_string())?;
        log::warn!("fleet recovery key cleared");
        return Ok(());
    }
    // Refuse anything mbedTLS will not load later.
    let mut key = Pk::new();
    let mut pem_z = trimmed.as_bytes().to_vec();
    pem_z.push(0);
    let rc = unsafe { mbedtls_pk_parse_public_key(&mut key.0, pem_z.as_ptr(), pem_z.len()) };
    if rc != 0 {
        return Err(format!("not a usable public key (mbedtls {rc})"));
    }
    store
        .factory_set(FLEET_PUBKEY_KEY, trimmed)
        .map_err(|e| e.to_string())?;
    log::warn!("fleet recovery key replaced");
    Ok(())
}
