//! The transport-agnostic HTTP API (ADR 0001 component 8).
//!
//! This module is the whole route table of the setup page and the JSON
//! API. It parses nothing network-specific: a transport (the ESP-IDF
//! server in `granite-fw/src/http.rs`, axum in `granite-sim`) turns a
//! request into an [`ApiRequest`], calls [`handle`], and writes the
//! [`ApiResponse`] back. Everything the core cannot know - the network
//! state, the OTA slots, identity, randomness, persistence - comes in as
//! a trait object on [`ApiCtx`], so the same route table runs on the
//! board and on the host.
//!
//! The API speaks the same `Command` vocabulary as MQTT: `POST
//! /api/v1/cmd` takes exactly the payload of the `.../cmd` topic, and
//! `POST /api/v1/nodes/<n>/<action>` is sugar for it. Both go through
//! [`Dispatcher`], which is the one hook into
//! [`crate::dispatch::dispatch`].
//!
//! Authentication (ADR component 8): one admin password, PBKDF2-HMAC-
//! SHA256 with [`PBKDF2_ITERS`] iterations and a per-device salt; a
//! session cookie of [`SESSION_BYTES`] random bytes valid for
//! `sec.session_hours`; `Authorization: Bearer <token>` for scripts with
//! the tokens stored hashed. While no password exists the API refuses
//! everything except `POST /api/v1/security/password`, `/id` and
//! `/recover`. [`SecCfg::max_login_fails`] failures lock logins out for
//! `sec.lockout_s`.
//!
//! [`SecCfg::max_login_fails`]: crate::config::SecCfg::max_login_fails

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::NodeId;
use crate::Target;
use crate::actuator::ActionKind;
use crate::config::{ApiToken, Config, Secrets, Section};
use crate::hal::BootReason;
use crate::msg::{Args, Command, CommandKind, EventKind, PROTOCOL_VERSION, Reply, State};
use crate::observed::Observed;
use crate::rules::Rule;

/// Prefix of every authenticated route.
pub const API_PREFIX: &str = "/api/v1/";

/// Name of the session cookie.
pub const SESSION_COOKIE: &str = "granite_session";

/// Random bytes in a session id (ADR: 32).
pub const SESSION_BYTES: usize = 32;

/// Random bytes in an API token.
pub const TOKEN_BYTES: usize = 32;

/// PBKDF2-HMAC-SHA256 iterations for the admin password (ADR: 20k).
pub const PBKDF2_ITERS: u32 = 20_000;

/// Bytes of derived key stored for a password or a token.
pub const HASH_BYTES: usize = 32;

/// Per-device salt length.
pub const SALT_BYTES: usize = 16;

/// Largest request body the transport may buffer. `firmware/upload` is
/// the one exception: it streams into an [`OtaSink`].
pub const MAX_BODY: usize = 64 * 1024;

/// How long a `/id` recovery nonce stays usable (ADR: 5 min, one use).
pub const NONCE_TTL_MS: u64 = 5 * 60 * 1000;

/// Shortest interval between two `/recover` attempts (ADR: 1 per minute).
pub const RECOVER_INTERVAL_MS: u64 = 60 * 1000;

/// What `/recover` signs over, after `device` and `nonce`.
pub const RECOVER_PURPOSE: &[u8] = b"factory-reset";

/// Most log lines `log/tail` returns in one response.
pub const LOG_TAIL_MAX: usize = 500;

// ---------------------------------------------------------------------
// Request and response
// ---------------------------------------------------------------------

/// The HTTP methods the API uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Method {
    /// GET.
    Get,
    /// HEAD.
    Head,
    /// POST.
    Post,
    /// PUT.
    Put,
    /// DELETE.
    Delete,
    /// OPTIONS.
    Options,
    /// Anything else; always answered with 405.
    Other,
}

impl Method {
    /// Parse a method name, case insensitively.
    pub fn parse(s: &str) -> Self {
        if s.eq_ignore_ascii_case("get") {
            Method::Get
        } else if s.eq_ignore_ascii_case("head") {
            Method::Head
        } else if s.eq_ignore_ascii_case("post") {
            Method::Post
        } else if s.eq_ignore_ascii_case("put") {
            Method::Put
        } else if s.eq_ignore_ascii_case("delete") {
            Method::Delete
        } else if s.eq_ignore_ascii_case("options") {
            Method::Options
        } else {
            Method::Other
        }
    }

    /// Uppercase name.
    pub const fn as_str(self) -> &'static str {
        match self {
            Method::Get => "GET",
            Method::Head => "HEAD",
            Method::Post => "POST",
            Method::Put => "PUT",
            Method::Delete => "DELETE",
            Method::Options => "OPTIONS",
            Method::Other => "OTHER",
        }
    }
}

/// What the client presented.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Auth {
    /// Nothing.
    None,
    /// A session cookie value.
    Session(String),
    /// An `Authorization: Bearer` token.
    Bearer(String),
}

impl Auth {
    /// Pick the credential out of a cookie header and an authorization
    /// header, preferring the bearer token.
    pub fn from_headers(cookie: Option<&str>, authorization: Option<&str>) -> Self {
        if let Some(a) = authorization {
            let t = a.trim();
            if let Some(rest) = t
                .strip_prefix("Bearer ")
                .or_else(|| t.strip_prefix("bearer "))
            {
                let rest = rest.trim();
                if !rest.is_empty() {
                    return Auth::Bearer(String::from(rest));
                }
            }
        }
        if let Some(c) = cookie {
            for part in c.split(';') {
                let part = part.trim();
                if let Some(v) = part.strip_prefix(SESSION_COOKIE)
                    && let Some(v) = v.strip_prefix('=')
                    && !v.is_empty()
                {
                    return Auth::Session(String::from(v));
                }
            }
        }
        Auth::None
    }
}

/// The outcome of a streamed `firmware/upload` body. The transport fills
/// this in after it has pushed every chunk into the [`OtaSink`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UploadOutcome {
    /// Bytes handed to the sink.
    pub bytes: u64,
    /// Set when the transport or the sink gave up; the handler then only
    /// reports the failure.
    pub error: Option<String>,
}

/// One request, already parsed by the transport.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiRequest {
    /// Method.
    pub method: Method,
    /// Path, without the query string.
    pub path: String,
    /// Raw query string, without the `?`.
    pub query: String,
    /// Body, at most [`MAX_BODY`] bytes.
    pub body: Vec<u8>,
    /// Credential.
    pub auth: Auth,
    /// Peer address, for the log. Empty when the transport cannot tell.
    pub peer: String,
    /// Set only for `firmware/upload`, where the transport streams the
    /// body into the sink itself.
    pub upload: Option<UploadOutcome>,
}

impl ApiRequest {
    /// A request with an empty body and no credential.
    pub fn new(method: Method, path: impl Into<String>) -> Self {
        ApiRequest {
            method,
            path: path.into(),
            query: String::new(),
            body: Vec::new(),
            auth: Auth::None,
            peer: String::new(),
            upload: None,
        }
    }

    /// Builder: JSON body.
    pub fn with_body(mut self, body: impl AsRef<[u8]>) -> Self {
        self.body = body.as_ref().to_vec();
        self
    }

    /// Builder: credential.
    pub fn with_auth(mut self, auth: Auth) -> Self {
        self.auth = auth;
        self
    }

    /// Builder: query string.
    pub fn with_query(mut self, query: impl Into<String>) -> Self {
        self.query = query.into();
        self
    }

    /// Value of one query parameter.
    pub fn param(&self, name: &str) -> Option<&str> {
        self.query.split('&').find_map(|kv| {
            let (k, v) = kv.split_once('=')?;
            (k == name).then_some(v)
        })
    }

    /// Parse the body as JSON. An empty body is an empty object, so
    /// handlers can treat "no arguments" and `{}` alike.
    pub fn json(&self) -> Result<Value, String> {
        if self.body.is_empty() {
            return Ok(json!({}));
        }
        let text =
            core::str::from_utf8(&self.body).map_err(|_| String::from("body is not UTF-8"))?;
        serde_json::from_str(text).map_err(|e| e.to_string())
    }
}

/// A large body the transport produces itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamKind {
    /// A gzipped static asset, by its canonical path (`/index.html`).
    Asset(String),
}

/// What comes back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Body {
    /// No body.
    Empty,
    /// A buffered body.
    Bytes(Vec<u8>),
    /// The transport writes the body (static assets, chunked payloads).
    Stream(StreamKind),
}

impl Body {
    /// Buffered length, `None` for a streamed body.
    pub fn len(&self) -> Option<usize> {
        match self {
            Body::Empty => Some(0),
            Body::Bytes(b) => Some(b.len()),
            Body::Stream(_) => None,
        }
    }

    /// True for an empty buffered body.
    pub fn is_empty(&self) -> bool {
        self.len() == Some(0)
    }
}

