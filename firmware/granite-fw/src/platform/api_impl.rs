//! The platform side of the HTTP API's contract
//! (`granite_core::api`): the traits [`crate::http::HttpCtx`] wants, and
//! [`http_ctx`], which builds one.
//!
//! Every handler, every authentication rule and every JSON shape lives in
//! `granite-core/src/api.rs`, which is host-tested. This file is the
//! adapter from those traits to the platform layer, and nothing else.
//!
//! ### Locking
//!
//! `http.rs` documents the order it takes its guards in
//! (`auth -> store -> platform -> identity -> net -> ota -> rng ->
//! config -> observed`) and holds the **config write guard for a whole
//! request**. Nothing in this file may therefore touch
//! [`Platform::config`]: a handler that asked for, say, the broker state
//! would deadlock against the guard its own request is holding. That is
//! why [`Platform::mqtt`] exists as a separate mutex and why
//! [`NetControl::stage_and_apply`] works from the JSON it is handed
//! rather than from the live config. It is also why a revert does not put
//! the stored section back itself: [`revert_sections`] queues it with
//! [`queue_restore`] and the config-watch thread, which holds no guard,
//! carries it out.

use std::sync::{Arc, Mutex};

use granite_core::api::{
    Identity as ApiIdentity, MqttStatus, NetControl, NetStatus as ApiNetStatus, OtaStatus,
    Platform as ApiPlatform, Rng, SlotInfo, StagedInfo, Store as ApiStore,
};
use granite_core::config::{Section, Secrets};
use granite_core::hal::BootReason;
use granite_core::msg::{Event, EventKind};

use super::store::{RECOVERY_TOKEN_KEY, TOKEN_SHOWN_KEY};
use super::{Platform, identity, logring, ota};
use crate::http::{CommandChannel, HttpCtx};

/// HTTPS port (ADR component 8).
pub const HTTPS_PORT: u16 = 443;
/// Plain HTTP port; serves `/id` and a redirect only.
pub const HTTP_PORT: u16 = 80;

// ---------------------------------------------------------------------
// api::Platform
// ---------------------------------------------------------------------

/// System state for the status and maintenance pages.
pub struct PlatformApi(pub Arc<Platform>);

impl ApiPlatform for PlatformApi {
    fn fw_version(&self) -> String {
        super::FW_VERSION.to_string()
    }

    fn uptime_s(&self) -> u32 {
        (super::now_ms() / 1000) as u32
    }

    fn boot_reason(&self) -> BootReason {
        super::boot_reason()
    }

    fn free_heap(&self) -> u32 {
        unsafe { esp_idf_svc::sys::esp_get_free_heap_size() }
    }

    fn net(&self) -> ApiNetStatus {
        let s = self.0.net.status();
        ApiNetStatus {
            link_up: s.link,
            ip_mode: s.mode.as_str().to_string(),
            ip: s.ip,
            netmask: s.netmask,
            gateway: s.gateway,
            // The DNS servers live in the config, which a request already
            // holds; the page reads them from the config section instead.
            dns: Vec::new(),
            hostname: s.hostname,
            dhcp_fallback: s.deadman_fired,
            sntp_synced: s.time_synced,
        }
    }

    fn mqtt(&self) -> MqttStatus {
        self.0.mqtt_status()
    }

    fn ota(&self) -> OtaStatus {
        let state = ota::ota_state();
        match ota::info() {
            Ok(info) => OtaStatus {
                running: info.running,
                state: state.as_str().to_string(),
                pending_verify: state == ota::OtaState::Pending,
                validate_left_s: None,
                slots: vec![SlotInfo {
                    label: info.next,
                    state: String::from("next"),
                    version: String::new(),
                    size: 0,
                }],
                key_id: super::app_elf_sha256_short(),
                rollback_available: info.last_invalid.is_none(),
            },
            Err(e) => OtaStatus {
                running: String::new(),
                state: format!("unreadable: {e}"),
                pending_verify: false,
                validate_left_s: None,
                slots: Vec::new(),
                key_id: super::app_elf_sha256_short(),
                rollback_available: false,
            },
        }
    }

