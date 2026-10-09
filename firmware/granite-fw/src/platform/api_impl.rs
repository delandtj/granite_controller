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
//! rather than from the live config.

use std::sync::{Arc, Mutex};

use granite_core::api::{
    Identity as ApiIdentity, MqttStatus, NetControl, NetStatus as ApiNetStatus, OtaStatus,
    Platform as ApiPlatform, Rng, SlotInfo, StagedInfo, Store as ApiStore,
};
use granite_core::config::{Section, Secrets};
use granite_core::hal::BootReason;
use granite_core::msg::{Event, EventKind};

use super::store::{DEVICE_CERT_KEY, RECOVERY_TOKEN_KEY};
use super::{Platform, identity, logring, ota};
use crate::http::{CommandChannel, HttpCtx};

/// Key in the `factory` namespace recording that the recovery token has
/// been shown on the setup page once.
const TOKEN_SHOWN_KEY: &str = "rtokshown";

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
        let mut store = self.0.store.lock().map_err(|_| "the store is locked")?;
        store
            .set_str(
                granite_core::config::SECRETS_NAMESPACE,
                DEVICE_CERT_KEY,
                cert_pem,
            )
            .map_err(|e| e.to_string())?;
        let mut secrets = store.load_secrets();
        secrets.device_key_pem = key_pem.to_string();
        store.save_secrets(&secrets).map_err(|e| e.to_string())?;
        log::warn!(
            "device certificate replaced; the HTTPS server picks it up on the next reboot"
        );
        Ok(())
    }
}

// ---------------------------------------------------------------------
// api::NetControl
// ---------------------------------------------------------------------

/// Commit-confirmed configuration (ADR component 6).
///
/// `cfg` in NVS keeps the value that is running until a confirm promotes
/// the staging slot, so an unconfirmed change is undone by a reboot and
/// nothing has to remember a "previous" copy.
pub struct NetControlApi {
    platform: Arc<Platform>,
    /// Sections staged in this session, with the deadline, so
    /// [`NetControl::staged`] can answer without reading NVS.
    staged: Vec<Section>,
    deadline_ms: u64,
}

impl NetControlApi {
    /// A fresh controller. Anything staged before the last reboot was
    /// already discarded by `platform::init`.
    pub fn new(platform: Arc<Platform>) -> Self {
        NetControlApi {
            platform,
            staged: Vec::new(),
            deadline_ms: 0,
        }
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
            // The broker and security sections are staged and promoted on
            // confirm; applying them live needs the MQTT worker's
            // reconnect path and the HTTPS server's certificate reload.
            // TODO(ADR 0001 6): wire those two once mqtt.rs is in.
            None => log::warn!(
                "{section} staged; it takes effect on confirm plus a reboot, not immediately"
            ),
        }

        self.staged.retain(|s| *s != section);
        self.staged.push(section);
        self.deadline_ms = super::now_ms() + u64::from(window) * 1000;
        Ok(window)
    }

    fn confirm(&mut self) -> bool {
        if self.staged.is_empty() {
            return false;
        }
        let sections = std::mem::take(&mut self.staged);
        self.deadline_ms = 0;
        if let Ok(mut store) = self.platform.store.lock() {
            for section in &sections {
                match store.promote_staged(*section) {
                    Ok(true) => log::warn!("{section} confirmed and stored"),
                    Ok(false) => log::warn!("{section} had nothing staged to promote"),
                    Err(e) => log::error!("{section} could not be promoted: {e}"),
                }
            }
        }
        self.platform.net.confirm();
        true
    }

    fn revert(&mut self) -> bool {
        if self.staged.is_empty() {
            return false;
        }
        let sections = std::mem::take(&mut self.staged);
        self.deadline_ms = 0;
        if let Ok(mut store) = self.platform.store.lock() {
            for section in &sections {
                if let Err(e) = store.clear_staged(*section) {
                    log::error!("staging slot for {section} could not be cleared: {e}");
                }
            }
        }
        // net.revert reboots into the stored configuration, which is the
        // only way to be sure the live interface matches it again.
        self.platform.net.revert();
        true
    }

    fn staged(&self) -> Option<StagedInfo> {
        if self.staged.is_empty() {
            return None;
        }
        Some(StagedInfo {
            sections: self.staged.clone(),
            seconds_left: (self.deadline_ms.saturating_sub(super::now_ms()) / 1000) as u32,
        })
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
        cert_pem: platform.identity.cert_pem.clone(),
        key_pem: platform.identity.key_pem.clone(),
        https_port: HTTPS_PORT,
        http_port: HTTP_PORT,
    }
}

/// The command channel every transport shares. Re-exported so `main.rs`
/// does not have to reach into `http`.
pub type Commands = Arc<CommandChannel>;