/// The response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiResponse {
    /// HTTP status.
    pub status: u16,
    /// Content type. Empty for an empty body.
    pub content_type: &'static str,
    /// The payload.
    pub body: Body,
    /// A `Set-Cookie` value, on login and logout.
    pub set_cookie: Option<String>,
    /// Extra headers (`Location`, `Content-Disposition`).
    pub headers: Vec<(&'static str, String)>,
}

const JSON: &str = "application/json";
const TEXT: &str = "text/plain; charset=utf-8";

impl ApiResponse {
    /// A JSON response.
    pub fn json(status: u16, value: Value) -> Self {
        let body = serde_json::to_vec(&value).unwrap_or_else(|_| b"{}".to_vec());
        ApiResponse {
            status,
            content_type: JSON,
            body: Body::Bytes(body),
            set_cookie: None,
            headers: Vec::new(),
        }
    }

    /// 200 with a JSON body.
    pub fn ok(value: Value) -> Self {
        Self::json(200, value)
    }

    /// 200 with `{"ok":true}`.
    pub fn ok_true() -> Self {
        Self::ok(json!({"ok": true}))
    }

    /// 204 with no body: a write that has nothing to report back.
    pub fn no_content() -> Self {
        ApiResponse {
            status: 204,
            content_type: "",
            body: Body::Empty,
            set_cookie: None,
            headers: Vec::new(),
        }
    }

    /// An error as `{"ok":false,"error":"..."}`.
    pub fn error(status: u16, message: impl Into<String>) -> Self {
        Self::json(status, json!({"ok": false, "error": message.into()}))
    }

    /// A plain text response.
    pub fn text(status: u16, body: impl Into<String>) -> Self {
        ApiResponse {
            status,
            content_type: TEXT,
            body: Body::Bytes(body.into().into_bytes()),
            set_cookie: None,
            headers: Vec::new(),
        }
    }

    /// A body the transport produces.
    pub fn stream(status: u16, content_type: &'static str, kind: StreamKind) -> Self {
        ApiResponse {
            status,
            content_type,
            body: Body::Stream(kind),
            set_cookie: None,
            headers: Vec::new(),
        }
    }

    /// A redirect.
    pub fn redirect(location: impl Into<String>) -> Self {
        ApiResponse {
            status: 302,
            content_type: "",
            body: Body::Empty,
            set_cookie: None,
            headers: vec![("Location", location.into())],
        }
    }

    /// Builder: set the cookie.
    pub fn with_cookie(mut self, cookie: impl Into<String>) -> Self {
        self.set_cookie = Some(cookie.into());
        self
    }

    /// Builder: add a header.
    pub fn with_header(mut self, name: &'static str, value: impl Into<String>) -> Self {
        self.headers.push((name, value.into()));
        self
    }

    /// True for 2xx.
    pub const fn is_ok(&self) -> bool {
        self.status >= 200 && self.status < 300
    }
}

// ---------------------------------------------------------------------
// What the platform tells the API
// ---------------------------------------------------------------------

/// The network summary shown on the status page.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct NetStatus {
    /// Ethernet link.
    pub link_up: bool,
    /// `dhcp`, `static` or `autoip`.
    pub ip_mode: String,
    /// Current address.
    pub ip: String,
    /// Netmask.
    pub netmask: String,
    /// Gateway.
    pub gateway: String,
    /// Name servers.
    pub dns: Vec<String>,
    /// Hostname and mDNS name in use.
    pub hostname: String,
    /// True while DHCP runs alongside a static config (dead-man).
    pub dhcp_fallback: bool,
    /// True once SNTP has set the clock.
    pub sntp_synced: bool,
}

/// The broker summary shown on the status page.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MqttStatus {
    /// Configured at all.
    pub enabled: bool,
    /// Session up.
    pub connected: bool,
    /// `host:port`.
    pub broker: String,
    /// Last error, empty when there was none.
    pub last_error: String,
}

/// One OTA slot.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SlotInfo {
    /// Partition label (`factory`, `ota_0`, `ota_1`).
    pub label: String,
    /// `running`, `valid`, `pending_verify`, `invalid`, `aborted`, `empty`.
    pub state: String,
    /// Firmware version in the slot, when it could be read.
    pub version: String,
    /// Partition size in bytes.
    pub size: u32,
}

/// The firmware summary of the Firmware page.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct OtaStatus {
    /// Label of the running partition.
    pub running: String,
    /// State of the running image.
    pub state: String,
    /// True while the running image still has to prove itself.
    pub pending_verify: bool,
    /// Seconds left of the validation window, while pending.
    pub validate_left_s: Option<u32>,
    /// Every partition that can hold an app.
    pub slots: Vec<SlotInfo>,
    /// Key id of the signing key this image trusts.
    pub key_id: String,
    /// True when a previous image can be booted again.
    pub rollback_available: bool,
}

/// What a staged, not yet confirmed configuration looks like.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct StagedInfo {
    /// Sections waiting for a confirmation.
    pub sections: Vec<Section>,
    /// Seconds left before the automatic revert.
    pub seconds_left: u32,
}

/// What [`OtaSink::finish`] reports.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct OtaFinish {
    /// Partition the image landed in.
    pub slot: String,
    /// Bytes written.
    pub bytes: u64,
    /// True when the image only becomes active after a reboot.
    pub reboot_required: bool,
}

// ---------------------------------------------------------------------
// The traits the platform implements
// ---------------------------------------------------------------------

/// Everything about the running system the core does not keep itself.
///
/// Implemented by `granite-fw/src/platform` on the board and by
/// `granite-sim` on the host.
pub trait Platform {
    /// Firmware version string, as published in `status`.
    fn fw_version(&self) -> String;

    /// Seconds since boot.
    fn uptime_s(&self) -> u32;

    /// Why this boot happened.
    fn boot_reason(&self) -> BootReason;

    /// Free heap in bytes, for the maintenance page.
    fn free_heap(&self) -> u32;

    /// Network summary.
    fn net(&self) -> NetStatus;

    /// Broker summary.
    fn mqtt(&self) -> MqttStatus;

    /// Firmware slots and OTA state.
    fn ota(&self) -> OtaStatus;

    /// Last `lines` log records, oldest first.
    fn log_tail(&self, lines: usize) -> Vec<String>;

    /// Boot the previous image (ADR component 11, "Rollback").
    fn ota_rollback(&mut self) -> Result<(), String>;

    /// Mark the running image valid.
    fn ota_mark_valid(&mut self) -> Result<(), String>;

    /// Publish an event. The API uses it for the security events the ADR
    /// asks to be logged (failed logins, lockout, recovery).
    fn event(&mut self, kind: EventKind);
}

/// Identity and recovery (ADR component 13).
pub trait Identity {
    /// Device id, `granite-<mac6>` unless configured otherwise.
    fn device_id(&self) -> String;

    /// Ethernet MAC as `aa:bb:cc:dd:ee:ff`.
    fn mac(&self) -> String;

    /// SHA-256 of the HTTPS device certificate, hex.
    fn cert_sha256(&self) -> String;

    /// The per-device recovery token, but only while it has never been
    /// shown. First setup displays it once; afterwards this is `None` and
    /// the USB console is the only way to see it again.
    fn recovery_token_once(&mut self) -> Option<String>;

    /// Check a per-device recovery token.
    fn verify_recovery_token(&mut self, token: &str) -> bool;

    /// Check a fleet-key signature over `message`. `sig` is whatever the
    /// host tool produced (base64 of a DER ECDSA signature).
    fn verify_fleet_sig(&self, message: &[u8], sig: &str) -> bool;

    /// Replace the fleet recovery public key (PEM).
    fn set_fleet_pubkey(&mut self, pem: &str) -> Result<(), String>;

    /// Replace the HTTPS device certificate and its private key (PEM).
    fn set_device_cert(&mut self, cert_pem: &str, key_pem: &str) -> Result<(), String>;
}

/// Commit-confirmed configuration (ADR component 6, "Commit-confirmed
/// network changes"). Sections that can cut the session never go
/// straight to storage.
pub trait NetControl {
    /// Stage `json` for `section`, apply it, and start the confirm timer.
    /// Returns the window in seconds.
    fn stage_and_apply(&mut self, section: Section, json: &str) -> Result<u32, String>;

    /// Confirm whatever is staged. False when nothing was staged.
    fn confirm(&mut self) -> bool;

    /// Drop the staged configuration and restore the previous one.
    fn revert(&mut self) -> bool;

    /// What is staged, if anything.
    fn staged(&self) -> Option<StagedInfo>;
}

/// Where a pushed firmware image goes (ADR component 11).
pub trait OtaSink {
    /// Start an update. `total_len` is the Content-Length when known.
    fn begin(&mut self, total_len: Option<u64>) -> Result<(), String>;

    /// Append a chunk.
    fn write(&mut self, chunk: &[u8]) -> Result<(), String>;

    /// Verify and activate the image.
    fn finish(&mut self) -> Result<OtaFinish, String>;

    /// Give up on the image in progress.
    fn abort(&mut self);
}