    fn log_tail(&self, lines: usize) -> Vec<String> {
        logring::tail(lines)
    }

    fn ota_rollback(&mut self) -> Result<(), String> {
        // Marks the running image invalid and reboots into the previous
        // slot. Only returns when ESP-IDF refused, which it does when
        // there is no other app to go back to.
        let e = ota::mark_invalid_and_reboot();
        Err(format!("rollback refused: {e}"))
    }

    fn ota_mark_valid(&mut self) -> Result<(), String> {
        ota::mark_valid().map_err(|e| e.to_string())
    }

    fn event(&mut self, kind: EventKind) {
        self.0.publish(Event::new(super::now_ms(), kind));
    }
}

// ---------------------------------------------------------------------
// api::Identity
// ---------------------------------------------------------------------

/// Identity and recovery (ADR component 13).
pub struct IdentityApi(pub Arc<Platform>);

impl ApiIdentity for IdentityApi {
    fn device_id(&self) -> String {
        self.0.identity.device_id.clone()
    }

    fn mac(&self) -> String {
        identity::mac_string(&self.0.identity.mac)
    }

    fn cert_sha256(&self) -> String {
        self.0.identity.cert_sha256.clone()
    }

    fn recovery_token_once(&mut self) -> Option<String> {
        let mut store = self.0.store.lock().ok()?;
        if matches!(store.factory_get(TOKEN_SHOWN_KEY), Ok(Some(_))) {
            return None;
        }
        // The flag goes into `factory`, which a factory reset keeps, so a
        // reboot in the middle of first setup cannot turn the token into
        // something the page shows a second time.
        if let Err(e) = store.factory_set(TOKEN_SHOWN_KEY, "1") {
            log::error!("recovery token shown flag could not be stored: {e}");
            return None;
        }
        log::warn!("recovery token handed to the setup page; it will not be shown again");
        Some(self.0.identity.recovery_token.clone())
    }

    fn verify_recovery_token(&mut self, token: &str) -> bool {
        // Read the token back from `factory` rather than trusting the copy
        // in RAM: this is the path that erases the board.
        let stored = match self.0.store.lock() {
            Ok(mut store) => store
                .factory_get(RECOVERY_TOKEN_KEY)
                .unwrap_or(None)
                .unwrap_or_default(),
            Err(_) => return false,
        };
        identity::recovery_token_matches(&stored, token)
    }

    fn verify_fleet_sig(&self, message: &[u8], sig: &str) -> bool {
        identity::verify_fleet_sig(self.0.identity.fleet_pubkey_pem.as_deref(), message, sig)
    }

    fn set_fleet_pubkey(&mut self, pem: &str) -> Result<(), String> {
        let mut store = self.0.store.lock().map_err(|_| "the store is locked")?;
        identity::set_fleet_pubkey(&mut store, pem)
    }

    fn set_device_cert(&mut self, cert_pem: &str, key_pem: &str) -> Result<(), String> {
        if !cert_pem.contains("BEGIN CERTIFICATE") {
            return Err(String::from("that is not a PEM certificate"));
        }
        if !key_pem.contains("PRIVATE KEY") {
            return Err(String::from("that is not a PEM private key"));
        }
        // Straight into `factory`, the namespace a factory reset keeps
        // (ADR component 10, "device cert if kept"), so an uploaded pair
        // survives a reset like a generated one and nothing has to be
        // migrated out of `secrets` on the next boot.
        let mut store = self.0.store.lock().map_err(|_| "the store is locked")?;
        store
            .save_device_cert(cert_pem)
            .map_err(|e| e.to_string())?;
        store.save_device_key(key_pem).map_err(|e| e.to_string())?;
        log::warn!(
            "device certificate replaced; the HTTPS server cannot swap its TLS configuration \
             while it runs, so it serves the new one after the next reboot"
        );
        Ok(())
    }
}

// ---------------------------------------------------------------------
// Staged sections: one registry, one deadline (ADR component 6)
// ---------------------------------------------------------------------

