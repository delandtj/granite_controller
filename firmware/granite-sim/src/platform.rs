//! The [`granite_core::api`] traits, implemented against the simulation.
//!
//! Each of these is the host twin of something the firmware's platform
//! layer (`granite-fw/src/platform`) provides on the board: NVS, the
//! identity and recovery material, the commit-confirm machinery, the OTA
//! partition writer and the hardware RNG. They are separate structs on
//! purpose: [`crate::sim::Sim`] hands out one mutable borrow of each at
//! the same time when it builds an `ApiCtx`.

use std::collections::BTreeMap;

use granite_core::api::{
    Identity, MqttStatus, NetControl, NetStatus, OtaFinish, OtaSink, OtaStatus, Platform, Rng,
    SlotInfo, StagedInfo, Store,
};
use granite_core::config::{Config, Section, Secrets};
use granite_core::hal::BootReason;
use granite_core::msg::{Event, EventKind};

use crate::scenario::Scenario;

/// In-memory NVS: the config sections and the secrets blob.
#[derive(Debug, Default)]
pub struct SimStore {
    /// The secrets namespace.
    pub secrets: Secrets,
    /// One JSON blob per section, exactly as NVS holds them.
    pub sections: BTreeMap<Section, String>,
    /// Every save, in order, for the tests.
    pub saves: Vec<Section>,
}

impl Store for SimStore {
    fn load_secrets(&mut self) -> Secrets {
        self.secrets.clone()
    }

    fn save_secrets(&mut self, secrets: &Secrets) -> Result<(), String> {
        self.secrets = secrets.clone();
        Ok(())
    }

    fn save_section(&mut self, section: Section, json: &str) -> Result<(), String> {
        self.sections.insert(section, String::from(json));
        self.saves.push(section);
        Ok(())
    }
}

/// Firmware version, uptime, network and OTA summaries, the log ring and
/// the event stream.
#[derive(Debug)]
pub struct SimPlatform {
    /// Version string.
    pub fw: String,
    /// Monotonic now, updated by the tick loop.
    pub now_ms: u64,
    /// What the status page shows for the network.
    pub net: NetStatus,
    /// What it shows for the broker.
    pub mqtt: MqttStatus,
    /// What the firmware page shows.
    pub ota: OtaStatus,
    /// The 16 KB log ring, as lines.
    pub log: Vec<String>,
    /// Events, newest last; the MQTT publisher would consume these.
    pub events: Vec<Event>,
    /// Set by `ota_rollback`, acted on by the simulator.
    pub rollback_requested: bool,
    /// Set by `ota_mark_valid`.
    pub marked_valid: bool,
}

/// Most log lines the ring keeps.
pub const LOG_LINES: usize = 500;

impl SimPlatform {
    /// A platform view for a scenario.
    pub fn new(scenario: &Scenario) -> Self {
        SimPlatform {
            fw: scenario.fw.clone(),
            now_ms: 0,
            net: NetStatus {
                link_up: scenario.hardware.link_up,
                ip_mode: String::from("dhcp"),
                ip: String::from("127.0.0.1"),
                netmask: String::from("255.255.255.0"),
                gateway: String::from("127.0.0.1"),
                dns: vec![String::from("127.0.0.1")],
                hostname: scenario.device_id.clone(),
                dhcp_fallback: false,
                sntp_synced: true,
            },
            mqtt: MqttStatus::default(),
            ota: OtaStatus {
                running: String::from("ota_0"),
                state: String::from("valid"),
                pending_verify: false,
                validate_left_s: None,
                slots: vec![
                    SlotInfo {
                        label: String::from("factory"),
                        state: String::from("valid"),
                        version: scenario.fw.clone(),
                        size: 2_621_440,
                    },
                    SlotInfo {
                        label: String::from("ota_0"),
                        state: String::from("running"),
                        version: scenario.fw.clone(),
                        size: 2_621_440,
                    },
                    SlotInfo {
                        label: String::from("ota_1"),
                        state: String::from("empty"),
                        version: String::new(),
                        size: 2_621_440,
                    },
                ],
                key_id: String::from("sim-no-signing-key"),
                rollback_available: false,
            },
            log: Vec::new(),
            events: Vec::new(),
            rollback_requested: false,
            marked_valid: false,
        }
    }

    /// Append a log line with the monotonic timestamp the board uses.
    pub fn log_line(&mut self, level: &str, message: impl AsRef<str>) {
        let line = format!(
            "{:>8}.{:03} {level:<5} {}",
            self.now_ms / 1000,
            self.now_ms % 1000,
            message.as_ref()
        );
        self.log.push(line);
        if self.log.len() > LOG_LINES {
            let drop = self.log.len() - LOG_LINES;
            self.log.drain(0..drop);
        }
    }