/// Randomness for session ids, API tokens and salts.
pub trait Rng {
    /// Fill `out` with random bytes.
    fn fill(&mut self, out: &mut [u8]);
}

/// The one hook into [`crate::dispatch::dispatch`]. An implementation
/// either locks the controller and calls it, or sends the command to the
/// actuator task and waits for the ack.
pub trait Dispatcher {
    /// Run one command and return its ack.
    fn dispatch(&mut self, cmd: &Command) -> Reply;
}

/// Persistence for the parts of the configuration the API writes itself.
pub trait Store {
    /// Load the secrets blob.
    fn load_secrets(&mut self) -> Secrets;

    /// Store the secrets blob.
    fn save_secrets(&mut self, secrets: &Secrets) -> Result<(), String>;

    /// Persist one configuration section as `json`.
    fn save_section(&mut self, section: Section, json: &str) -> Result<(), String>;
}

// ---------------------------------------------------------------------
// Hashing and sessions
// ---------------------------------------------------------------------

/// Lowercase hex of a byte slice.
pub fn hex(bytes: &[u8]) -> String {
    use core::fmt::Write as _;
    let mut s = String::new();
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// Parse lowercase or uppercase hex. `None` on any non-hex input.
pub fn unhex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(s.len() / 2);
    for pair in b.chunks(2) {
        let hi = (pair[0] as char).to_digit(16)?;
        let lo = (pair[1] as char).to_digit(16)?;
        out.push((hi * 16 + lo) as u8);
    }
    Some(out)
}

/// Compare two byte slices without an early exit.
pub fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// PBKDF2-HMAC-SHA256 of `secret` with `salt`, hex encoded.
pub fn pbkdf2_hex(secret: &[u8], salt: &[u8], iters: u32) -> String {
    let mut out = [0u8; HASH_BYTES];
    pbkdf2::pbkdf2_hmac::<sha2::Sha256>(secret, salt, iters, &mut out);
    hex(&out)
}

/// True when `secret` matches a stored hash. Any malformed stored value
/// fails closed.
pub fn verify_hash(secret: &[u8], salt_hex: &str, hash_hex: &str, iters: u32) -> bool {
    if salt_hex.is_empty() || hash_hex.is_empty() || iters == 0 {
        return false;
    }
    let Some(salt) = unhex(salt_hex) else {
        return false;
    };
    let got = pbkdf2_hex(secret, &salt, iters);
    ct_eq(got.as_bytes(), hash_hex.as_bytes())
}

/// One live browser session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    /// Cookie value, hex.
    pub id: String,
    /// Monotonic millisecond at which it stops being valid.
    pub expires_ms: u64,
}

/// Login state the API keeps between requests: sessions, the lockout
/// counter, the recovery nonce and the recovery rate limit. The
/// transport owns one of these for the lifetime of the server.
#[derive(Debug, Clone, Default)]
pub struct AuthState {
    sessions: Vec<Session>,
    fails: u32,
    locked_until_ms: u64,
    nonce: Option<(String, u64)>,
    last_recover_ms: Option<u64>,
    next_cmd: u32,
}

impl AuthState {
    /// Empty state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Live sessions, after dropping the expired ones.
    pub fn sessions(&mut self, now_ms: u64) -> &[Session] {
        self.sessions.retain(|s| s.expires_ms > now_ms);
        &self.sessions
    }

    /// True while logins are locked out.
    pub fn is_locked(&self, now_ms: u64) -> bool {
        now_ms < self.locked_until_ms
    }

    /// Seconds left of the lockout.
    pub fn lockout_left_s(&self, now_ms: u64) -> u32 {
        (self.locked_until_ms.saturating_sub(now_ms) / 1000) as u32
    }

    /// Failed logins since the last success.
    pub const fn fails(&self) -> u32 {
        self.fails
    }

    /// Note a failed login. Returns true when this one triggered the
    /// lockout.
    pub fn note_fail(&mut self, now_ms: u64, max_fails: u8, lockout_s: u32) -> bool {
        self.fails += 1;
        if max_fails > 0 && self.fails >= u32::from(max_fails) {
            self.locked_until_ms = now_ms + u64::from(lockout_s) * 1000;
            self.fails = 0;
            return true;
        }
        false
    }

    /// Clear the failure counter and the lockout.
    pub fn note_success(&mut self) {
        self.fails = 0;
        self.locked_until_ms = 0;
    }

    /// Mint a session.
    pub fn open_session(&mut self, rng: &mut dyn Rng, now_ms: u64, hours: u16) -> String {
        let mut raw = [0u8; SESSION_BYTES];
        rng.fill(&mut raw);
        let id = hex(&raw);
        let hours = if hours == 0 { 12 } else { hours };
        self.sessions.retain(|s| s.expires_ms > now_ms);
        self.sessions.push(Session {
            id: id.clone(),
            expires_ms: now_ms + u64::from(hours) * 3_600_000,
        });
        id
    }

    /// True when `id` is a live session.
    pub fn has_session(&mut self, id: &str, now_ms: u64) -> bool {
        self.sessions.retain(|s| s.expires_ms > now_ms);
        self.sessions
            .iter()
            .any(|s| ct_eq(s.id.as_bytes(), id.as_bytes()))
    }

    /// Drop one session.
    pub fn close_session(&mut self, id: &str) {
        self.sessions.retain(|s| s.id != id);
    }

    /// Drop every session (password change, factory reset).
    pub fn close_all(&mut self) {
        self.sessions.clear();
    }

    /// Issue a fresh recovery nonce, replacing any previous one.
    pub fn new_nonce(&mut self, rng: &mut dyn Rng, now_ms: u64) -> String {
        let mut raw = [0u8; 16];
        rng.fill(&mut raw);
        let nonce = hex(&raw);
        self.nonce = Some((nonce.clone(), now_ms + NONCE_TTL_MS));
        nonce
    }

    /// Consume the nonce: true only for the current, unexpired one.
    pub fn take_nonce(&mut self, nonce: &str, now_ms: u64) -> bool {
        matches!(
            self.nonce.take(),
            Some((want, until)) if until > now_ms && ct_eq(want.as_bytes(), nonce.as_bytes())
        )
    }

    /// True when a `/recover` attempt is allowed now.
    pub fn recover_allowed(&self, now_ms: u64) -> bool {
        match self.last_recover_ms {
            None => true,
            Some(t) => now_ms.saturating_sub(t) >= RECOVER_INTERVAL_MS,
        }
    }

    /// Note a `/recover` attempt, successful or not.
    pub fn note_recover(&mut self, now_ms: u64) {
        self.last_recover_ms = Some(now_ms);
    }

    /// A command id for a request that did not bring one.
    pub fn next_command_id(&mut self, prefix: &str) -> String {
        self.next_cmd = self.next_cmd.wrapping_add(1);
        format!("{prefix}-{}", self.next_cmd)
    }
}

// ---------------------------------------------------------------------
// The context
// ---------------------------------------------------------------------

/// Everything one request may touch.
pub struct ApiCtx<'a> {
    /// Monotonic now, milliseconds.
    pub now_ms: u64,
    /// The live configuration. Handlers mutate it and persist through
    /// [`Store`] or [`NetControl`].
    pub config: &'a mut Config,
    /// The current sensor snapshot.
    pub observed: &'a Observed,
    /// Sessions, lockout, nonce.
    pub auth: &'a mut AuthState,
    /// Secrets and section persistence.
    pub store: &'a mut dyn Store,
    /// System state.
    pub platform: &'a mut dyn Platform,
    /// Identity and recovery.
    pub identity: &'a mut dyn Identity,
    /// Commit-confirm.
    pub net: &'a mut dyn NetControl,
    /// Firmware upload target.
    pub ota: &'a mut dyn OtaSink,
    /// Randomness.
    pub rng: &'a mut dyn Rng,
    /// The command path into the core dispatcher.
    pub commands: &'a mut dyn Dispatcher,
}

impl ApiCtx<'_> {
    /// True when an admin password exists.
    pub fn password_set(&mut self) -> bool {
        !self.store.load_secrets().admin_password_missing()
    }

    /// True when the request carries a valid credential.
    pub fn is_authenticated(&mut self, req: &ApiRequest) -> bool {
        match &req.auth {
            Auth::None => false,
            Auth::Session(id) => self.auth.has_session(id, self.now_ms),
            Auth::Bearer(token) => {
                let secrets = self.store.load_secrets();
                if secrets.admin_password_missing() {
                    return false;
                }
                secrets.api_tokens.iter().any(|t| {
                    verify_hash(
                        token.as_bytes(),
                        &secrets.admin_salt,
                        &t.hash,
                        secrets.admin_iters,
                    )
                })
            }
        }
    }
}

// ---------------------------------------------------------------------
// The route table
// ---------------------------------------------------------------------

/// What a route needs before it runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// No credential at all: identity, recovery, login, static assets.
    Public,
    /// Public while no admin password exists, authenticated afterwards.
    /// Only `security/password` is in this class: it is how first setup
    /// gets out of the unauthenticated state.
    PasswordSetup,
    /// A session cookie or a bearer token.
    Authed,
}