/// What reverting a staged section costs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Revert {
    /// `net`: the live interface was already changed, so the only way back
    /// is a reboot into the stored configuration.
    Reboot,
    /// `mqtt` and `sec`: their owners (the MQTT worker, the Modbus
    /// supervisor) re-read the live configuration, so putting the stored
    /// section back into it is enough. No reboot.
    RestoreLive,
}

impl Revert {
    /// How `section` goes back to its stored value.
    pub const fn of(section: Section) -> Self {
        match section {
            Section::Net => Revert::Reboot,
            _ => Revert::RestoreLive,
        }
    }
}

/// The staged sections and the single deadline they share.
///
/// Pure bookkeeping on an injected clock, so the policy is testable
/// without a board: `now_ms` is always a parameter and nothing in here
/// touches NVS, the network or the config.
#[derive(Debug, Default)]
struct Staged {
    sections: Vec<Section>,
    deadline_ms: u64,
    /// Sections whose *live* value still has to be restored from the
    /// store. Carried out by the config-watch thread, never here: this
    /// file may not take the config guard (see the locking note above).
    restore: Vec<Section>,
}

impl Staged {
    const fn new() -> Self {
        Staged {
            sections: Vec::new(),
            deadline_ms: 0,
            restore: Vec::new(),
        }
    }

    /// Record a staged section. One timer for all of them: the newest
    /// window wins, so a second change is not handed a deadline that is
    /// about to pass.
    fn stage(&mut self, section: Section, now_ms: u64, window_s: u32) {
        self.sections.retain(|s| *s != section);
        self.sections.push(section);
        self.deadline_ms = now_ms + u64::from(window_s) * 1000;
    }

    /// Everything staged, and nothing is staged afterwards.
    fn take(&mut self) -> Vec<Section> {
        self.deadline_ms = 0;
        core::mem::take(&mut self.sections)
    }

    /// True once the window has passed with something still staged.
    fn due(&self, now_ms: u64) -> bool {
        !self.sections.is_empty() && self.deadline_ms > 0 && now_ms >= self.deadline_ms
    }

    fn seconds_left(&self, now_ms: u64) -> u32 {
        (self.deadline_ms.saturating_sub(now_ms) / 1000) as u32
    }
}

/// Staged sections for the whole firmware.
///
/// It is global because three paths stage and two more read: the HTTP API
/// ([`NetControlApi`]), a `config_set` command through the dispatcher, the
/// config-watch thread that enforces the deadline, and the MQTT worker on
/// `auto_confirm_on_connect`. Each HTTP request builds its own
/// [`NetControlApi`], so a per-instance list could not survive one.
static STAGED: Mutex<Staged> = Mutex::new(Staged::new());

/// Run `f` against the registry. `None` when the mutex is poisoned, which
/// only a panic while holding it can do.
fn with_staged<T>(f: impl FnOnce(&mut Staged) -> T) -> Option<T> {
    STAGED.lock().ok().map(|mut s| f(&mut s))
}

/// Record that `section` is staged and start (or restart) the one window.
pub fn note_staged(section: Section, window_s: u32) {
    let now = super::now_ms();
    with_staged(|s| s.stage(section, now, window_s));
}

/// True while `section` is staged and unconfirmed.
pub fn is_staged(section: Section) -> bool {
    with_staged(|s| s.sections.contains(&section)).unwrap_or(false)
}

/// What is staged, for the status page.
pub fn staged_info() -> Option<StagedInfo> {
    let now = super::now_ms();
    with_staged(|s| {
        if s.sections.is_empty() {
            return None;
        }
        Some(StagedInfo {
            sections: s.sections.clone(),
            seconds_left: s.seconds_left(now),
        })
    })
    .flatten()
}

/// Take every staged section (a confirm or an explicit revert).
pub fn take_staged() -> Vec<Section> {
    with_staged(Staged::take).unwrap_or_default()
}

/// Take every staged section, but only once the window has passed.
pub fn take_due_staged(now_ms: u64) -> Vec<Section> {
    with_staged(|s| {
        if s.due(now_ms) {
            s.take()
        } else {
            Vec::new()
        }
    })
    .unwrap_or_default()
}

