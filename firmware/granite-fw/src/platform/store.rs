//! NVS store (ADR 0001 component 10).
//!
//! Four namespaces in the `nvs` partition:
//!
//! | Namespace | Holds | Export | Factory reset |
//! |---|---|---|---|
//! | `cfg` | one JSON blob per [`Section`] | yes | erased |
//! | `stage` | the staged value of a reachability section, until it is confirmed | no | erased |
//! | `secrets` | password hash, API tokens, MQTT credentials | never | erased |
//! | `factory` | recovery token, fleet recovery public key, device key and cert | never | kept |
//!
//! The device key and certificate live in `factory` so the HTTPS
//! fingerprint survives a factory reset and stays pinnable (ADR
//! component 10, "device cert if kept"). Boards written by an older
//! firmware kept them in `secrets`; [`Store::migrate_identity_to_factory`]
//! moves them on the next boot.
//!
//! A section that does not parse loses only itself: [`Store::load_config`]
//! hands every blob to [`Config::from_sections`], which falls back per
//! section and reports what it had to default. The caller logs that as a
//! `config` event.
//!
//! Commit-confirm (ADR component 6) needs no "previous" copy: `cfg` keeps
//! the old value until [`Store::promote_staged`] runs, so a reboot without
//! a confirm restores the old config by doing nothing.

use std::collections::HashMap;

use esp_idf_svc::nvs::{EspDefaultNvsPartition, EspNvs, NvsDefault};
use esp_idf_svc::sys::EspError;
use granite_core::config::{
    CONFIG_NAMESPACE, Config, FACTORY_NAMESPACE, SECRETS_NAMESPACE, Section, SectionFallback,
    Secrets,
};
use granite_core::hal::{HalError, HalResult, Persist};

/// Namespace holding staged (applied but unconfirmed) config sections.
pub const STAGE_NAMESPACE: &str = "stage";

/// Key in [`CONFIG_NAMESPACE`] holding the intended-OFF set.
pub const INTENDED_OFF_KEY: &str = "intended_off";

/// Key in [`SECRETS_NAMESPACE`] holding the serialised [`Secrets`].
pub const SECRETS_KEY: &str = "secrets";
/// Key holding the device certificate, PEM. In [`FACTORY_NAMESPACE`]
/// since 2026-10-10; older boards have it in [`SECRETS_NAMESPACE`] and
/// the Security page's upload still writes it there.
pub const DEVICE_CERT_KEY: &str = "devcert";

/// Key in [`FACTORY_NAMESPACE`] holding the recovery token, base32.
pub const RECOVERY_TOKEN_KEY: &str = "rtok";
/// Key in [`FACTORY_NAMESPACE`] holding the fleet recovery public key, PEM.
pub const FLEET_PUBKEY_KEY: &str = "fleetpub";
/// Key in [`FACTORY_NAMESPACE`] holding the device private key, PEM.
pub const DEVICE_KEY_KEY: &str = "devkey";
/// Key in [`FACTORY_NAMESPACE`] marking the recovery token as shown once.
/// Cleared by a factory reset so the next first setup shows it again.
pub const TOKEN_SHOWN_KEY: &str = "rtokshown";

/// Namespaces a factory reset erases. `factory` is deliberately absent.
const WIPED_ON_RESET: [&str; 3] = [CONFIG_NAMESPACE, STAGE_NAMESPACE, SECRETS_NAMESPACE];

fn map_err(e: EspError) -> HalError {
    HalError::Other(format!("nvs: {e}"))
}

/// The NVS-backed [`Persist`] implementation, plus the config-shaped
/// helpers the rest of the firmware uses.
pub struct Store {
    partition: EspDefaultNvsPartition,
    /// One open handle per namespace, opened on first use. NVS handles are
    /// cheap but not free, and there are four of them at most.
    handles: HashMap<String, EspNvs<NvsDefault>>,
}

