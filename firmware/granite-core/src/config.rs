//! Configuration: one `Config`, stored and parsed per section so a
//! corrupt section loses only itself, with secrets kept in a separate
//! struct that never appears in an export (ADR component 10).

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::array;
use core::fmt;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::NODE_COUNT;
use crate::NodeId;
use crate::actuator::{Timings, default_order, sanitise_order};
use crate::node::{BootPolicy, NodeSettings, SenseMode};
use crate::rules::{MAX_RULES, Rule, default_rules};

/// Bumped whenever a stored section needs migrating.
pub const SCHEMA_VERSION: u32 = 1;

/// NVS namespace holding the config sections.
pub const CONFIG_NAMESPACE: &str = "cfg";
/// NVS namespace holding secrets. Excluded from export.
pub const SECRETS_NAMESPACE: &str = "secrets";
/// NVS namespace that survives a factory reset.
pub const FACTORY_NAMESPACE: &str = "factory";

/// The six stored sections.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Section {
    /// IP, hostname, SNTP, commit-confirm timers.
    Net,
    /// Broker, TLS, topics, publish cadence.
    Mqtt,
    /// Node names, order, boot policy, sense, timings, probes.
    Nodes,
    /// The rule set.
    Rules,
    /// Password policy, Modbus allow-list, public key material.
    Sec,
    /// Device id, log level, OTA validation window.
    Sys,
}

impl Section {
    /// Every section, in storage order.
    pub const ALL: [Section; 6] = [
        Section::Net,
        Section::Mqtt,
        Section::Nodes,
        Section::Rules,
        Section::Sec,
        Section::Sys,
    ];

    /// NVS key / JSON field name.
    pub const fn key(self) -> &'static str {
        match self {
            Section::Net => "net",
            Section::Mqtt => "mqtt",
            Section::Nodes => "nodes",
            Section::Rules => "rules",
            Section::Sec => "sec",
            Section::Sys => "sys",
        }
    }

    /// Parse a section name.
    pub fn from_key(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|sec| sec.key() == s)
    }

    /// True for sections where a mistake can cut the session, and which
    /// therefore go through the commit-confirm path.
    pub const fn affects_reachability(self) -> bool {
        matches!(self, Section::Net | Section::Mqtt | Section::Sec)
    }
}

impl fmt::Display for Section {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.key())
    }
}

/// Anything that can go wrong with a config document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    /// The JSON did not parse, or did not fit the schema.
    Parse(String),
    /// Serialising failed (out of memory).
    Serialize(String),
    /// Unknown schema version that no migration handles.
    Schema {
        /// What the document claims.
        found: u32,
        /// What this firmware knows.
        expected: u32,
    },
    /// The document parsed but is not usable.
    Invalid(Vec<String>),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::Parse(m) => write!(f, "parse error: {m}"),
            ConfigError::Serialize(m) => write!(f, "serialize error: {m}"),
            ConfigError::Schema { found, expected } => {
                write!(f, "schema version {found} is newer than {expected}")
            }
            ConfigError::Invalid(v) => write!(f, "invalid config: {}", v.join("; ")),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for ConfigError {}

/// How a node gets its IPv4 address.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IpMode {
    /// DHCP, with AutoIP as a fallback after 30 s.
    #[default]
    Dhcp,
    /// Static address, with the dead-man fallback in [`NetCfg::t_deadman_s`].
    Static,
}

/// The `net` section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct NetCfg {
    /// DHCP or static.
    pub ip_mode: IpMode,
    /// Address in CIDR form, e.g. `192.168.1.10/24`. Empty for DHCP.
    pub address: String,
    /// Default gateway. Empty for DHCP.
    pub gateway: String,
    /// Name servers. Empty for DHCP.
    pub dns: Vec<String>,
    /// Hostname and mDNS name. Empty means `granite-<mac6>`.
    pub hostname: String,
    /// Advertise over mDNS.
    pub mdns: bool,
    /// 802.1Q VLAN id, `None` for untagged.
    pub vlan: Option<u16>,
    /// SNTP server. Empty means "whatever DHCP option 42 says".
    pub sntp: String,
    /// Commit-confirm window for reachability-affecting changes.
    pub t_confirm_s: u32,
    /// Static-config dead-man timer; 0 disables it.
    pub t_deadman_s: u32,
}