/// Every route the API answers. One variant per resource; the method
/// picks the operation inside the handler, so a known resource with the
/// wrong method answers 405 and an unknown path answers 404.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    /// `GET /id`.
    Id,
    /// `POST /recover`.
    Recover,
    /// `POST /api/v1/session` (login), `DELETE` (logout).
    Session,
    /// `GET /api/v1/status`.
    Status,
    /// `GET /api/v1/state`.
    State,
    /// `GET /api/v1/nodes`.
    Nodes,
    /// `POST /api/v1/nodes/<n|all>/<action>`.
    NodeAction {
        /// Target node, `None` for `all`.
        node: Option<NodeId>,
        /// The actuator action.
        action: ActionKind,
    },
    /// `POST /api/v1/cmd`.
    Cmd,
    /// `GET`/`PUT /api/v1/config`.
    Config,
    /// `GET`/`PUT /api/v1/config/<section>`.
    ConfigSection(Section),
    /// `POST /api/v1/config/confirm`.
    ConfigConfirm,
    /// `POST /api/v1/config/revert`.
    ConfigRevert,
    /// `GET /api/v1/config/export`.
    ConfigExport,
    /// `POST /api/v1/config/import`.
    ConfigImport,
    /// `GET`/`PUT /api/v1/rules`.
    Rules,
    /// `POST /api/v1/rules/<id>/ack`.
    RuleAck(u8),
    /// `POST /api/v1/probes/scan`.
    ProbeScan,
    /// `GET /api/v1/firmware`.
    Firmware,
    /// `POST /api/v1/firmware/upload`.
    FirmwareUpload,
    /// `POST /api/v1/firmware/rollback`.
    FirmwareRollback,
    /// `POST /api/v1/firmware/mark-valid`.
    FirmwareMarkValid,
    /// `POST /api/v1/reboot`.
    Reboot,
    /// `POST /api/v1/factory-reset`.
    FactoryReset,
    /// `POST /api/v1/security/password`.
    Password,
    /// `GET`/`POST /api/v1/security/tokens`.
    Tokens,
    /// `DELETE /api/v1/security/tokens/<name>`.
    Token(String),
    /// `PUT /api/v1/security/fleet-key`.
    FleetKey,
    /// `PUT /api/v1/security/cert`.
    Cert,
    /// `GET`/`PUT /api/v1/security/modbus-allowlist`.
    ModbusAllowlist,
    /// `PUT /api/v1/security/mqtt-credentials`.
    MqttCredentials,
    /// `GET /api/v1/log/tail`.
    LogTail,
    /// Anything else: a static asset by canonical path.
    Asset(String),
}

/// What `route` requires.
pub const fn access_of(route: &Route) -> Access {
    match route {
        Route::Id | Route::Recover | Route::Session | Route::Asset(_) => Access::Public,
        Route::Password => Access::PasswordSetup,
        _ => Access::Authed,
    }
}

/// Collapse `//`, drop a trailing slash (except for the root) and strip
/// any query fragment a transport left in place.
fn normalise_path(path: &str) -> String {
    let path = path.split('?').next().unwrap_or("");
    let mut out = String::from("/");
    for seg in path.split('/').filter(|s| !s.is_empty() && *s != ".") {
        if seg == ".." {
            // No traversal: the asset namespace is flat.
            continue;
        }
        if out.len() > 1 {
            out.push('/');
        }
        out.push_str(seg);
    }
    out
}

/// The canonical asset path for a request path: `/` becomes
/// `/index.html`, everything else keeps its name.
fn asset_path(path: &str) -> String {
    if path == "/" {
        String::from("/index.html")
    } else {
        String::from(path)
    }
}

/// Match a path onto the route table. `None` means 404.
pub fn route_of(path: &str) -> Option<Route> {
    let path = normalise_path(path);
    match path.as_str() {
        "/id" => return Some(Route::Id),
        "/recover" => return Some(Route::Recover),
        _ => {}
    }
    let Some(rest) = path.strip_prefix(API_PREFIX) else {
        if path.starts_with("/api") {
            // A versioned prefix that is not ours.
            return None;
        }
        return Some(Route::Asset(asset_path(&path)));
    };
    let segs: Vec<&str> = rest.split('/').filter(|s| !s.is_empty()).collect();
    let route = match segs.as_slice() {
        ["session"] => Route::Session,
        ["status"] => Route::Status,
        ["state"] => Route::State,
        ["nodes"] => Route::Nodes,
        ["nodes", target, action] => Route::NodeAction {
            node: parse_target(target)?,
            action: ActionKind::from_str_opt(action)?,
        },
        ["cmd"] => Route::Cmd,
        ["config"] => Route::Config,
        ["config", "export"] => Route::ConfigExport,
        ["config", "import"] => Route::ConfigImport,
        ["config", "confirm"] => Route::ConfigConfirm,
        ["config", "revert"] => Route::ConfigRevert,
        ["config", section] => Route::ConfigSection(Section::from_key(section)?),
        ["rules"] => Route::Rules,
        ["rules", id, "ack"] => Route::RuleAck(id.parse().ok()?),
        ["probes", "scan"] => Route::ProbeScan,
        ["firmware"] => Route::Firmware,
        ["firmware", "upload"] => Route::FirmwareUpload,
        ["firmware", "rollback"] => Route::FirmwareRollback,
        ["firmware", "mark-valid"] => Route::FirmwareMarkValid,
        ["reboot"] => Route::Reboot,
        ["factory-reset"] => Route::FactoryReset,
        ["security", "password"] => Route::Password,
        ["security", "tokens"] => Route::Tokens,
        ["security", "tokens", name] => Route::Token(String::from(*name)),
        ["security", "fleet-key"] => Route::FleetKey,
        ["security", "cert"] => Route::Cert,
        ["security", "modbus-allowlist"] => Route::ModbusAllowlist,
        ["security", "mqtt-credentials"] => Route::MqttCredentials,
        ["log", "tail"] => Route::LogTail,
        _ => return None,
    };
    Some(route)
}

/// `1`..`8` or `all`.
fn parse_target(s: &str) -> Option<Option<NodeId>> {
    if s.eq_ignore_ascii_case("all") {
        return Some(None);
    }
    let n: NodeId = s.parse().ok()?;
    crate::is_node(n).then_some(Some(n))
}

/// Route one request.
///
/// The only entry point. A transport must not implement any policy of
/// its own: authentication, first-boot gating, the lockout and the body
/// cap all live here (the cap as [`MAX_BODY`], which the transport
/// enforces because it owns the socket).
pub fn handle(req: ApiRequest, ctx: &mut ApiCtx<'_>) -> ApiResponse {
    let Some(route) = route_of(&req.path) else {
        return ApiResponse::error(404, "no such route");
    };

    match access_of(&route) {
        Access::Public => {}
        Access::PasswordSetup => {
            if ctx.password_set() && !ctx.is_authenticated(&req) {
                return unauthorized();
            }
        }
        Access::Authed => {
            if !ctx.password_set() {
                return ApiResponse::error(
                    403,
                    "no admin password is set; POST /api/v1/security/password first",
                );
            }
            if !ctx.is_authenticated(&req) {
                return unauthorized();
            }
        }
    }

    dispatch_route(route, &req, ctx)
}

fn unauthorized() -> ApiResponse {
    ApiResponse::error(401, "authentication required")
        .with_header("WWW-Authenticate", "Bearer realm=\"granite\"")
}

fn method_not_allowed() -> ApiResponse {
    ApiResponse::error(405, "method not allowed")
}