/// Ask the config-watch thread to put the stored value of these sections
/// back into the live configuration.
pub fn queue_restore(sections: &[Section]) {
    with_staged(|s| {
        for section in sections {
            if !s.restore.contains(section) {
                s.restore.push(*section);
            }
        }
    });
}

/// The restores the config-watch thread still owes.
pub fn take_restore() -> Vec<Section> {
    with_staged(|s| core::mem::take(&mut s.restore)).unwrap_or_default()
}

/// Promote staged sections into the stored configuration.
///
/// This *is* the confirm: the HTTP API's `confirm` calls it for everything
/// staged, and the MQTT worker calls it through [`auto_confirm_mqtt`] for
/// the broker section alone.
pub fn promote(platform: &Platform, sections: &[Section]) -> bool {
    let mut promoted = false;
    if let Ok(mut store) = platform.store.lock() {
        for section in sections {
            match store.promote_staged(*section) {
                Ok(true) => {
                    promoted = true;
                    log::warn!("{section} confirmed and stored");
                }
                Ok(false) => log::warn!("{section} had nothing staged to promote"),
                Err(e) => log::error!("{section} could not be promoted: {e}"),
            }
        }
    }
    if sections.contains(&Section::Net) {
        platform.net.confirm();
    }
    with_staged(|s| s.sections.retain(|s| !sections.contains(s)));
    promoted
}

/// Put the staged sections back in their stored state.
///
/// `net` reboots (the live interface was changed under the session);
/// `mqtt` and `sec` only need their stored section back in the live
/// configuration, which the config-watch thread does on its next pass.
pub fn revert_sections(platform: &Platform, sections: &[Section]) {
    if let Ok(mut store) = platform.store.lock() {
        for section in sections {
            if let Err(e) = store.clear_staged(*section) {
                log::error!("staging slot for {section} could not be cleared: {e}");
            }
        }
    }
    let live: Vec<Section> = sections
        .iter()
        .copied()
        .filter(|s| Revert::of(*s) == Revert::RestoreLive)
        .collect();
    if !live.is_empty() {
        queue_restore(&live);
    }
    if sections.iter().any(|s| Revert::of(*s) == Revert::Reboot) {
        // net.revert reboots into the stored configuration, which is the
        // only way to be sure the live interface matches it again.
        platform.net.revert();
    }
}

/// `mqtt.auto_confirm_on_connect`: a broker session on the staged section
/// is the confirmation (ADR component 6). Only `mqtt` is promoted - the
/// ADR ties auto-confirm to the broker change, so a `net` or `sec` change
/// staged at the same time still waits for its own confirm.
pub fn auto_confirm_mqtt(platform: &Platform) -> bool {
    if !is_staged(Section::Mqtt) {
        return false;
    }
    if !promote(platform, &[Section::Mqtt]) {
        return false;
    }
    log::warn!("mqtt: the broker connected and auto_confirm_on_connect is set; section confirmed");
    platform.publish(Event::new(
        super::now_ms(),
        EventKind::Config {
            section: Some(Section::Mqtt),
            change: String::from("confirmed"),
            detail: Some(String::from("auto_confirm_on_connect")),
        },
    ));
    true
}

// ---------------------------------------------------------------------
// api::NetControl
// ---------------------------------------------------------------------

/// Commit-confirmed configuration (ADR component 6).
///
/// `cfg` in NVS keeps the value that is running until a confirm promotes
/// the staging slot, so an unconfirmed change is undone by a reboot and
/// nothing has to remember a "previous" copy. The staged list itself lives
/// in [`STAGED`], not here: a request builds a fresh `NetControlApi`, and
/// the deadline has to outlive it.
pub struct NetControlApi {
    platform: Arc<Platform>,
}

impl NetControlApi {
    /// A fresh controller. Anything staged before the last reboot was
    /// already discarded by `platform::init`.
    pub fn new(platform: Arc<Platform>) -> Self {
        NetControlApi { platform }
    }
}