impl Store {
    /// Take the default NVS partition and open the store.
    ///
    /// `nvs_flash_init` happens inside `EspDefaultNvsPartition::take`; a
    /// partition that fails to mount (new chip, changed layout) is
    /// reformatted by ESP-IDF, so this call succeeding means the namespaces
    /// below are usable.
    pub fn new() -> Result<Self, EspError> {
        let partition = EspDefaultNvsPartition::take()?;
        Ok(Store {
            partition,
            handles: HashMap::new(),
        })
    }

    /// The NVS partition, for a module that keeps a namespace of its own
    /// (the OTA rollback report).
    pub fn partition(&self) -> EspDefaultNvsPartition {
        self.partition.clone()
    }

    fn ns(&mut self, namespace: &str) -> HalResult<&mut EspNvs<NvsDefault>> {
        if !self.handles.contains_key(namespace) {
            let nvs =
                EspNvs::new(self.partition.clone(), namespace, true).map_err(map_err)?;
            self.handles.insert(namespace.to_string(), nvs);
        }
        // Just inserted or already present.
        Ok(self.handles.get_mut(namespace).expect("namespace handle"))
    }

    /// Read a UTF-8 blob.
    pub fn get_str(&mut self, namespace: &str, key: &str) -> HalResult<Option<String>> {
        match self.get(namespace, key)? {
            None => Ok(None),
            Some(bytes) => match String::from_utf8(bytes) {
                Ok(s) => Ok(Some(s)),
                Err(_) => Err(HalError::Other(format!(
                    "{namespace}/{key} is not valid UTF-8"
                ))),
            },
        }
    }

    /// Write a UTF-8 blob.
    pub fn set_str(&mut self, namespace: &str, key: &str, value: &str) -> HalResult<()> {
        self.set(namespace, key, value.as_bytes())
    }

    // -- configuration ----------------------------------------------------

    /// Load every section, falling back per section.
    ///
    /// The second element lists the sections that had to be defaulted; the
    /// caller logs one line per entry and emits a `config` event, which is
    /// how a corrupt NVS becomes visible instead of silent.
    pub fn load_config(&mut self) -> (Config, Vec<SectionFallback>) {
        let mut blobs: HashMap<Section, String> = HashMap::new();
        let mut unreadable: Vec<(Section, HalError)> = Vec::new();
        for section in Section::ALL {
            match self.get_str(CONFIG_NAMESPACE, section.key()) {
                Ok(Some(json)) => {
                    blobs.insert(section, json);
                }
                Ok(None) => {}
                Err(e) => unreadable.push((section, e)),
            }
        }
        for (section, e) in &unreadable {
            log::warn!("config: {section} could not be read ({e}), using defaults");
        }
        Config::from_sections(|section| blobs.get(&section).cloned())
    }

    /// The nodes the controller last commanded rail-off, as stored.
    ///
    /// ADR 0004 keeps this set in NVS so an OFF survives a reboot of the
    /// controller as well as of a plane: a plane that lost its own
    /// intended state is put back from it. Stored as a comma-separated
    /// list of node ids, which is a dozen bytes and readable from the
    /// console.
    pub fn load_intended_off(&mut self) -> Vec<granite_core::NodeId> {
        let Ok(Some(text)) = self.get_str(CONFIG_NAMESPACE, INTENDED_OFF_KEY) else {
            return Vec::new();
        };
        text.split(',')
            .filter_map(|part| part.trim().parse::<granite_core::NodeId>().ok())
            .filter(|node| granite_core::is_node(*node))
            .collect()
    }

    /// Store the intended-OFF set.
    pub fn save_intended_off(&mut self, nodes: &[granite_core::NodeId]) -> HalResult<()> {
        let text = nodes
            .iter()
            .map(|node| node.to_string())
            .collect::<Vec<String>>()
            .join(",");
        self.set_str(CONFIG_NAMESPACE, INTENDED_OFF_KEY, &text)
    }

    /// Persist one section as it now stands in `cfg`.
    pub fn save_section(&mut self, cfg: &Config, section: Section) -> HalResult<()> {
        let json = cfg
            .section_json(section)
            .map_err(|e| HalError::Other(format!("config {section}: {e}")))?;
        self.set_str(CONFIG_NAMESPACE, section.key(), &json)
    }