fn dispatch_route(route: Route, req: &ApiRequest, ctx: &mut ApiCtx<'_>) -> ApiResponse {
    use Method::{Delete, Get, Head, Post, Put};
    match (&route, req.method) {
        (Route::Id, Get | Head) => identity_response(ctx),
        (Route::Recover, Post) => recover(req, ctx),
        (Route::Session, Post) => login(req, ctx),
        (Route::Session, Delete) => logout(req, ctx),
        (Route::Session, Get) => session_info(req, ctx),
        (Route::Status, Get | Head) => status(ctx),
        (Route::State, Get | Head) => {
            let state = State::from_observed(ctx.observed, &ctx.config.nodes, ctx.now_ms);
            match serde_json::to_value(&state) {
                Ok(v) => ApiResponse::ok(v),
                Err(e) => ApiResponse::error(500, e.to_string()),
            }
        }
        (Route::Nodes, Get | Head) => nodes(ctx),
        (Route::NodeAction { node, action }, Post) => node_action(*node, *action, req, ctx),
        (Route::Cmd, Post) => raw_command(req, ctx),
        (Route::Config, Get | Head) => match ctx.config.export_json() {
            Ok(text) => json_text(200, text),
            Err(e) => ApiResponse::error(500, e.to_string()),
        },
        (Route::Config, Put) => import_config(req, ctx),
        (Route::ConfigSection(section), Get | Head) => match ctx.config.section_json(*section) {
            Ok(text) => json_text(200, text),
            Err(e) => ApiResponse::error(500, e.to_string()),
        },
        (Route::ConfigSection(section), Put) => put_section_body(*section, req, ctx),
        (Route::ConfigExport, Get | Head) => export_config(ctx),
        (Route::ConfigImport, Post) => import_config(req, ctx),
        (Route::ConfigConfirm, Post) => {
            if ctx.net.confirm() {
                ctx.platform.event(EventKind::Config {
                    section: None,
                    change: String::from("confirmed"),
                    detail: None,
                });
                ApiResponse::ok(json!({"ok": true, "confirmed": true}))
            } else {
                ApiResponse::error(409, "nothing is staged")
            }
        }
        (Route::ConfigRevert, Post) => {
            if ctx.net.revert() {
                ctx.platform.event(EventKind::Config {
                    section: None,
                    change: String::from("reverted"),
                    detail: None,
                });
                ApiResponse::ok(json!({"ok": true, "reverted": true}))
            } else {
                ApiResponse::error(409, "nothing is staged")
            }
        }
        (Route::Rules, Get | Head) => match ctx.config.section_json(Section::Rules) {
            Ok(text) => json_text(200, text),
            Err(e) => ApiResponse::error(500, e.to_string()),
        },
        (Route::Rules, Put) => put_rules(req, ctx),
        (Route::RuleAck(id), Post) => {
            let mut cmd = new_command(ctx, CommandKind::RuleAck);
            cmd.args.rule = Some(*id);
            reply_response(ctx.commands.dispatch(&cmd))
        }
        (Route::ProbeScan, Post) => {
            let cmd = new_command(ctx, CommandKind::ProbeScan);
            reply_response(ctx.commands.dispatch(&cmd))
        }
        (Route::Firmware, Get | Head) => match serde_json::to_value(ctx.platform.ota()) {
            Ok(v) => ApiResponse::ok(v),
            Err(e) => ApiResponse::error(500, e.to_string()),
        },
        (Route::FirmwareUpload, Post) => firmware_upload(req, ctx),
        (Route::FirmwareRollback, Post) => match ctx.platform.ota_rollback() {
            Ok(()) => ApiResponse::ok(json!({"ok": true, "result": "rollback scheduled"})),
            Err(e) => ApiResponse::error(409, e),
        },
        (Route::FirmwareMarkValid, Post) => match ctx.platform.ota_mark_valid() {
            Ok(()) => ApiResponse::ok(json!({"ok": true, "result": "marked valid"})),
            Err(e) => ApiResponse::error(409, e),
        },
        (Route::Reboot, Post) => {
            let cmd = new_command(ctx, CommandKind::Reboot);
            reply_response(ctx.commands.dispatch(&cmd))
        }
        (Route::FactoryReset, Post) => factory_reset(req, ctx),
        (Route::Password, Post) => set_password(req, ctx),
        (Route::Tokens, Get | Head) => list_tokens(ctx),
        (Route::Tokens, Post) => create_token(req, ctx),
        (Route::Token(name), Delete) => delete_token(name, ctx),
        (Route::FleetKey, Put | Post) => set_fleet_key(req, ctx),
        (Route::Cert, Put | Post) => set_cert(req, ctx),
        (Route::ModbusAllowlist, Get | Head) => {
            match serde_json::to_value(&ctx.config.sec.modbus) {
                Ok(v) => ApiResponse::ok(v),
                Err(e) => ApiResponse::error(500, e.to_string()),
            }
        }
        (Route::ModbusAllowlist, Put | Post) => set_modbus_allowlist(req, ctx),
        (Route::MqttCredentials, Put | Post) => set_mqtt_credentials(req, ctx),
        (Route::LogTail, Get | Head) => log_tail(req, ctx),
        (Route::Asset(path), Get | Head) => asset(path),
        _ => method_not_allowed(),
    }
}

/// A JSON response whose body is already serialised.
fn json_text(status: u16, text: String) -> ApiResponse {
    ApiResponse {
        status,
        content_type: JSON,
        body: Body::Bytes(text.into_bytes()),
        set_cookie: None,
        headers: Vec::new(),
    }
}

fn asset(path: &str) -> ApiResponse {
    ApiResponse::stream(200, "", StreamKind::Asset(String::from(path)))
}

/// A command with an API-generated id.
fn new_command(ctx: &mut ApiCtx<'_>, action: CommandKind) -> Command {
    Command {
        v: PROTOCOL_VERSION,
        id: ctx.auth.next_command_id("api"),
        action,
        target: Target::All,
        args: Args::default(),
        sig: None,
    }
}

/// Turn an ack into a response: a refusal is a client error, not a 200.
fn reply_response(reply: Reply) -> ApiResponse {
    let status = if reply.ok { 200 } else { 400 };
    match serde_json::to_value(&reply) {
        Ok(v) => ApiResponse::json(status, v),
        Err(e) => ApiResponse::error(500, e.to_string()),
    }
}

// ---------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------

/// `GET /id`: the unauthenticated identity endpoint (ADR component 8).
/// Every call also mints the single-use recovery nonce `/recover` wants,
/// and reports whether first setup is still pending so the setup page
/// can show the right thing before anybody is logged in.
fn identity_response(ctx: &mut ApiCtx<'_>) -> ApiResponse {
    let nonce = ctx.auth.new_nonce(ctx.rng, ctx.now_ms);
    let password_set = ctx.password_set();
    let ota = ctx.platform.ota();
    ApiResponse::ok(json!({
        "device": ctx.identity.device_id(),
        "mac": ctx.identity.mac(),
        "fw": ctx.platform.fw_version(),
        "cert_sha256": ctx.identity.cert_sha256(),
        "nonce": nonce,
        "password_set": password_set,
        "uptime_s": ctx.platform.uptime_s(),
        "ota": {"running": ota.running, "state": ota.state, "pending_verify": ota.pending_verify},
        "modbus_map_version": crate::modbus_map::MAP_VERSION,
        "v": PROTOCOL_VERSION,
    }))
}

/// `POST /recover`: factory reset without the admin password, either
/// with the per-device token or with a fleet-key signature over
/// `device || nonce || "factory-reset"`. One attempt per minute, logged
/// either way (ADR component 13).
fn recover(req: &ApiRequest, ctx: &mut ApiCtx<'_>) -> ApiResponse {
    if !ctx.auth.recover_allowed(ctx.now_ms) {
        return ApiResponse::error(429, "one recovery attempt per minute");
    }
    ctx.auth.note_recover(ctx.now_ms);

    let body = match req.json() {
        Ok(v) => v,
        Err(e) => return ApiResponse::error(400, e),
    };
    let device_id = ctx.identity.device_id();
    let peer = req.peer.clone();

    let accepted = if let Some(token) = body.get("token").and_then(Value::as_str) {
        ctx.identity.verify_recovery_token(token.trim())
    } else if let (Some(device), Some(nonce), Some(sig)) = (
        body.get("device").and_then(Value::as_str),
        body.get("nonce").and_then(Value::as_str),
        body.get("sig").and_then(Value::as_str),
    ) {
        if device != device_id {
            false
        } else if !ctx.auth.take_nonce(nonce, ctx.now_ms) {
            ctx.platform.event(EventKind::Security {
                what: String::from("recover_failed"),
                detail: Some(String::from("stale or unknown nonce")),
                peer: Some(peer.clone()),
            });
            return ApiResponse::error(403, "nonce is stale; fetch a new one from /id");
        } else {
            let mut message = Vec::new();
            message.extend_from_slice(device.as_bytes());
            message.extend_from_slice(nonce.as_bytes());
            message.extend_from_slice(RECOVER_PURPOSE);
            ctx.identity.verify_fleet_sig(&message, sig)
        }
    } else {
        return ApiResponse::error(
            400,
            "send {\"token\":\"...\"} or {\"device\":..,\"nonce\":..,\"sig\":..}",
        );
    };

    if !accepted {
        ctx.platform.event(EventKind::Security {
            what: String::from("recover_failed"),
            detail: None,
            peer: Some(peer),
        });
        return ApiResponse::error(403, "recovery rejected");
    }

    ctx.platform.event(EventKind::Security {
        what: String::from("recover_accepted"),
        detail: Some(String::from("factory reset")),
        peer: Some(peer),
    });
    ctx.auth.close_all();
    let mut cmd = new_command(ctx, CommandKind::FactoryReset);
    cmd.args.confirm = Some(device_id);
    reply_response(ctx.commands.dispatch(&cmd))
}