impl Default for NetCfg {
    fn default() -> Self {
        NetCfg {
            ip_mode: IpMode::Dhcp,
            address: String::new(),
            gateway: String::new(),
            dns: Vec::new(),
            hostname: String::new(),
            mdns: true,
            vlan: None,
            sntp: String::new(),
            t_confirm_s: 300,
            t_deadman_s: 3_600,
        }
    }
}

/// The `mqtt` section. No password here: see [`Secrets`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MqttCfg {
    /// Off by default.
    pub enabled: bool,
    /// Broker host or IP.
    pub host: String,
    /// Broker port.
    pub port: u16,
    /// Use TLS.
    pub tls: bool,
    /// CA chain in PEM. Public material, so it is exported.
    pub ca_pem: String,
    /// Username, if the broker wants one.
    pub username: String,
    /// Client id. Empty means `granite-<mac6>`.
    pub client_id: String,
    /// `<site>` level of the topic tree.
    pub site: String,
    /// Root of the topic tree.
    pub topic_root: String,
    /// QoS for everything the controller publishes.
    pub qos: u8,
    /// Keepalive.
    pub keepalive_s: u16,
    /// Full-state republish period.
    pub t_state_s: u32,
    /// Treat a successful connection as the confirmation of a broker change.
    pub auto_confirm_on_connect: bool,
    /// Skip the TLS not-before check (clock without SNTP).
    pub skip_time_check: bool,
}

impl Default for MqttCfg {
    fn default() -> Self {
        MqttCfg {
            enabled: false,
            host: String::new(),
            port: 8883,
            tls: true,
            ca_pem: String::new(),
            username: String::new(),
            client_id: String::new(),
            site: String::from("default"),
            topic_root: String::from("granite"),
            qos: 1,
            keepalive_s: 30,
            t_state_s: 60,
            auto_confirm_on_connect: false,
            skip_time_check: false,
        }
    }
}

/// A probe's ROM id mapped to a user name and a slot.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProbeCfg {
    /// ROM id as 16 hex digits.
    pub rom: String,
    /// User name.
    pub name: String,
}

/// The `nodes` section.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NodesCfg {
    /// Per-node settings, index 0 is node 1.
    pub nodes: [NodeSettings; NODE_COUNT],
    /// Order used by staggered actions and boot policies.
    pub order: [NodeId; NODE_COUNT],
    /// Action timings.
    pub timings: Timings,
    /// ROM id to name mapping, in slot order.
    pub probes: Vec<ProbeCfg>,
    /// Probe read period.
    pub t_probe_s: u32,
    /// Per-board correction for the VIN divider.
    pub vin_trim: f32,
}

impl Default for NodesCfg {
    fn default() -> Self {
        NodesCfg {
            nodes: array::from_fn(|i| NodeSettings::for_node((i + 1) as NodeId)),
            order: default_order(),
            timings: Timings::default(),
            probes: Vec::new(),
            t_probe_s: 10,
            vin_trim: 1.0,
        }
    }
}

impl NodesCfg {
    /// Boot policies in node order (index 0 = node 1).
    pub fn boot_policies(&self) -> [BootPolicy; NODE_COUNT] {
        array::from_fn(|i| self.nodes[i].boot_policy)
    }

    /// Sense flags in node order.
    pub fn sense_modes(&self) -> [SenseMode; NODE_COUNT] {
        array::from_fn(|i| self.nodes[i].sense)
    }
}

/// The `rules` section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RulesCfg {
    /// The rule set, at most [`MAX_RULES`].
    pub rules: Vec<Rule>,
}

impl Default for RulesCfg {
    fn default() -> Self {
        RulesCfg {
            rules: default_rules(),
        }
    }
}

/// Modbus TCP server settings. Off by default; the allow-list is the only
/// guard the protocol permits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ModbusCfg {
    /// Off by default.
    pub enabled: bool,
    /// TCP port.
    pub port: u16,
    /// Unit id the server answers for.
    pub unit_id: u8,
    /// Concurrent connections accepted.
    pub max_conn: u8,
    /// Allowed peers as addresses or CIDRs. Empty means the server
    /// refuses to start even when `enabled`.
    pub allow: Vec<String>,
}