    /// Persist every section. Used once, when a board boots with an empty
    /// `cfg` namespace, so the defaults become a real document.
    pub fn save_all(&mut self, cfg: &Config) -> HalResult<()> {
        for section in Section::ALL {
            self.save_section(cfg, section)?;
        }
        Ok(())
    }

    /// True when no section has ever been written (first boot).
    pub fn is_blank(&mut self) -> bool {
        Section::ALL.iter().all(|s| {
            matches!(
                self.get(CONFIG_NAMESPACE, s.key()),
                Ok(None) | Err(HalError::Storage)
            )
        })
    }

    // -- staging (commit-confirm) ----------------------------------------

    /// Write the new value of a reachability section to the staging slot.
    /// `cfg` keeps the old value, so a reboot reverts.
    pub fn stage_section(&mut self, cfg: &Config, section: Section) -> HalResult<()> {
        let json = cfg
            .section_json(section)
            .map_err(|e| HalError::Other(format!("config {section}: {e}")))?;
        self.set_str(STAGE_NAMESPACE, section.key(), &json)
    }

    /// The staged value of a section, if one is waiting for a confirm.
    pub fn staged_section(&mut self, section: Section) -> HalResult<Option<String>> {
        self.get_str(STAGE_NAMESPACE, section.key())
    }

    /// Any section with a staged value.
    pub fn staged_sections(&mut self) -> Vec<Section> {
        Section::ALL
            .into_iter()
            .filter(|s| matches!(self.staged_section(*s), Ok(Some(_))))
            .collect()
    }

    /// Promote a staged section into `cfg` and clear the staging slot.
    pub fn promote_staged(&mut self, section: Section) -> HalResult<bool> {
        let Some(json) = self.staged_section(section)? else {
            return Ok(false);
        };
        self.set_str(CONFIG_NAMESPACE, section.key(), &json)?;
        self.remove(STAGE_NAMESPACE, section.key())?;
        Ok(true)
    }

    /// Drop a staged section without applying it.
    pub fn clear_staged(&mut self, section: Section) -> HalResult<()> {
        self.remove(STAGE_NAMESPACE, section.key())
    }

    // -- secrets ----------------------------------------------------------

    /// Load the secrets blob, defaulting (and reporting) on corruption.
    pub fn load_secrets(&mut self) -> Secrets {
        match self.get_str(SECRETS_NAMESPACE, SECRETS_KEY) {
            Ok(Some(json)) => match Secrets::from_json(&json) {
                Ok(s) => s,
                Err(e) => {
                    log::error!("secrets blob does not parse ({e}); starting from defaults");
                    Secrets::default()
                }
            },
            Ok(None) => Secrets::default(),
            Err(e) => {
                log::error!("secrets blob could not be read ({e}); starting from defaults");
                Secrets::default()
            }
        }
    }

    /// Store the secrets blob.
    pub fn save_secrets(&mut self, secrets: &Secrets) -> HalResult<()> {
        let json = secrets
            .to_json()
            .map_err(|e| HalError::Other(format!("secrets: {e}")))?;
        self.set_str(SECRETS_NAMESPACE, SECRETS_KEY, &json)
    }

    /// The device certificate, PEM, from the namespace a factory reset
    /// keeps.
    pub fn device_cert(&mut self) -> HalResult<Option<String>> {
        self.get_str(FACTORY_NAMESPACE, DEVICE_CERT_KEY)
    }

    /// Store the device certificate, PEM.
    pub fn save_device_cert(&mut self, pem: &str) -> HalResult<()> {
        self.set_str(FACTORY_NAMESPACE, DEVICE_CERT_KEY, pem)
    }

    /// The device private key, PEM, from the namespace a factory reset
    /// keeps.
    pub fn device_key(&mut self) -> HalResult<Option<String>> {
        self.get_str(FACTORY_NAMESPACE, DEVICE_KEY_KEY)
    }

    /// Store the device private key, PEM.
    pub fn save_device_key(&mut self, pem: &str) -> HalResult<()> {
        self.set_str(FACTORY_NAMESPACE, DEVICE_KEY_KEY, pem)
    }