/// `POST /api/v1/session`: password in, session cookie out.
fn login(req: &ApiRequest, ctx: &mut ApiCtx<'_>) -> ApiResponse {
    let body = match req.json() {
        Ok(v) => v,
        Err(e) => return ApiResponse::error(400, e),
    };
    let Some(password) = body.get("password").and_then(Value::as_str) else {
        return ApiResponse::error(400, "password is required");
    };
    let secrets = ctx.store.load_secrets();
    if secrets.admin_password_missing() {
        return ApiResponse::error(409, "no admin password is set yet");
    }
    if ctx.auth.is_locked(ctx.now_ms) {
        return ApiResponse::error(
            429,
            format!(
                "locked out for another {} s",
                ctx.auth.lockout_left_s(ctx.now_ms)
            ),
        );
    }
    let ok = verify_hash(
        password.as_bytes(),
        &secrets.admin_salt,
        &secrets.admin_hash,
        secrets.admin_iters,
    );
    if !ok {
        let max = ctx.config.sec.max_login_fails;
        let lockout = ctx.config.sec.lockout_s;
        let locked = ctx.auth.note_fail(ctx.now_ms, max, lockout);
        ctx.platform.event(EventKind::Security {
            what: String::from(if locked {
                "login_lockout"
            } else {
                "login_failed"
            }),
            detail: locked.then(|| format!("{lockout} s")),
            peer: Some(req.peer.clone()),
        });
        return ApiResponse::error(401, "bad password");
    }
    ctx.auth.note_success();
    let hours = ctx.config.sec.session_hours;
    let id = ctx.auth.open_session(ctx.rng, ctx.now_ms, hours);
    let cookie = format!(
        "{SESSION_COOKIE}={id}; Path=/; HttpOnly; Secure; SameSite=Strict; Max-Age={}",
        u32::from(if hours == 0 { 12 } else { hours }) * 3600
    );
    ctx.platform.event(EventKind::Security {
        what: String::from("login"),
        detail: None,
        peer: Some(req.peer.clone()),
    });
    ApiResponse::ok(json!({"ok": true, "session_hours": hours})).with_cookie(cookie)
}

/// `DELETE /api/v1/session`.
fn logout(req: &ApiRequest, ctx: &mut ApiCtx<'_>) -> ApiResponse {
    if let Auth::Session(id) = &req.auth {
        ctx.auth.close_session(id);
    }
    ApiResponse::ok_true().with_cookie(format!(
        "{SESSION_COOKIE}=; Path=/; HttpOnly; Secure; SameSite=Strict; Max-Age=0"
    ))
}

/// `GET /api/v1/session`: is this credential still good?
fn session_info(req: &ApiRequest, ctx: &mut ApiCtx<'_>) -> ApiResponse {
    let authed = ctx.is_authenticated(req);
    let password_set = ctx.password_set();
    ApiResponse::ok(json!({
        "ok": true,
        "authenticated": authed,
        "password_set": password_set,
        "locked_out": ctx.auth.is_locked(ctx.now_ms),
        "lockout_left_s": ctx.auth.lockout_left_s(ctx.now_ms),
    }))
}

/// `GET /api/v1/status`: the whole status page in one response.
fn status(ctx: &mut ApiCtx<'_>) -> ApiResponse {
    let state = State::from_observed(ctx.observed, &ctx.config.nodes, ctx.now_ms);
    let state_value = match serde_json::to_value(&state) {
        Ok(v) => v,
        Err(e) => return ApiResponse::error(500, e.to_string()),
    };
    let net = ctx.platform.net();
    let mqtt = ctx.platform.mqtt();
    let ota = ctx.platform.ota();
    let staged = ctx.net.staged();
    ApiResponse::ok(json!({
        "ok": true,
        "v": PROTOCOL_VERSION,
        "device": ctx.identity.device_id(),
        "mac": ctx.identity.mac(),
        "fw": ctx.platform.fw_version(),
        "cert_sha256": ctx.identity.cert_sha256(),
        "boot_reason": ctx.platform.boot_reason().as_str(),
        "uptime_s": ctx.platform.uptime_s(),
        "free_heap": ctx.platform.free_heap(),
        "state": state_value,
        "net": net,
        "mqtt": mqtt,
        "ota": ota,
        "staged": staged,
        "modbus": ctx.config.sec.modbus,
    }))
}

/// `GET /api/v1/nodes`: names, order, policy and live state together,
/// which is what the Nodes page needs.
fn nodes(ctx: &mut ApiCtx<'_>) -> ApiResponse {
    let state = State::from_observed(ctx.observed, &ctx.config.nodes, ctx.now_ms);
    let settings = match serde_json::to_value(&ctx.config.nodes) {
        Ok(v) => v,
        Err(e) => return ApiResponse::error(500, e.to_string()),
    };
    let live = match serde_json::to_value(&state.nodes) {
        Ok(v) => v,
        Err(e) => return ApiResponse::error(500, e.to_string()),
    };
    ApiResponse::ok(json!({"ok": true, "settings": settings, "nodes": live}))
}

/// `POST /api/v1/nodes/<n|all>/<action>`: the same `Command` the MQTT
/// `cmd` topic takes, with the action and the target in the path and the
/// optional `args` object as the body.
fn node_action(
    node: Option<NodeId>,
    action: ActionKind,
    req: &ApiRequest,
    ctx: &mut ApiCtx<'_>,
) -> ApiResponse {
    let body = match req.json() {
        Ok(v) => v,
        Err(e) => return ApiResponse::error(400, e),
    };
    let args: Args = match body.get("args").cloned().unwrap_or(body) {
        Value::Null => Args::default(),
        v => match serde_json::from_value(v) {
            Ok(a) => a,
            Err(e) => return ApiResponse::error(400, format!("bad args: {e}")),
        },
    };
    let mut cmd = new_command(ctx, CommandKind::of_action(action));
    cmd.target = match node {
        Some(n) => Target::Node(n),
        None => Target::All,
    };
    cmd.args = args;
    reply_response(ctx.commands.dispatch(&cmd))
}

/// `POST /api/v1/cmd`: a raw `Command`, byte for byte what the MQTT
/// `cmd` topic accepts.
fn raw_command(req: &ApiRequest, ctx: &mut ApiCtx<'_>) -> ApiResponse {
    let text = match core::str::from_utf8(&req.body) {
        Ok(t) => t,
        Err(_) => return ApiResponse::error(400, "body is not UTF-8"),
    };
    let mut cmd = match Command::from_json(text) {
        Ok(c) => c,
        Err(e) => return ApiResponse::error(400, e),
    };
    if cmd.id.trim().is_empty() {
        cmd.id = ctx.auth.next_command_id("api");
    }
    reply_response(ctx.commands.dispatch(&cmd))
}

/// `GET /api/v1/config/export`: the whole document, secrets excluded by
/// construction ([`Config`] holds none).
fn export_config(ctx: &mut ApiCtx<'_>) -> ApiResponse {
    match ctx.config.export_json() {
        Ok(text) => json_text(200, text).with_header(
            "Content-Disposition",
            format!(
                "attachment; filename=\"{}-config.json\"",
                ctx.identity.device_id()
            ),
        ),
        Err(e) => ApiResponse::error(500, e.to_string()),
    }
}

/// `POST /api/v1/config/import` and `PUT /api/v1/config`.
fn import_config(req: &ApiRequest, ctx: &mut ApiCtx<'_>) -> ApiResponse {
    let text = match core::str::from_utf8(&req.body) {
        Ok(t) => t,
        Err(_) => return ApiResponse::error(400, "body is not UTF-8"),
    };
    let next = match Config::import_json(text) {
        Ok(c) => c,
        Err(e) => return ApiResponse::error(400, e.to_string()),
    };
    let mut saved: Vec<Section> = Vec::new();
    let mut staged: Vec<Section> = Vec::new();
    let mut confirm_s = 0u32;
    for section in Section::ALL {
        let before = ctx.config.section_json(section).ok();
        let after = next.section_json(section).ok();
        if before == after {
            continue;
        }
        let Some(json) = after else {
            continue;
        };
        if section.affects_reachability() {
            match ctx.net.stage_and_apply(section, &json) {
                Ok(secs) => {
                    confirm_s = confirm_s.max(secs);
                    staged.push(section);
                }
                Err(e) => return ApiResponse::error(500, e),
            }
        } else if let Err(e) = ctx.store.save_section(section, &json) {
            return ApiResponse::error(500, e);
        } else {
            saved.push(section);
        }
    }
    *ctx.config = next;
    ctx.platform.event(EventKind::Config {
        section: None,
        change: String::from("imported"),
        detail: None,
    });
    ApiResponse::ok(json!({
        "ok": true,
        "saved": saved,
        "staged": staged,
        "confirm_s": confirm_s,
    }))
}

/// `PUT /api/v1/config/<section>`.
fn put_section_body(section: Section, req: &ApiRequest, ctx: &mut ApiCtx<'_>) -> ApiResponse {
    let text = match core::str::from_utf8(&req.body) {
        Ok(t) => t,
        Err(_) => return ApiResponse::error(400, "body is not UTF-8"),
    };
    put_section(section, text, ctx)
}

