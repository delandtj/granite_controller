use std::time::{SystemTime, UNIX_EPOCH};

fn main() {
    embuild::espidf::sysenv::output();

    // esp-idf-sys turns every sdkconfig symbol into a cfg, but only the
    // ones it knows at build time land in cargo's check-cfg list. This one
    // appears only when the build includes sdkconfig.defaults.signing, so
    // declare it here or `cargo clippy -D warnings` trips on it.
    println!("cargo::rustc-check-cfg=cfg(esp_idf_secure_signed_on_update_no_secure_boot)");

    // The fleet recovery public key may be baked in at build time
    // (ADR 0001 component 13). option_env! alone would not notice a change.
    println!("cargo:rerun-if-env-changed=FLEET_RECOVERY_PUBKEY");

    // Build timestamp: the clock starts here until SNTP syncs (ADR
    // component 6) and it is the notBefore of the device certificate
    // (component 13).
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    println!("cargo:rustc-env=GRANITE_BUILD_UNIX={secs}");
    println!("cargo:rustc-env=GRANITE_BUILD_X509={}", x509_time(secs));
}

/// Unix seconds as mbedTLS wants a certificate validity: `YYYYMMDDHHMMSS`.
fn x509_time(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}{m:02}{d:02}{:02}{:02}{:02}",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

/// Howard Hinnant's days-to-civil algorithm, for days since 1970-01-01.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}