impl Default for ModbusCfg {
    fn default() -> Self {
        ModbusCfg {
            enabled: false,
            port: 502,
            unit_id: 1,
            max_conn: 4,
            allow: Vec::new(),
        }
    }
}

/// The `sec` section. Public material only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SecCfg {
    /// Whether an admin password exists. False forces first-setup mode.
    pub admin_password_set: bool,
    /// Session cookie lifetime.
    pub session_hours: u16,
    /// Failed logins before the lockout.
    pub max_login_fails: u8,
    /// Lockout length.
    pub lockout_s: u32,
    /// Modbus server.
    pub modbus: ModbusCfg,
    /// Serve `/id` on plain HTTP as well (the recovery path).
    pub id_on_plain_http: bool,
    /// Fleet recovery public key, PEM. Not a secret.
    pub fleet_recovery_pubkey_pem: String,
    /// Device certificate, PEM. The private key lives in [`Secrets`].
    pub device_cert_pem: String,
}

impl Default for SecCfg {
    fn default() -> Self {
        SecCfg {
            admin_password_set: false,
            session_hours: 12,
            max_login_fails: 5,
            lockout_s: 60,
            modbus: ModbusCfg::default(),
            id_on_plain_http: true,
            fleet_recovery_pubkey_pem: String::new(),
            device_cert_pem: String::new(),
        }
    }
}

/// Log severity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogLevel {
    /// Errors only.
    Error,
    /// Errors and warnings. The default for the MQTT log topic.
    #[default]
    Warn,
    /// Plus informational lines.
    Info,
    /// Plus debug lines.
    Debug,
    /// Everything.
    Trace,
}

/// The `sys` section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SysCfg {
    /// Device id. Empty means `granite-<mac6>`.
    pub device_id: String,
    /// Level at or above which log records are published.
    pub log_level: LogLevel,
    /// How long a pending OTA image has to prove itself.
    pub t_validate_s: u32,
    /// Publish a core-dump summary on the next boot.
    pub coredump_report: bool,
    /// Time zone for display only; timestamps on the wire are UTC.
    pub timezone: String,
}

impl Default for SysCfg {
    fn default() -> Self {
        SysCfg {
            device_id: String::new(),
            log_level: LogLevel::Warn,
            t_validate_s: 600,
            coredump_report: true,
            timezone: String::from("UTC"),
        }
    }
}

/// The whole configuration. [`Default`] is a full, valid, safe config:
/// DHCP, MQTT off, Modbus off, boot policy `leave` everywhere, the three
/// example rules present and disabled.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Schema version of this document.
    pub schema_version: u32,
    /// Network.
    pub net: NetCfg,
    /// Broker.
    pub mqtt: MqttCfg,
    /// Nodes, timings, probes.
    pub nodes: NodesCfg,
    /// Rules.
    pub rules: RulesCfg,
    /// Security and Modbus.
    pub sec: SecCfg,
    /// Device-wide settings.
    pub sys: SysCfg,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            schema_version: SCHEMA_VERSION,
            net: NetCfg::default(),
            mqtt: MqttCfg::default(),
            nodes: NodesCfg::default(),
            rules: RulesCfg::default(),
            sec: SecCfg::default(),
            sys: SysCfg::default(),
        }
    }
}

/// A section that could not be loaded and fell back to its default.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SectionFallback {
    /// Which section.
    pub section: Section,
    /// Why.
    pub error: ConfigError,
}

impl fmt::Display for SectionFallback {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} fell back to defaults: {}", self.section, self.error)
    }
}

fn to_json<T: Serialize>(v: &T) -> Result<String, ConfigError> {
    serde_json::to_string(v).map_err(|e| ConfigError::Serialize(e.to_string()))
}

fn from_json<T: DeserializeOwned>(s: &str) -> Result<T, ConfigError> {
    serde_json::from_str(s).map_err(|e| ConfigError::Parse(e.to_string()))
}

impl Config {
    /// The default config.
    pub fn new() -> Self {
        Self::default()
    }

    /// Serialise one section.
    pub fn section_json(&self, section: Section) -> Result<String, ConfigError> {
        match section {
            Section::Net => to_json(&self.net),
            Section::Mqtt => to_json(&self.mqtt),
            Section::Nodes => to_json(&self.nodes),
            Section::Rules => to_json(&self.rules),
            Section::Sec => to_json(&self.sec),
            Section::Sys => to_json(&self.sys),
        }
    }