/// `PUT /api/v1/rules`: accepts the `rules` section or a bare array.
fn put_rules(req: &ApiRequest, ctx: &mut ApiCtx<'_>) -> ApiResponse {
    let body = match req.json() {
        Ok(v) => v,
        Err(e) => return ApiResponse::error(400, e),
    };
    let section_value = match body {
        Value::Array(items) => {
            if let Err(e) = serde_json::from_value::<Vec<Rule>>(Value::Array(items.clone())) {
                return ApiResponse::error(400, format!("bad rule: {e}"));
            }
            json!({"rules": items})
        }
        other => other,
    };
    match serde_json::to_string(&section_value) {
        Ok(text) => put_section(Section::Rules, &text, ctx),
        Err(e) => ApiResponse::error(500, e.to_string()),
    }
}

/// Validate one section, then either stage it (when getting it wrong can
/// cut the session) or write it straight through.
fn put_section(section: Section, json_text: &str, ctx: &mut ApiCtx<'_>) -> ApiResponse {
    let mut probe = ctx.config.clone();
    if let Err(e) = probe.set_section_json(section, json_text) {
        return ApiResponse::error(400, e.to_string());
    }
    if let Err(errs) = probe.validate() {
        return ApiResponse::error(400, errs.join("; "));
    }
    let normalised = match probe.section_json(section) {
        Ok(t) => t,
        Err(e) => return ApiResponse::error(500, e.to_string()),
    };
    if section.affects_reachability() {
        match ctx.net.stage_and_apply(section, &normalised) {
            Ok(confirm_s) => {
                *ctx.config = probe;
                ctx.platform.event(EventKind::Config {
                    section: Some(section),
                    change: String::from("staged"),
                    detail: Some(format!("confirm within {confirm_s} s")),
                });
                ApiResponse::ok(json!({
                    "ok": true,
                    "section": section,
                    "staged": true,
                    "confirm_s": confirm_s,
                }))
            }
            Err(e) => ApiResponse::error(500, e),
        }
    } else {
        match ctx.store.save_section(section, &normalised) {
            Ok(()) => {
                *ctx.config = probe;
                ctx.platform.event(EventKind::Config {
                    section: Some(section),
                    change: String::from("set"),
                    detail: None,
                });
                ApiResponse::ok(json!({"ok": true, "section": section, "staged": false}))
            }
            Err(e) => ApiResponse::error(500, e),
        }
    }
}

/// `POST /api/v1/firmware/upload`. Either the transport streamed the
/// body into the sink (and filled [`ApiRequest::upload`]), or the body is
/// small enough that it arrived buffered.
fn firmware_upload(req: &ApiRequest, ctx: &mut ApiCtx<'_>) -> ApiResponse {
    if let Some(outcome) = &req.upload {
        if let Some(err) = &outcome.error {
            ctx.ota.abort();
            ctx.platform.event(EventKind::Ota {
                phase: String::from("failed"),
                progress: None,
                detail: Some(err.clone()),
            });
            return ApiResponse::error(500, err.clone());
        }
        return finish_ota(ctx);
    }
    if req.body.is_empty() {
        return ApiResponse::error(400, "empty image");
    }
    if let Err(e) = ctx.ota.begin(Some(req.body.len() as u64)) {
        return ApiResponse::error(500, e);
    }
    if let Err(e) = ctx.ota.write(&req.body) {
        ctx.ota.abort();
        return ApiResponse::error(500, e);
    }
    finish_ota(ctx)
}

fn finish_ota(ctx: &mut ApiCtx<'_>) -> ApiResponse {
    match ctx.ota.finish() {
        Ok(done) => {
            ctx.platform.event(EventKind::Ota {
                phase: String::from("verified"),
                progress: Some(100),
                detail: Some(format!("{} bytes into {}", done.bytes, done.slot)),
            });
            match serde_json::to_value(&done) {
                Ok(mut v) => {
                    if let Value::Object(map) = &mut v {
                        map.insert(String::from("ok"), Value::Bool(true));
                    }
                    ApiResponse::ok(v)
                }
                Err(e) => ApiResponse::error(500, e.to_string()),
            }
        }
        Err(e) => {
            ctx.platform.event(EventKind::Ota {
                phase: String::from("failed"),
                progress: None,
                detail: Some(e.clone()),
            });
            ApiResponse::error(400, e)
        }
    }
}

/// `POST /api/v1/factory-reset`, body `{"confirm":"<device id>"}`. The
/// check itself lives in the dispatcher, so HTTP and MQTT agree.
fn factory_reset(req: &ApiRequest, ctx: &mut ApiCtx<'_>) -> ApiResponse {
    let body = match req.json() {
        Ok(v) => v,
        Err(e) => return ApiResponse::error(400, e),
    };
    let confirm = body
        .get("confirm")
        .and_then(Value::as_str)
        .map(String::from);
    let mut cmd = new_command(ctx, CommandKind::FactoryReset);
    cmd.args.confirm = confirm;
    let reply = ctx.commands.dispatch(&cmd);
    if reply.ok {
        ctx.auth.close_all();
    }
    reply_response(reply)
}

/// `POST /api/v1/security/password`. The one route that is public while
/// no password exists; afterwards it needs the session plus, when the
/// client sends one, the current password.
fn set_password(req: &ApiRequest, ctx: &mut ApiCtx<'_>) -> ApiResponse {
    let body = match req.json() {
        Ok(v) => v,
        Err(e) => return ApiResponse::error(400, e),
    };
    let Some(password) = body.get("password").and_then(Value::as_str) else {
        return ApiResponse::error(400, "password is required");
    };
    if password.len() < 8 {
        return ApiResponse::error(400, "password must be at least 8 characters");
    }
    let mut secrets = ctx.store.load_secrets();
    let first_boot = secrets.admin_password_missing();
    if !first_boot
        && let Some(current) = body.get("current").and_then(Value::as_str)
        && !verify_hash(
            current.as_bytes(),
            &secrets.admin_salt,
            &secrets.admin_hash,
            secrets.admin_iters,
        )
    {
        return ApiResponse::error(403, "current password does not match");
    }

    // The salt is minted once and then kept: API tokens are hashed with
    // it too, and rotating it would invalidate every token.
    if secrets.admin_salt.is_empty() {
        let mut salt = [0u8; SALT_BYTES];
        ctx.rng.fill(&mut salt);
        secrets.admin_salt = hex(&salt);
    }
    let Some(salt) = unhex(&secrets.admin_salt) else {
        return ApiResponse::error(500, "stored salt is corrupt");
    };
    secrets.admin_iters = PBKDF2_ITERS;
    secrets.admin_hash = pbkdf2_hex(password.as_bytes(), &salt, PBKDF2_ITERS);
    if let Err(e) = ctx.store.save_secrets(&secrets) {
        return ApiResponse::error(500, e);
    }

    ctx.config.sec.admin_password_set = true;
    if let Ok(json) = ctx.config.section_json(Section::Sec) {
        // First setup must not need a commit-confirm round trip, so the
        // `sec` section is written straight through here.
        if let Err(e) = ctx.store.save_section(Section::Sec, &json) {
            return ApiResponse::error(500, e);
        }
    }
    ctx.auth.close_all();
    ctx.platform.event(EventKind::Security {
        what: String::from(if first_boot {
            "password_set"
        } else {
            "password_changed"
        }),
        detail: None,
        peer: Some(req.peer.clone()),
    });

    let token = if first_boot {
        ctx.identity.recovery_token_once()
    } else {
        None
    };
    ApiResponse::ok(json!({
        "ok": true,
        "first_boot": first_boot,
        "recovery_token": token,
    }))
}

/// `GET /api/v1/security/tokens`: names and creation times only.
fn list_tokens(ctx: &mut ApiCtx<'_>) -> ApiResponse {
    let secrets = ctx.store.load_secrets();
    let tokens: Vec<Value> = secrets
        .api_tokens
        .iter()
        .map(|t| json!({"name": t.name, "created_s": t.created_s}))
        .collect();
    ApiResponse::ok(json!({"ok": true, "tokens": tokens}))
}

/// `POST /api/v1/security/tokens`: returns the token once, stores it
/// hashed.
fn create_token(req: &ApiRequest, ctx: &mut ApiCtx<'_>) -> ApiResponse {
    let body = match req.json() {
        Ok(v) => v,
        Err(e) => return ApiResponse::error(400, e),
    };
    let name = body
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if name.is_empty() {
        return ApiResponse::error(400, "name is required");
    }
    let mut secrets = ctx.store.load_secrets();
    if secrets.api_tokens.iter().any(|t| t.name == name) {
        return ApiResponse::error(409, "a token with that name exists");
    }
    let Some(salt) = unhex(&secrets.admin_salt) else {
        return ApiResponse::error(500, "no device salt; set the admin password first");
    };
    let mut raw = [0u8; TOKEN_BYTES];
    ctx.rng.fill(&mut raw);
    let token = hex(&raw);
    secrets.api_tokens.push(ApiToken {
        name: name.clone(),
        hash: pbkdf2_hex(token.as_bytes(), &salt, secrets.admin_iters),
        created_s: u64::from(ctx.platform.uptime_s()),
    });
    if let Err(e) = ctx.store.save_secrets(&secrets) {
        return ApiResponse::error(500, e);
    }
    ctx.platform.event(EventKind::Security {
        what: String::from("token_created"),
        detail: Some(name.clone()),
        peer: Some(req.peer.clone()),
    });
    ApiResponse::ok(json!({"ok": true, "name": name, "token": token}))
}