    /// Move a device key and certificate left in `secrets` into `factory`.
    ///
    /// Only a board written by a firmware older than 2026-10-10 has them
    /// there (the Security page's certificate upload writes `factory`
    /// directly). The `secrets` copy is removed afterwards, so this is a
    /// no-op on every later boot. A `secrets` copy wins over a `factory`
    /// one: it is the identity the board was actually using.
    pub fn migrate_identity_to_factory(&mut self) -> HalResult<bool> {
        let mut moved = false;
        let legacy_cert = self
            .get_str(SECRETS_NAMESPACE, DEVICE_CERT_KEY)?
            .filter(|p| !p.trim().is_empty());
        if let Some(pem) = legacy_cert {
            self.save_device_cert(&pem)?;
            self.remove(SECRETS_NAMESPACE, DEVICE_CERT_KEY)?;
            moved = true;
        }
        let mut secrets = self.load_secrets();
        if !secrets.device_key_pem.trim().is_empty() {
            let pem = core::mem::take(&mut secrets.device_key_pem);
            self.save_device_key(&pem)?;
            self.save_secrets(&secrets)?;
            moved = true;
        }
        if moved {
            log::warn!("device key and certificate moved from secrets to the factory namespace");
        }
        Ok(moved)
    }

    // -- factory namespace ------------------------------------------------

    /// Read a value from the namespace a factory reset keeps.
    pub fn factory_get(&mut self, key: &str) -> HalResult<Option<String>> {
        self.get_str(FACTORY_NAMESPACE, key)
    }

    /// Write a value into the namespace a factory reset keeps.
    pub fn factory_set(&mut self, key: &str, value: &str) -> HalResult<()> {
        self.set_str(FACTORY_NAMESPACE, key, value)
    }

    /// Erase everything except [`FACTORY_NAMESPACE`].
    ///
    /// Does not reboot: [`Store::factory_reset`] does both, and the caller
    /// that wants to log first can use this one.
    pub fn wipe(&mut self) -> HalResult<()> {
        for namespace in WIPED_ON_RESET {
            self.erase_namespace(namespace)?;
        }
        // The recovery token itself is kept, but the "already shown"
        // flag is not: whoever does the first setup after a reset may
        // not be the person who wrote the token down.
        self.remove(FACTORY_NAMESPACE, TOKEN_SHOWN_KEY)?;
        Ok(())
    }

    /// Erase everything except [`FACTORY_NAMESPACE`] and reboot.
    ///
    /// Goes out through the planned-reboot path, so every relay is released
    /// before the restart.
    pub fn factory_reset(&mut self) -> ! {
        match self.wipe() {
            Ok(()) => log::warn!(
                "factory reset: cfg, stage and secrets erased; recovery token, fleet key and \
                 device certificate kept"
            ),
            Err(e) => log::error!("factory reset: erase failed ({e}), rebooting anyway"),
        }
        super::planned_reboot("factory_reset")
    }
}

impl Persist for Store {
    fn get(&mut self, namespace: &str, key: &str) -> HalResult<Option<Vec<u8>>> {
        let nvs = self.ns(namespace)?;
        let Some(len) = nvs.blob_len(key).map_err(map_err)? else {
            return Ok(None);
        };
        let mut buf = vec![0u8; len];
        match nvs.get_blob(key, &mut buf).map_err(map_err)? {
            Some(slice) => {
                let n = slice.len();
                buf.truncate(n);
                Ok(Some(buf))
            }
            None => Ok(None),
        }
    }

    fn set(&mut self, namespace: &str, key: &str, value: &[u8]) -> HalResult<()> {
        self.ns(namespace)?.set_blob(key, value).map_err(map_err)
    }

    fn remove(&mut self, namespace: &str, key: &str) -> HalResult<()> {
        self.ns(namespace)?.remove(key).map_err(map_err).map(|_| ())
    }

    fn erase_namespace(&mut self, namespace: &str) -> HalResult<()> {
        self.ns(namespace)?.erase_all().map_err(map_err)
    }
}