    /// Record an event and log it.
    pub fn push_event(&mut self, kind: EventKind) {
        let event = Event::new(self.now_ms, kind);
        if let Ok(json) = event.to_json() {
            self.log_line("info", json);
        }
        self.events.push(event);
        if self.events.len() > LOG_LINES {
            let drop = self.events.len() - LOG_LINES;
            self.events.drain(0..drop);
        }
    }
}

impl Platform for SimPlatform {
    fn fw_version(&self) -> String {
        self.fw.clone()
    }

    fn uptime_s(&self) -> u32 {
        (self.now_ms / 1000) as u32
    }

    fn boot_reason(&self) -> BootReason {
        BootReason::PowerOn
    }

    fn free_heap(&self) -> u32 {
        // A plausible figure, so the page shows something sensible.
        180_000
    }

    fn net(&self) -> NetStatus {
        self.net.clone()
    }

    fn mqtt(&self) -> MqttStatus {
        self.mqtt.clone()
    }

    fn ota(&self) -> OtaStatus {
        self.ota.clone()
    }

    fn log_tail(&self, lines: usize) -> Vec<String> {
        let start = self.log.len().saturating_sub(lines);
        self.log[start..].to_vec()
    }

    fn ota_rollback(&mut self) -> Result<(), String> {
        if !self.ota.rollback_available {
            return Err(String::from("no previous image to roll back to"));
        }
        self.rollback_requested = true;
        Ok(())
    }

    fn ota_mark_valid(&mut self) -> Result<(), String> {
        if !self.ota.pending_verify {
            return Err(String::from("the running image is not on probation"));
        }
        self.ota.pending_verify = false;
        self.ota.state = String::from("valid");
        self.ota.validate_left_s = None;
        self.marked_valid = true;
        Ok(())
    }

    fn event(&mut self, kind: EventKind) {
        self.push_event(kind);
    }
}

/// Device id, MAC, certificate fingerprint, the recovery token and the
/// fleet recovery key.
#[derive(Debug)]
pub struct SimIdentity {
    /// Device id.
    pub device_id: String,
    /// MAC as `aa:bb:cc:dd:ee:ff`.
    pub mac: String,
    /// The HTTPS certificate in PEM, so its fingerprint matches what the
    /// browser sees.
    pub cert_pem: String,
    /// Its private key.
    pub key_pem: String,
    /// Per-device recovery token.
    pub recovery_token: String,
    /// True once it has been shown on the setup page.
    pub token_shown: bool,
    /// Fleet recovery public key, PEM.
    pub fleet_pem: String,
}

impl SimIdentity {
    /// Identity from a scenario and the serving certificate.
    pub fn new(scenario: &Scenario, cert_pem: &str, key_pem: &str) -> Self {
        let token = if scenario.recovery_token.trim().is_empty() {
            random_token()
        } else {
            scenario.recovery_token.clone()
        };
        SimIdentity {
            device_id: scenario.device_id.clone(),
            mac: scenario.mac.clone(),
            cert_pem: String::from(cert_pem),
            key_pem: String::from(key_pem),
            recovery_token: token,
            token_shown: false,
            fleet_pem: String::new(),
        }
    }
}

/// 20 random bytes in the grouped base32 the ADR asks for.
pub fn random_token() -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut raw = [0u8; 20];
    SimRng.fill(&mut raw);
    let mut out = String::new();
    for (i, b) in raw.iter().enumerate() {
        if i > 0 && i % 4 == 0 {
            out.push('-');
        }
        out.push(ALPHABET[(*b as usize) % 32] as char);
    }
    out
}