impl NetControl for NetControlApi {
    fn stage_and_apply(&mut self, section: Section, json: &str) -> Result<u32, String> {
        // The window comes from the staged net section when that is what
        // is changing, so a client that shortens t_confirm_s gets the
        // short window it asked for on the very change that sets it.
        let mut window = 300u32;
        let mut new_net = None;
        if section == Section::Net {
            let cfg: granite_core::config::NetCfg =
                serde_json::from_str(json).map_err(|e| format!("net section: {e}"))?;
            if cfg.t_confirm_s > 0 {
                window = cfg.t_confirm_s;
            }
            new_net = Some(cfg);
        }

        {
            let mut store = self.platform.store.lock().map_err(|_| "the store is locked")?;
            store
                .set_str(
                    super::store::STAGE_NAMESPACE,
                    section.key(),
                    json,
                )
                .map_err(|e| e.to_string())?;
        }

        match new_net {
            Some(cfg) => self.platform.net.apply(&cfg, window),
            // `mqtt` and `sec` are applied live by the tasks that own
            // them: the MQTT worker rebuilds its client when the live
            // section differs and the config watcher pushes `sec.modbus`
            // into the Modbus supervisor. `put_section` has already
            // written the staged value into the live config, so both pick
            // it up within their refresh period, and the deadline in the
            // config-watch thread puts the stored value back if no confirm
            // arrives. The exception is a new device certificate: ESP-IDF's
            // HTTPS server cannot swap its TLS config in place, so that
            // one needs a reboot.
            None => log::warn!(
                "{section} staged and applied live; confirm within {window} s or the stored value \
                 comes back (a new device certificate needs a reboot either way)"
            ),
        }

        note_staged(section, window);
        Ok(window)
    }

    fn confirm(&mut self) -> bool {
        let sections = take_staged();
        if sections.is_empty() {
            return false;
        }
        promote(&self.platform, &sections);
        true
    }

    fn revert(&mut self) -> bool {
        let sections = take_staged();
        if sections.is_empty() {
            return false;
        }
        revert_sections(&self.platform, &sections);
        true
    }

    fn staged(&self) -> Option<StagedInfo> {
        staged_info()
    }
}

// ---------------------------------------------------------------------
// api::Store and api::Rng
// ---------------------------------------------------------------------

/// NVS through the API's narrow view of it (ADR component 10).
pub struct StoreApi(pub Arc<Platform>);

impl ApiStore for StoreApi {
    fn load_secrets(&mut self) -> Secrets {
        match self.0.store.lock() {
            Ok(mut store) => store.load_secrets(),
            Err(_) => Secrets::default(),
        }
    }

    fn save_secrets(&mut self, secrets: &Secrets) -> Result<(), String> {
        let mut store = self.0.store.lock().map_err(|_| "the store is locked")?;
        store.save_secrets(secrets).map_err(|e| e.to_string())
    }

    fn save_section(&mut self, section: Section, json: &str) -> Result<(), String> {
        let mut store = self.0.store.lock().map_err(|_| "the store is locked")?;
        store
            .set_str(granite_core::config::CONFIG_NAMESPACE, section.key(), json)
            .map_err(|e| e.to_string())
    }
}

/// The hardware RNG.
#[derive(Debug, Default)]
pub struct EspRng;

impl Rng for EspRng {
    fn fill(&mut self, out: &mut [u8]) {
        if out.is_empty() {
            return;
        }
        unsafe { esp_idf_svc::sys::esp_fill_random(out.as_mut_ptr().cast(), out.len()) };
    }
}

// ---------------------------------------------------------------------
// The context
// ---------------------------------------------------------------------