    /// Parse one section into this config. The other sections are left
    /// alone, and on error nothing changes.
    pub fn set_section_json(&mut self, section: Section, json: &str) -> Result<(), ConfigError> {
        match section {
            Section::Net => self.net = from_json(json)?,
            Section::Mqtt => self.mqtt = from_json(json)?,
            Section::Nodes => self.nodes = from_json(json)?,
            Section::Rules => self.rules = from_json(json)?,
            Section::Sec => self.sec = from_json(json)?,
            Section::Sys => self.sys = from_json(json)?,
        }
        self.normalise();
        Ok(())
    }

    /// Reset one section to its default.
    pub fn reset_section(&mut self, section: Section) {
        match section {
            Section::Net => self.net = NetCfg::default(),
            Section::Mqtt => self.mqtt = MqttCfg::default(),
            Section::Nodes => self.nodes = NodesCfg::default(),
            Section::Rules => self.rules = RulesCfg::default(),
            Section::Sec => self.sec = SecCfg::default(),
            Section::Sys => self.sys = SysCfg::default(),
        }
    }

    /// Load a whole config from per-section blobs, as stored in NVS.
    ///
    /// A section that is missing, unparseable or invalid falls back to its
    /// own default and is reported; the rest still loads. This is the
    /// behaviour the ADR asks for ("a corrupt section loses only itself").
    pub fn from_sections<F>(mut get: F) -> (Config, Vec<SectionFallback>)
    where
        F: FnMut(Section) -> Option<String>,
    {
        let mut cfg = Config::default();
        let mut fallbacks = Vec::new();
        for section in Section::ALL {
            let Some(json) = get(section) else {
                continue;
            };
            if let Err(error) = cfg.set_section_json(section, &json) {
                cfg.reset_section(section);
                fallbacks.push(SectionFallback { section, error });
            }
        }
        cfg.normalise();
        if let Err(errors) = cfg.validate() {
            // Validation failures are per-section by construction; find
            // the offending ones and default them.
            for section in Section::ALL {
                let mut probe = cfg.clone();
                probe.reset_section(section);
                if probe.validate().is_ok() {
                    cfg.reset_section(section);
                    fallbacks.push(SectionFallback {
                        section,
                        error: ConfigError::Invalid(errors.clone()),
                    });
                    break;
                }
            }
        }
        (cfg, fallbacks)
    }

    /// Export as one JSON document. Contains no secrets: [`Secrets`] is a
    /// separate struct that is never part of a `Config`.
    pub fn export_json(&self) -> Result<String, ConfigError> {
        serde_json::to_string_pretty(self).map_err(|e| ConfigError::Serialize(e.to_string()))
    }

    /// Import one JSON document, migrating and validating it.
    pub fn import_json(json: &str) -> Result<Config, ConfigError> {
        let mut cfg: Config = from_json(json)?;
        if cfg.schema_version > SCHEMA_VERSION {
            return Err(ConfigError::Schema {
                found: cfg.schema_version,
                expected: SCHEMA_VERSION,
            });
        }
        cfg.migrate();
        cfg.normalise();
        cfg.validate().map_err(ConfigError::Invalid)?;
        Ok(cfg)
    }

    /// Bring an older document up to the current schema. There is only
    /// one version so far, so this just stamps it.
    pub fn migrate(&mut self) {
        self.schema_version = SCHEMA_VERSION;
    }

    /// Fix up what can be fixed silently: node order, node names, rule
    /// count, timing clamping.
    pub fn normalise(&mut self) {
        self.nodes.order = sanitise_order(self.nodes.order);
        for (i, n) in self.nodes.nodes.iter_mut().enumerate() {
            if n.name.trim().is_empty() {
                n.name = crate::node::default_name((i + 1) as NodeId);
            }
        }
        self.nodes.timings = self.nodes.timings.clamped();
        if self.nodes.t_probe_s == 0 {
            self.nodes.t_probe_s = 10;
        }
        if !self.nodes.vin_trim.is_finite() || self.nodes.vin_trim <= 0.0 {
            self.nodes.vin_trim = 1.0;
        }
        self.rules.rules.truncate(MAX_RULES);
        if self.mqtt.qos > 2 {
            self.mqtt.qos = 1;
        }
        if self.mqtt.site.trim().is_empty() {
            self.mqtt.site = String::from("default");
        }
        if self.mqtt.topic_root.trim().is_empty() {
            self.mqtt.topic_root = String::from("granite");
        }
    }