/// SHA-256 of a PEM blob, hex, as the fingerprint the page shows.
pub fn sha256_hex(data: &str) -> String {
    use sha2::Digest as _;
    let digest = sha2::Sha256::digest(data.as_bytes());
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

impl Identity for SimIdentity {
    fn device_id(&self) -> String {
        self.device_id.clone()
    }

    fn mac(&self) -> String {
        self.mac.clone()
    }

    fn cert_sha256(&self) -> String {
        sha256_hex(&self.cert_pem)
    }

    fn recovery_token_once(&mut self) -> Option<String> {
        if self.token_shown {
            None
        } else {
            self.token_shown = true;
            Some(self.recovery_token.clone())
        }
    }

    fn verify_recovery_token(&mut self, token: &str) -> bool {
        let want = self.recovery_token.replace('-', "").to_ascii_uppercase();
        let got = token.replace('-', "").to_ascii_uppercase();
        !want.is_empty() && want == got
    }

    fn verify_fleet_sig(&self, message: &[u8], sig: &str) -> bool {
        crate::keys::verify(&self.fleet_pem, message, sig)
    }

    fn set_fleet_pubkey(&mut self, pem: &str) -> Result<(), String> {
        crate::keys::parse_public_pem(pem)?;
        self.fleet_pem = String::from(pem);
        Ok(())
    }

    fn set_device_cert(&mut self, cert_pem: &str, key_pem: &str) -> Result<(), String> {
        if !cert_pem.contains("BEGIN CERTIFICATE") {
            return Err(String::from("cert_pem is not a PEM certificate"));
        }
        if !key_pem.contains("PRIVATE KEY") {
            return Err(String::from("key_pem is not a PEM private key"));
        }
        self.cert_pem = String::from(cert_pem);
        self.key_pem = String::from(key_pem);
        Ok(())
    }
}

/// Commit-confirm, the host version: the staged sections are held here
/// and [`crate::sim::Sim`] persists or reverts them.
#[derive(Debug, Default)]
pub struct SimNet {
    /// Monotonic now, from the tick loop.
    pub now_ms: u64,
    /// The confirm window in seconds.
    pub window_s: u32,
    /// What is staged.
    pub staged: Vec<(Section, String)>,
    /// When the automatic revert happens.
    pub deadline_ms: Option<u64>,
    /// The configuration as it was before the first staged change.
    pub previous: Option<Config>,
    /// Set by `confirm`, consumed by the simulator.
    pub confirmed: bool,
    /// Set by `revert` or by the deadline, consumed by the simulator.
    pub reverted: bool,
}

impl SimNet {
    /// A commit-confirm view with the window from the config.
    pub fn new(window_s: u32) -> Self {
        SimNet {
            window_s,
            ..Default::default()
        }
    }

    /// Seconds left before the revert.
    pub fn seconds_left(&self) -> u32 {
        match self.deadline_ms {
            Some(d) => ((d.saturating_sub(self.now_ms)) / 1000) as u32,
            None => 0,
        }
    }

    /// True once the window has run out.
    pub fn expired(&self) -> bool {
        matches!(self.deadline_ms, Some(d) if self.now_ms >= d)
    }

    /// Forget whatever is staged.
    pub fn clear(&mut self) {
        self.staged.clear();
        self.deadline_ms = None;
        self.previous = None;
    }
}

impl NetControl for SimNet {
    fn stage_and_apply(&mut self, section: Section, json: &str) -> Result<u32, String> {
        self.staged.retain(|(s, _)| *s != section);
        self.staged.push((section, String::from(json)));
        let window = if self.window_s == 0 { 300 } else { self.window_s };
        self.deadline_ms = Some(self.now_ms + u64::from(window) * 1000);
        Ok(window)
    }

    fn confirm(&mut self) -> bool {
        if self.staged.is_empty() {
            return false;
        }
        self.confirmed = true;
        true
    }

    fn revert(&mut self) -> bool {
        if self.staged.is_empty() {
            return false;
        }
        self.reverted = true;
        true
    }

    fn staged(&self) -> Option<StagedInfo> {
        if self.staged.is_empty() {
            return None;
        }
        Some(StagedInfo {
            sections: self.staged.iter().map(|(s, _)| *s).collect(),
            seconds_left: self.seconds_left(),
        })
    }
}

/// The OTA sink: a buffer plus the minimal image sanity check the board's
/// `esp_ota_write` would do.
#[derive(Debug, Default)]
pub struct SimOta {
    /// What has been pushed so far.
    pub buffer: Vec<u8>,
    /// Content-Length, when the client sent one.
    pub expected: Option<u64>,
    /// Set once an image was accepted; the simulator moves it into the
    /// slot table.
    pub finished: Option<OtaFinish>,
    /// True while an upload is open.
    pub open: bool,
}

/// First byte of an ESP32 application image.
pub const ESP_IMAGE_MAGIC: u8 = 0xe9;

impl OtaSink for SimOta {
    fn begin(&mut self, total_len: Option<u64>) -> Result<(), String> {
        self.buffer.clear();
        self.expected = total_len;
        self.open = true;
        self.finished = None;
        Ok(())
    }

    fn write(&mut self, chunk: &[u8]) -> Result<(), String> {
        if !self.open {
            self.begin(None)?;
        }
        self.buffer.extend_from_slice(chunk);
        Ok(())
    }

    fn finish(&mut self) -> Result<OtaFinish, String> {
        self.open = false;
        if self.buffer.is_empty() {
            return Err(String::from("no image data"));
        }
        if self.buffer[0] != ESP_IMAGE_MAGIC {
            return Err(format!(
                "not an ESP32 application image: first byte is 0x{:02x}, expected 0x{ESP_IMAGE_MAGIC:02x}",
                self.buffer[0]
            ));
        }
        let done = OtaFinish {
            slot: String::from("ota_1"),
            bytes: self.buffer.len() as u64,
            reboot_required: true,
        };
        self.finished = Some(done.clone());
        Ok(done)
    }

    fn abort(&mut self) {
        self.buffer.clear();
        self.open = false;
        self.finished = None;
    }
}

/// Randomness for session ids, tokens and salts.
#[derive(Debug, Default, Clone, Copy)]
pub struct SimRng;

impl Rng for SimRng {
    fn fill(&mut self, out: &mut [u8]) {
        use rand::Rng as _;
        rand::rng().fill_bytes(out);
    }
}