/// `DELETE /api/v1/security/tokens/<name>`.
fn delete_token(name: &str, ctx: &mut ApiCtx<'_>) -> ApiResponse {
    let mut secrets = ctx.store.load_secrets();
    let before = secrets.api_tokens.len();
    secrets.api_tokens.retain(|t| t.name != name);
    if secrets.api_tokens.len() == before {
        return ApiResponse::error(404, "no such token");
    }
    if let Err(e) = ctx.store.save_secrets(&secrets) {
        return ApiResponse::error(500, e);
    }
    ctx.platform.event(EventKind::Security {
        what: String::from("token_deleted"),
        detail: Some(String::from(name)),
        peer: None,
    });
    ApiResponse::ok_true()
}

/// `PUT /api/v1/security/fleet-key`, body `{"pem":"..."}`.
fn set_fleet_key(req: &ApiRequest, ctx: &mut ApiCtx<'_>) -> ApiResponse {
    let body = match req.json() {
        Ok(v) => v,
        Err(e) => return ApiResponse::error(400, e),
    };
    let pem = body
        .get("pem")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if let Err(e) = ctx.identity.set_fleet_pubkey(&pem) {
        return ApiResponse::error(400, e);
    }
    ctx.config.sec.fleet_recovery_pubkey_pem = pem;
    let json = match ctx.config.section_json(Section::Sec) {
        Ok(t) => t,
        Err(e) => return ApiResponse::error(500, e.to_string()),
    };
    if let Err(e) = ctx.store.save_section(Section::Sec, &json) {
        return ApiResponse::error(500, e);
    }
    ctx.platform.event(EventKind::Security {
        what: String::from("fleet_key_set"),
        detail: None,
        peer: Some(req.peer.clone()),
    });
    ApiResponse::ok_true()
}

/// `PUT /api/v1/security/cert`, body `{"cert_pem":"..","key_pem":".."}`.
fn set_cert(req: &ApiRequest, ctx: &mut ApiCtx<'_>) -> ApiResponse {
    let body = match req.json() {
        Ok(v) => v,
        Err(e) => return ApiResponse::error(400, e),
    };
    let cert = body.get("cert_pem").and_then(Value::as_str).unwrap_or("");
    let key = body.get("key_pem").and_then(Value::as_str).unwrap_or("");
    if cert.trim().is_empty() || key.trim().is_empty() {
        return ApiResponse::error(400, "cert_pem and key_pem are required");
    }
    if let Err(e) = ctx.identity.set_device_cert(cert, key) {
        return ApiResponse::error(400, e);
    }
    ctx.config.sec.device_cert_pem = String::from(cert);
    let json = match ctx.config.section_json(Section::Sec) {
        Ok(t) => t,
        Err(e) => return ApiResponse::error(500, e.to_string()),
    };
    if let Err(e) = ctx.store.save_section(Section::Sec, &json) {
        return ApiResponse::error(500, e);
    }
    ctx.platform.event(EventKind::Security {
        what: String::from("device_cert_set"),
        detail: Some(ctx.identity.cert_sha256()),
        peer: Some(req.peer.clone()),
    });
    ApiResponse::ok(json!({"ok": true, "cert_sha256": ctx.identity.cert_sha256()}))
}

/// `PUT /api/v1/security/modbus-allowlist`, body is the `modbus` object
/// of the `sec` section.
fn set_modbus_allowlist(req: &ApiRequest, ctx: &mut ApiCtx<'_>) -> ApiResponse {
    let body = match req.json() {
        Ok(v) => v,
        Err(e) => return ApiResponse::error(400, e),
    };
    let modbus = match serde_json::from_value(body) {
        Ok(m) => m,
        Err(e) => return ApiResponse::error(400, format!("bad modbus settings: {e}")),
    };
    let mut probe = ctx.config.clone();
    probe.sec.modbus = modbus;
    let json = match probe.section_json(Section::Sec) {
        Ok(t) => t,
        Err(e) => return ApiResponse::error(500, e.to_string()),
    };
    put_section(Section::Sec, &json, ctx)
}

/// `PUT /api/v1/security/mqtt-credentials`, body
/// `{"username":..,"password":..,"client_cert_pem":..,"client_key_pem":..}`.
///
/// Every field is optional: a field that is absent is left as it is, a
/// field that is present and empty clears the stored value. The password
/// and the client key are secrets and never come back out, so there is no
/// `GET` here and the answer carries no body (204).
///
/// The username is public material and lives in the `mqtt` config
/// section; it is written straight through rather than staged, like the
/// fleet key and the device certificate, because a credential change
/// cannot cut the HTTP session. The MQTT worker reads both on its next
/// connection attempt.
fn set_mqtt_credentials(req: &ApiRequest, ctx: &mut ApiCtx<'_>) -> ApiResponse {
    let body = match req.json() {
        Ok(v) => v,
        Err(e) => return ApiResponse::error(400, e),
    };
    let Some(fields) = body.as_object() else {
        return ApiResponse::error(400, "the body must be a JSON object");
    };
    let field = |name: &str| -> Result<Option<String>, ApiResponse> {
        match fields.get(name) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(s)) => Ok(Some(s.clone())),
            Some(_) => Err(ApiResponse::error(400, format!("{name} must be a string"))),
        }
    };
    let username = match field("username") {
        Ok(v) => v,
        Err(r) => return r,
    };
    let password = match field("password") {
        Ok(v) => v,
        Err(r) => return r,
    };
    let cert_pem = match field("client_cert_pem") {
        Ok(v) => v,
        Err(r) => return r,
    };
    let key_pem = match field("client_key_pem") {
        Ok(v) => v,
        Err(r) => return r,
    };
    if username.is_none() && password.is_none() && cert_pem.is_none() && key_pem.is_none() {
        return ApiResponse::error(
            400,
            "one of username, password, client_cert_pem, client_key_pem is required",
        );
    }
    // A certificate without its key (or the other way round) is a
    // configuration the broker can only reject, so refuse it here.
    let pair_ok = match (&cert_pem, &key_pem) {
        (Some(c), None) => c.trim().is_empty() || !secrets_key_missing(ctx),
        (None, Some(k)) => k.trim().is_empty() || !secrets_cert_missing(ctx),
        _ => true,
    };
    if !pair_ok {
        return ApiResponse::error(
            400,
            "client_cert_pem and client_key_pem have to be set together",
        );
    }

    let mut secrets = ctx.store.load_secrets();
    let mut changed: Vec<&str> = Vec::new();
    if let Some(v) = password {
        secrets.mqtt_password = v;
        changed.push("password");
    }
    if let Some(v) = cert_pem {
        secrets.mqtt_client_cert_pem = v;
        changed.push("client_cert_pem");
    }
    if let Some(v) = key_pem {
        secrets.mqtt_client_key_pem = v;
        changed.push("client_key_pem");
    }
    if !changed.is_empty()
        && let Err(e) = ctx.store.save_secrets(&secrets)
    {
        return ApiResponse::error(500, e);
    }
    if let Some(v) = username {
        ctx.config.mqtt.username = v;
        changed.push("username");
        let json = match ctx.config.section_json(Section::Mqtt) {
            Ok(t) => t,
            Err(e) => return ApiResponse::error(500, e.to_string()),
        };
        if let Err(e) = ctx.store.save_section(Section::Mqtt, &json) {
            return ApiResponse::error(500, e);
        }
    }
    ctx.platform.event(EventKind::Security {
        what: String::from("mqtt_credentials_set"),
        detail: Some(changed.join(", ")),
        peer: Some(req.peer.clone()),
    });
    ApiResponse::no_content()
}

/// True when no client key is stored, so a lone certificate would leave
/// the pair incomplete.
fn secrets_key_missing(ctx: &mut ApiCtx<'_>) -> bool {
    ctx.store
        .load_secrets()
        .mqtt_client_key_pem
        .trim()
        .is_empty()
}

/// True when no client certificate is stored.
fn secrets_cert_missing(ctx: &mut ApiCtx<'_>) -> bool {
    ctx.store
        .load_secrets()
        .mqtt_client_cert_pem
        .trim()
        .is_empty()
}

/// `GET /api/v1/log/tail?lines=N`.
fn log_tail(req: &ApiRequest, ctx: &mut ApiCtx<'_>) -> ApiResponse {
    let lines = req
        .param("lines")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(100)
        .clamp(1, LOG_TAIL_MAX);
    let mut out = String::new();
    for line in ctx.platform.log_tail(lines) {
        out.push_str(&line);
        out.push('\n');
    }
    ApiResponse::text(200, out)
}