    /// Everything that must hold before the config is used.
    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut errs: Vec<String> = Vec::new();
        if let Err(e) = self.nodes.timings.validate() {
            errs.push(e.to_string());
        }
        if self.nodes.probes.len() > crate::PROBE_SLOTS {
            errs.push(format!(
                "nodes.probes has {} entries, at most {} fit the data model",
                self.nodes.probes.len(),
                crate::PROBE_SLOTS
            ));
        }
        for p in &self.nodes.probes {
            if crate::hal::rom_id_from_hex(&p.rom).is_none() {
                errs.push(format!("nodes.probes: \"{}\" is not a ROM id", p.rom));
            }
        }
        if self.rules.rules.len() > MAX_RULES {
            errs.push(format!(
                "rules.rules has {} entries, the limit is {MAX_RULES}",
                self.rules.rules.len()
            ));
        }
        let mut ids: Vec<u8> = self.rules.rules.iter().map(|r| r.id).collect();
        ids.sort_unstable();
        if ids.windows(2).any(|w| w[0] == w[1]) {
            errs.push(String::from("rules.rules: duplicate rule id"));
        }
        if self.rules.rules.iter().any(|r| r.id == 0) {
            errs.push(String::from("rules.rules: rule id 0 is reserved"));
        }
        if self.net.ip_mode == IpMode::Static && self.net.address.is_empty() {
            errs.push(String::from("net.address is required for a static config"));
        }
        if self.mqtt.enabled && self.mqtt.host.trim().is_empty() {
            errs.push(String::from("mqtt.host is required when mqtt is enabled"));
        }
        if self.sec.modbus.enabled && self.sec.modbus.allow.is_empty() {
            errs.push(String::from(
                "sec.modbus.allow must list at least one peer when Modbus is enabled",
            ));
        }
        if errs.is_empty() { Ok(()) } else { Err(errs) }
    }

    /// Topic root for this device: `<root>/<site>/<device>`.
    pub fn topic_base(&self, device_id: &str) -> String {
        format!("{}/{}/{}", self.mqtt.topic_root, self.mqtt.site, device_id)
    }
}

/// One API token, stored hashed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ApiToken {
    /// Label shown on the security page.
    pub name: String,
    /// PBKDF2 hash of the token, hex.
    pub hash: String,
    /// Unix seconds when it was created.
    pub created_s: u64,
}

/// Everything that must never leave the device in an export.
///
/// Kept apart from [`Config`] on purpose: there is no code path that can
/// serialise a `Config` and accidentally include a secret, because a
/// `Config` does not contain one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Secrets {
    /// PBKDF2-HMAC-SHA256 of the admin password, hex.
    pub admin_hash: String,
    /// Per-device salt, hex.
    pub admin_salt: String,
    /// Iteration count used for the stored hash.
    pub admin_iters: u32,
    /// API tokens.
    pub api_tokens: Vec<ApiToken>,
    /// Broker password.
    pub mqtt_password: String,
    /// Client certificate private key, PEM.
    pub mqtt_client_key_pem: String,
    /// HTTPS device certificate private key, PEM.
    pub device_key_pem: String,
    /// Per-device recovery token, base32.
    pub recovery_token: String,
}

impl Default for Secrets {
    fn default() -> Self {
        Secrets {
            admin_hash: String::new(),
            admin_salt: String::new(),
            admin_iters: 20_000,
            api_tokens: Vec::new(),
            mqtt_password: String::new(),
            mqtt_client_key_pem: String::new(),
            device_key_pem: String::new(),
            recovery_token: String::new(),
        }
    }
}

impl Secrets {
    /// Serialise for the secrets namespace.
    pub fn to_json(&self) -> Result<String, ConfigError> {
        to_json(self)
    }

    /// Parse from the secrets namespace.
    pub fn from_json(json: &str) -> Result<Self, ConfigError> {
        from_json(json)
    }

    /// True when first-setup mode applies.
    pub fn admin_password_missing(&self) -> bool {
        self.admin_hash.is_empty()
    }
}