/// Build the context the HTTPS server runs on.
///
/// The certificate and its key come out of the `secrets` namespace (or
/// were generated on this boot); the ports are the ADR's 443 and 80.
pub fn http_ctx(platform: &Arc<Platform>) -> HttpCtx {
    HttpCtx {
        config: Arc::clone(&platform.config),
        observed: Arc::clone(&platform.observed),
        auth: HttpCtx::new_auth(),
        store: Arc::new(Mutex::new(StoreApi(Arc::clone(platform)))),
        platform: Arc::new(Mutex::new(PlatformApi(Arc::clone(platform)))),
        identity: Arc::new(Mutex::new(IdentityApi(Arc::clone(platform)))),
        net: Arc::new(Mutex::new(NetControlApi::new(Arc::clone(platform)))),
        ota: Arc::new(Mutex::new(ota::OtaWriter::new())),
        rng: Arc::new(Mutex::new(EspRng)),
        commands: Arc::clone(&platform.commands),
        events: Arc::clone(&platform.events),
        cert_pem: platform.identity.cert_pem.clone(),
        key_pem: platform.identity.key_pem.clone(),
        https_port: HTTPS_PORT,
        http_port: HTTP_PORT,
    }
}

/// The command channel every transport shares. Re-exported so `main.rs`
/// does not have to reach into `http`.
pub type Commands = Arc<CommandChannel>;

// ---------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------

// The staged bookkeeping is the one piece of this file with no ESP-IDF in
// it, so it is the one piece that can be asserted. granite-fw only builds
// for the board, so these run with
// `cargo test --target riscv32imac-esp-espidf` on a devboard; a host
// `cargo test` never sees them. They are type-checked by
// `cargo clippy --all-targets`.
#[cfg(test)]
mod tests {
    use super::{Revert, Staged};
    use granite_core::config::Section;

    #[test]
    fn one_deadline_covers_every_staged_section() {
        let mut s = Staged::new();
        s.stage(Section::Net, 1_000, 300);
        assert_eq!(s.sections, vec![Section::Net]);
        assert_eq!(s.seconds_left(1_000), 300);
        assert!(!s.due(1_000));
        assert!(!s.due(300_999));
        assert!(s.due(301_000));

        // A second staged section restarts the one window rather than
        // inheriting a deadline that is about to pass.
        s.stage(Section::Mqtt, 300_000, 60);
        assert_eq!(s.sections, vec![Section::Net, Section::Mqtt]);
        assert!(!s.due(301_000));
        assert!(s.due(360_000));

        // Staging the same section again moves it to the end once, not
        // twice.
        s.stage(Section::Mqtt, 360_000, 60);
        assert_eq!(s.sections, vec![Section::Net, Section::Mqtt]);
    }

    #[test]
    fn nothing_staged_is_never_due() {
        let mut s = Staged::new();
        assert!(!s.due(u64::MAX));
        s.stage(Section::Mqtt, 0, 5);
        assert!(s.due(5_000));
        assert_eq!(s.take(), vec![Section::Mqtt]);
        assert!(!s.due(u64::MAX), "a taken section cannot fire twice");
        assert_eq!(s.seconds_left(0), 0);
        assert!(s.take().is_empty());
    }

    #[test]
    fn a_passed_deadline_does_not_underflow_the_countdown() {
        let mut s = Staged::new();
        s.stage(Section::Sec, 10_000, 30);
        assert_eq!(s.seconds_left(10_000), 30);
        assert_eq!(s.seconds_left(39_500), 0, "rounded down, not negative");
        assert_eq!(s.seconds_left(u64::MAX), 0);
    }

    #[test]
    fn only_net_reverts_by_rebooting() {
        assert_eq!(Revert::of(Section::Net), Revert::Reboot);
        assert_eq!(Revert::of(Section::Mqtt), Revert::RestoreLive);
        assert_eq!(Revert::of(Section::Sec), Revert::RestoreLive);
        // The other three are never staged (Section::affects_reachability),
        // but the policy must still be the harmless one.
        for section in [Section::Nodes, Section::Rules, Section::Sys] {
            assert_eq!(Revert::of(section), Revert::RestoreLive);
        }
    }

    #[test]
    fn the_restore_queue_does_not_repeat_a_section() {
        let mut s = Staged::new();
        s.restore.push(Section::Mqtt);
        let mut queue = |section: Section| {
            if !s.restore.contains(&section) {
                s.restore.push(section);
            }
        };
        queue(Section::Mqtt);
        queue(Section::Sec);
        assert_eq!(s.restore, vec![Section::Mqtt, Section::Sec]);
    }
}

