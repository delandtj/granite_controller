//! The HTTP route table: route matching, the auth gate per route, the
//! first-boot password rule, the login lockout, commit-confirm on a net
//! section, and the JSON shapes clients depend on.

mod common;

use std::collections::BTreeMap;

use granite_core::actuator::{ActionKind, Actuator};
use granite_core::api::{
    self, Access, ApiCtx, ApiRequest, Auth, AuthState, Body, Dispatcher, Identity, Method,
    MqttStatus, NetControl, NetStatus, OtaFinish, OtaSink, OtaStatus, Platform, Rng, Route,
    SlotInfo, StagedInfo, Store, StreamKind,
};
use granite_core::config::{Config, Secrets, Section};
use granite_core::dispatch::{DispatchCtx, SideEffect, dispatch};
use granite_core::hal::BootReason;
use granite_core::msg::{Command, EventKind, Reply};
use granite_core::observed::Observed;
use granite_core::rules::RuleEngine;

use common::FakeSwitches;

const DEVICE: &str = "granite-a1b2c3";

// ---------------------------------------------------------------------
// Fakes
// ---------------------------------------------------------------------

#[derive(Default)]
struct FakeStore {
    secrets: Secrets,
    sections: BTreeMap<Section, String>,
    saves: Vec<Section>,
    fail: bool,
}

impl Store for FakeStore {
    fn load_secrets(&mut self) -> Secrets {
        self.secrets.clone()
    }

    fn save_secrets(&mut self, secrets: &Secrets) -> Result<(), String> {
        if self.fail {
            return Err(String::from("storage error"));
        }
        self.secrets = secrets.clone();
        Ok(())
    }

    fn save_section(&mut self, section: Section, json: &str) -> Result<(), String> {
        if self.fail {
            return Err(String::from("storage error"));
        }
        self.sections.insert(section, String::from(json));
        self.saves.push(section);
        Ok(())
    }
}

#[derive(Default)]
struct FakePlatform {
    events: Vec<EventKind>,
    log: Vec<String>,
    rolled_back: bool,
    marked_valid: bool,
}

impl Platform for FakePlatform {
    fn fw_version(&self) -> String {
        String::from("0.1.0-test")
    }
    fn uptime_s(&self) -> u32 {
        42
    }
    fn boot_reason(&self) -> BootReason {
        BootReason::PowerOn
    }
    fn free_heap(&self) -> u32 {
        123_456
    }
    fn net(&self) -> NetStatus {
        NetStatus {
            link_up: true,
            ip_mode: String::from("dhcp"),
            ip: String::from("192.168.1.10"),
            netmask: String::from("255.255.255.0"),
            gateway: String::from("192.168.1.1"),
            dns: vec![String::from("192.168.1.1")],
            hostname: String::from(DEVICE),
            dhcp_fallback: false,
            sntp_synced: true,
        }
    }
    fn mqtt(&self) -> MqttStatus {
        MqttStatus::default()
    }
    fn ota(&self) -> OtaStatus {
        OtaStatus {
            running: String::from("ota_0"),
            state: String::from("valid"),
            pending_verify: false,
            validate_left_s: None,
            slots: vec![SlotInfo {
                label: String::from("ota_0"),
                state: String::from("running"),
                version: String::from("0.1.0-test"),
                size: 2_621_440,
            }],
            key_id: String::from("deadbeef"),
            rollback_available: true,
        }
    }
    fn log_tail(&self, lines: usize) -> Vec<String> {
        self.log.iter().rev().take(lines).rev().cloned().collect()
    }
    fn ota_rollback(&mut self) -> Result<(), String> {
        self.rolled_back = true;
        Ok(())
    }
    fn ota_mark_valid(&mut self) -> Result<(), String> {
        self.marked_valid = true;
        Ok(())
    }
    fn event(&mut self, kind: EventKind) {
        self.events.push(kind);
    }
}

impl FakePlatform {
    fn security_events(&self) -> Vec<String> {
        self.events
            .iter()
            .filter_map(|e| match e {
                EventKind::Security { what, .. } => Some(what.clone()),
                _ => None,
            })
            .collect()
    }
}

struct FakeIdentity {
    token: String,
    token_shown: bool,
    fleet_pem: String,
    cert_sha: String,
    good_sig: String,
}

impl Default for FakeIdentity {
    fn default() -> Self {
        FakeIdentity {
            token: String::from("AAAA-BBBB-CCCC-DDDD"),
            token_shown: false,
            fleet_pem: String::new(),
            cert_sha: "aa".repeat(32),
            good_sig: String::from("good-signature"),
        }
    }
}

impl Identity for FakeIdentity {
    fn device_id(&self) -> String {
        String::from(DEVICE)
    }
    fn mac(&self) -> String {
        String::from("aa:bb:cc:a1:b2:c3")
    }
    fn cert_sha256(&self) -> String {
        self.cert_sha.clone()
    }
    fn recovery_token_once(&mut self) -> Option<String> {
        if self.token_shown {
            None
        } else {
            self.token_shown = true;
            Some(self.token.clone())
        }
    }
    fn verify_recovery_token(&mut self, token: &str) -> bool {
        token == self.token
    }
    fn verify_fleet_sig(&self, message: &[u8], sig: &str) -> bool {
        !self.fleet_pem.is_empty()
            && sig == self.good_sig
            && message.ends_with(api::RECOVER_PURPOSE)
    }
    fn set_fleet_pubkey(&mut self, pem: &str) -> Result<(), String> {
        if !pem.contains("PUBLIC KEY") {
            return Err(String::from("not a PEM public key"));
        }
        self.fleet_pem = String::from(pem);
        Ok(())
    }
    fn set_device_cert(&mut self, cert_pem: &str, key_pem: &str) -> Result<(), String> {
        if !cert_pem.contains("CERTIFICATE") || !key_pem.contains("PRIVATE KEY") {
            return Err(String::from("not a PEM pair"));
        }
        self.cert_sha = "bb".repeat(32);
        Ok(())
    }
}

#[derive(Default)]
struct FakeNet {
    staged: Vec<(Section, String)>,
    confirmed: usize,
    reverted: usize,
}

impl NetControl for FakeNet {
    fn stage_and_apply(&mut self, section: Section, json: &str) -> Result<u32, String> {
        self.staged.push((section, String::from(json)));
        Ok(300)
    }
    fn confirm(&mut self) -> bool {
        if self.staged.is_empty() {
            return false;
        }
        self.staged.clear();
        self.confirmed += 1;
        true
    }
    fn revert(&mut self) -> bool {
        if self.staged.is_empty() {
            return false;
        }
        self.staged.clear();
        self.reverted += 1;
        true
    }
    fn staged(&self) -> Option<StagedInfo> {
        if self.staged.is_empty() {
            return None;
        }
        Some(StagedInfo {
            sections: self.staged.iter().map(|(s, _)| *s).collect(),
            seconds_left: 300,
        })
    }
}

#[derive(Default)]
struct FakeOta {
    written: Vec<u8>,
    begun: bool,
    aborted: bool,
}

impl OtaSink for FakeOta {
    fn begin(&mut self, _total_len: Option<u64>) -> Result<(), String> {
        self.begun = true;
        self.written.clear();
        Ok(())
    }
    fn write(&mut self, chunk: &[u8]) -> Result<(), String> {
        self.written.extend_from_slice(chunk);
        Ok(())
    }
    fn finish(&mut self) -> Result<OtaFinish, String> {
        if self.written.len() < 4 {
            return Err(String::from("image too short"));
        }
        Ok(OtaFinish {
            slot: String::from("ota_1"),
            bytes: self.written.len() as u64,
            reboot_required: true,
        })
    }
    fn abort(&mut self) {
        self.aborted = true;
        self.written.clear();
    }
}

/// Deterministic randomness: the tests need reproducible session ids.
#[derive(Default)]
struct SeqRng(u8);

impl Rng for SeqRng {
    fn fill(&mut self, out: &mut [u8]) {
        for b in out.iter_mut() {
            self.0 = self.0.wrapping_add(7);
            *b = self.0;
        }
    }
}

/// The real dispatcher over a fake expander, so a node action goes all
/// the way into the actuator queue.
struct CoreDispatcher {
    actuator: Actuator<FakeSwitches>,
    rules: RuleEngine,
    config: Config,
    observed: Observed,
    effects: Vec<SideEffect>,
    seen: Vec<Command>,
}

impl CoreDispatcher {
    fn new() -> Self {
        let config = Config::default();
        // Every node sensed off, so state-dependent actions are not
        // refused for `unknown_state` in these tests.
        let mut observed = Observed::new();
        for node in observed.nodes.iter_mut() {
            node.led = Some(false);
            node.state = granite_core::node::NodeState::Off;
            node.ts_ms = 1;
        }
        CoreDispatcher {
            actuator: Actuator::new(FakeSwitches::default()),
            rules: RuleEngine::with_rules(config.rules.rules.clone()),
            config,
            observed,
            effects: Vec::new(),
            seen: Vec::new(),
        }
    }
}

impl Dispatcher for CoreDispatcher {
    fn dispatch(&mut self, cmd: &Command) -> Reply {
        self.seen.push(cmd.clone());
        let mut ctx = DispatchCtx {
            now_ms: 1_000,
            device_id: DEVICE,
            observed: &self.observed,
            actuator: &mut self.actuator,
            rules: &mut self.rules,
            config: &mut self.config,
            verify_sig: None,
        };
        let out = dispatch(cmd, &mut ctx);
        self.effects.extend(out.effects);
        out.reply
    }
}

/// Everything a request needs, in one owned bundle.
struct Rig {
    now_ms: u64,
    config: Config,
    observed: Observed,
    auth: AuthState,
    store: FakeStore,
    platform: FakePlatform,
    identity: FakeIdentity,
    net: FakeNet,
    ota: FakeOta,
    rng: SeqRng,
    commands: CoreDispatcher,
}

impl Rig {
    fn new() -> Self {
        Rig {
            now_ms: 10_000,
            config: Config::default(),
            observed: Observed::new(),
            auth: AuthState::new(),
            store: FakeStore::default(),
            platform: FakePlatform::default(),
            identity: FakeIdentity::default(),
            net: FakeNet::default(),
            ota: FakeOta::default(),
            rng: SeqRng::default(),
            commands: CoreDispatcher::new(),
        }
    }

    fn go(&mut self, req: ApiRequest) -> api::ApiResponse {
        let mut ctx = ApiCtx {
            now_ms: self.now_ms,
            config: &mut self.config,
            observed: &self.observed,
            auth: &mut self.auth,
            store: &mut self.store,
            platform: &mut self.platform,
            identity: &mut self.identity,
            net: &mut self.net,
            ota: &mut self.ota,
            rng: &mut self.rng,
            commands: &mut self.commands,
        };
        api::handle(req, &mut ctx)
    }

    /// Run first setup and log in; returns the session cookie value.
    fn setup(&mut self) -> String {
        let r = self.go(ApiRequest::new(Method::Post, "/api/v1/security/password")
            .with_body(br#"{"password":"correct horse"}"#));
        assert_eq!(r.status, 200, "{:?}", body_text(&r));
        let r = self.go(ApiRequest::new(Method::Post, "/api/v1/session")
            .with_body(br#"{"password":"correct horse"}"#));
        assert_eq!(r.status, 200, "{:?}", body_text(&r));
        let cookie = r.set_cookie.clone().expect("a session cookie");
        cookie
            .split(';')
            .next()
            .unwrap()
            .split_once('=')
            .unwrap()
            .1
            .to_string()
    }
}

fn body_text(r: &api::ApiResponse) -> String {
    match &r.body {
        Body::Bytes(b) => String::from_utf8_lossy(b).into_owned(),
        Body::Empty => String::new(),
        Body::Stream(k) => format!("{k:?}"),
    }
}

fn body_json(r: &api::ApiResponse) -> serde_json::Value {
    serde_json::from_str(&body_text(r)).expect("a JSON body")
}

// ---------------------------------------------------------------------
// Route matching
// ---------------------------------------------------------------------

#[test]
fn routes_match_the_table() {
    assert_eq!(api::route_of("/id"), Some(Route::Id));
    assert_eq!(api::route_of("/recover"), Some(Route::Recover));
    assert_eq!(api::route_of("/api/v1/session"), Some(Route::Session));
    assert_eq!(api::route_of("/api/v1/status"), Some(Route::Status));
    assert_eq!(api::route_of("/api/v1/state/"), Some(Route::State));
    assert_eq!(
        api::route_of("/api/v1/nodes/3/force_off"),
        Some(Route::NodeAction {
            node: Some(3),
            action: ActionKind::ForceOff
        })
    );
    assert_eq!(
        api::route_of("/api/v1/nodes/all/on_all"),
        Some(Route::NodeAction {
            node: None,
            action: ActionKind::OnAll
        })
    );
    assert_eq!(
        api::route_of("/api/v1/config/net"),
        Some(Route::ConfigSection(Section::Net))
    );
    assert_eq!(
        api::route_of("/api/v1/config/export"),
        Some(Route::ConfigExport)
    );
    assert_eq!(
        api::route_of("/api/v1/rules/7/ack"),
        Some(Route::RuleAck(7))
    );
    assert_eq!(
        api::route_of("/api/v1/firmware/mark-valid"),
        Some(Route::FirmwareMarkValid)
    );
    assert_eq!(
        api::route_of("/api/v1/security/tokens/deploy"),
        Some(Route::Token(String::from("deploy")))
    );
    assert_eq!(api::route_of("/api/v1/log/tail"), Some(Route::LogTail));
    assert_eq!(
        api::route_of("/api/v1/security/mqtt-credentials"),
        Some(Route::MqttCredentials)
    );

    // Unknown API paths are 404, not assets.
    assert_eq!(api::route_of("/api/v1/nope"), None);
    assert_eq!(api::route_of("/api/v2/status"), None);
    assert_eq!(api::route_of("/api/v1/nodes/9/on"), None);
    assert_eq!(api::route_of("/api/v1/nodes/3/explode"), None);
    assert_eq!(api::route_of("/api/v1/config/nosuch"), None);

    // Everything else is an asset, and traversal is stripped.
    assert_eq!(
        api::route_of("/"),
        Some(Route::Asset(String::from("/index.html")))
    );
    assert_eq!(
        api::route_of("/app.js"),
        Some(Route::Asset(String::from("/app.js")))
    );
    assert_eq!(
        api::route_of("/../../etc/passwd"),
        Some(Route::Asset(String::from("/etc/passwd")))
    );
}

#[test]
fn only_identity_recovery_login_and_assets_are_public() {
    for path in ["/id", "/recover", "/api/v1/session", "/style.css"] {
        let route = api::route_of(path).unwrap();
        assert_eq!(access_name(&route), "public", "{path}");
    }
    assert_eq!(
        access_of_path("/api/v1/security/password"),
        Access::PasswordSetup
    );
    for path in [
        "/api/v1/status",
        "/api/v1/state",
        "/api/v1/cmd",
        "/api/v1/nodes/1/on",
        "/api/v1/config/net",
        "/api/v1/config/export",
        "/api/v1/rules",
        "/api/v1/firmware/upload",
        "/api/v1/reboot",
        "/api/v1/factory-reset",
        "/api/v1/security/tokens",
        "/api/v1/security/fleet-key",
        "/api/v1/security/cert",
        "/api/v1/security/modbus-allowlist",
        "/api/v1/security/mqtt-credentials",
        "/api/v1/log/tail",
    ] {
        assert_eq!(access_of_path(path), Access::Authed, "{path}");
    }
}

fn access_of_path(path: &str) -> Access {
    api::access_of(&api::route_of(path).unwrap())
}

fn access_name(route: &Route) -> &'static str {
    match api::access_of(route) {
        Access::Public => "public",
        Access::PasswordSetup => "password-setup",
        Access::Authed => "authed",
    }
}

// ---------------------------------------------------------------------
// The auth gate
// ---------------------------------------------------------------------

#[test]
fn first_boot_refuses_everything_but_the_password() {
    let mut rig = Rig::new();

    // No password yet: the data routes say so with a 403, not a 401, so
    // the page can tell "set a password" from "log in".
    let r = rig.go(ApiRequest::new(Method::Get, "/api/v1/status"));
    assert_eq!(r.status, 403);
    assert!(body_text(&r).contains("no admin password"));

    // /id works and reports first setup.
    let r = rig.go(ApiRequest::new(Method::Get, "/id"));
    assert_eq!(r.status, 200);
    let id = body_json(&r);
    assert_eq!(id["device"], DEVICE);
    assert_eq!(id["password_set"], false);
    assert!(id["nonce"].as_str().unwrap().len() >= 32);
    assert_eq!(id["cert_sha256"].as_str().unwrap().len(), 64);

    // A short password is refused.
    let r = rig.go(ApiRequest::new(Method::Post, "/api/v1/security/password")
        .with_body(br#"{"password":"x"}"#));
    assert_eq!(r.status, 400);

    // Setting it returns the recovery token exactly once.
    let r = rig.go(ApiRequest::new(Method::Post, "/api/v1/security/password")
        .with_body(br#"{"password":"correct horse"}"#));
    assert_eq!(r.status, 200);
    let v = body_json(&r);
    assert_eq!(v["first_boot"], true);
    assert_eq!(v["recovery_token"], "AAAA-BBBB-CCCC-DDDD");
    assert!(rig.config.sec.admin_password_set);
    assert!(rig.store.saves.contains(&Section::Sec));
    assert!(!rig.store.secrets.admin_hash.is_empty());
    assert_eq!(rig.store.secrets.admin_iters, api::PBKDF2_ITERS);
    assert_eq!(rig.store.secrets.admin_salt.len(), api::SALT_BYTES * 2);

    // Now the same route needs authentication, and the token is gone.
    let r = rig.go(ApiRequest::new(Method::Post, "/api/v1/security/password")
        .with_body(br#"{"password":"another one"}"#));
    assert_eq!(r.status, 401);

    // And the data routes answer 401 instead of 403.
    let r = rig.go(ApiRequest::new(Method::Get, "/api/v1/status"));
    assert_eq!(r.status, 401);

    assert_eq!(rig.platform.security_events(), vec!["password_set"]);
}

#[test]
fn session_and_bearer_token_both_authenticate() {
    let mut rig = Rig::new();
    let session = rig.setup();

    let r = rig
        .go(ApiRequest::new(Method::Get, "/api/v1/status")
            .with_auth(Auth::Session(session.clone())));
    assert_eq!(r.status, 200);

    // A token is returned once and stored hashed.
    let r = rig.go(ApiRequest::new(Method::Post, "/api/v1/security/tokens")
        .with_auth(Auth::Session(session.clone()))
        .with_body(br#"{"name":"deploy"}"#));
    assert_eq!(r.status, 200);
    let token = body_json(&r)["token"].as_str().unwrap().to_string();
    assert_eq!(token.len(), api::TOKEN_BYTES * 2);
    assert!(rig.store.secrets.api_tokens.iter().all(|t| t.hash != token));

    let r = rig
        .go(ApiRequest::new(Method::Get, "/api/v1/state").with_auth(Auth::Bearer(token.clone())));
    assert_eq!(r.status, 200);

    let r = rig.go(ApiRequest::new(Method::Get, "/api/v1/state")
        .with_auth(Auth::Bearer(String::from("00") + &token[2..])));
    assert_eq!(r.status, 401);

    // The list never shows the secret.
    let r = rig.go(ApiRequest::new(Method::Get, "/api/v1/security/tokens")
        .with_auth(Auth::Session(session.clone())));
    let listed = body_text(&r);
    assert!(listed.contains("deploy"));
    assert!(!listed.contains(&token));

    // Deleting it takes the access away.
    let r = rig.go(
        ApiRequest::new(Method::Delete, "/api/v1/security/tokens/deploy")
            .with_auth(Auth::Session(session)),
    );
    assert_eq!(r.status, 200);
    let r = rig.go(ApiRequest::new(Method::Get, "/api/v1/state").with_auth(Auth::Bearer(token)));
    assert_eq!(r.status, 401);
}

#[test]
fn logout_drops_the_session() {
    let mut rig = Rig::new();
    let session = rig.setup();
    let r = rig.go(ApiRequest::new(Method::Delete, "/api/v1/session")
        .with_auth(Auth::Session(session.clone())));
    assert_eq!(r.status, 200);
    assert!(r.set_cookie.unwrap().contains("Max-Age=0"));
    let r =
        rig.go(ApiRequest::new(Method::Get, "/api/v1/status").with_auth(Auth::Session(session)));
    assert_eq!(r.status, 401);
}

#[test]
fn five_bad_logins_lock_the_login_out_for_a_minute() {
    let mut rig = Rig::new();
    let _ = rig.setup();
    assert_eq!(rig.config.sec.max_login_fails, 5);

    for attempt in 1..=4 {
        let r = rig
            .go(ApiRequest::new(Method::Post, "/api/v1/session")
                .with_body(br#"{"password":"wrong"}"#));
        assert_eq!(r.status, 401, "attempt {attempt}");
    }
    let r = rig
        .go(ApiRequest::new(Method::Post, "/api/v1/session").with_body(br#"{"password":"wrong"}"#));
    assert_eq!(r.status, 401);

    // Locked out now, even with the right password.
    let r = rig.go(ApiRequest::new(Method::Post, "/api/v1/session")
        .with_body(br#"{"password":"correct horse"}"#));
    assert_eq!(r.status, 429);
    assert!(body_text(&r).contains("locked out"));

    // The lockout expires after sec.lockout_s.
    rig.now_ms += u64::from(rig.config.sec.lockout_s) * 1000 + 1;
    let r = rig.go(ApiRequest::new(Method::Post, "/api/v1/session")
        .with_body(br#"{"password":"correct horse"}"#));
    assert_eq!(r.status, 200);

    let events = rig.platform.security_events();
    assert_eq!(events.iter().filter(|e| *e == "login_failed").count(), 4);
    assert_eq!(events.iter().filter(|e| *e == "login_lockout").count(), 1);
}

// ---------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------

#[test]
fn a_node_action_becomes_the_same_command_mqtt_takes() {
    let mut rig = Rig::new();
    let session = rig.setup();

    let r = rig.go(ApiRequest::new(Method::Post, "/api/v1/nodes/3/press")
        .with_auth(Auth::Session(session.clone()))
        .with_body(br#"{"switch":"rst","duration_ms":250}"#));
    assert_eq!(r.status, 200, "{}", body_text(&r));
    let cmd = rig.commands.seen.last().unwrap().clone();
    assert_eq!(cmd.action.as_str(), "press");
    assert_eq!(cmd.target, granite_core::Target::Node(3));
    assert_eq!(cmd.args.duration_ms, Some(250));
    assert_eq!(body_json(&r)["ok"], true);

    // The raw command route takes the MQTT payload byte for byte.
    let r = rig.go(ApiRequest::new(Method::Post, "/api/v1/cmd")
        .with_auth(Auth::Session(session.clone()))
        .with_body(br#"{"v":1,"id":"c-7","action":"on","target":2,"args":{}}"#));
    assert_eq!(r.status, 200, "{}", body_text(&r));
    assert_eq!(body_json(&r)["id"], "c-7");

    // A refused action is a client error, with the reason in the ack.
    let r = rig.go(ApiRequest::new(Method::Post, "/api/v1/nodes/1/reset")
        .with_auth(Auth::Session(session.clone()))
        .with_body(b"{}"));
    assert_eq!(r.status, 400);
    assert_eq!(body_json(&r)["ok"], false);

    // reboot, probe scan and rule ack all go through the dispatcher.
    for (path, want) in [
        ("/api/v1/reboot", SideEffect::Reboot),
        ("/api/v1/probes/scan", SideEffect::ProbeScan),
    ] {
        let r =
            rig.go(ApiRequest::new(Method::Post, path).with_auth(Auth::Session(session.clone())));
        assert_eq!(r.status, 200, "{path}");
        assert!(rig.commands.effects.contains(&want), "{path}");
    }
    let r = rig.go(ApiRequest::new(Method::Post, "/api/v1/rules/1/ack")
        .with_auth(Auth::Session(session.clone())));
    assert_eq!(r.status, 200);
    assert_eq!(rig.commands.seen.last().unwrap().args.rule, Some(1));
}

#[test]
fn factory_reset_needs_the_device_id() {
    let mut rig = Rig::new();
    let session = rig.setup();

    let r = rig.go(ApiRequest::new(Method::Post, "/api/v1/factory-reset")
        .with_auth(Auth::Session(session.clone()))
        .with_body(br#"{"confirm":"granite-wrong"}"#));
    assert_eq!(r.status, 400);
    assert!(!rig.commands.effects.contains(&SideEffect::FactoryReset));

    let r = rig.go(ApiRequest::new(Method::Post, "/api/v1/factory-reset")
        .with_auth(Auth::Session(session))
        .with_body(format!(r#"{{"confirm":"{DEVICE}"}}"#)));
    assert_eq!(r.status, 200);
    assert!(rig.commands.effects.contains(&SideEffect::FactoryReset));
}

// ---------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------

#[test]
fn a_net_section_put_goes_through_stage_and_confirm() {
    let mut rig = Rig::new();
    let session = rig.setup();

    let body = br#"{"ip_mode":"static","address":"192.168.1.10/24","gateway":"192.168.1.1"}"#;
    let r = rig.go(ApiRequest::new(Method::Put, "/api/v1/config/net")
        .with_auth(Auth::Session(session.clone()))
        .with_body(body));
    assert_eq!(r.status, 200, "{}", body_text(&r));
    let v = body_json(&r);
    assert_eq!(v["staged"], true);
    assert_eq!(v["confirm_s"], 300);
    assert_eq!(rig.net.staged.len(), 1);
    assert_eq!(rig.net.staged[0].0, Section::Net);
    // Staged, not written: the store never saw the net section.
    assert!(!rig.store.saves.contains(&Section::Net));
    assert_eq!(rig.config.net.address, "192.168.1.10/24");

    // The status page shows what is waiting.
    let r = rig
        .go(ApiRequest::new(Method::Get, "/api/v1/status")
            .with_auth(Auth::Session(session.clone())));
    assert_eq!(body_json(&r)["staged"]["seconds_left"], 300);

    let r = rig.go(ApiRequest::new(Method::Post, "/api/v1/config/confirm")
        .with_auth(Auth::Session(session.clone())));
    assert_eq!(r.status, 200);
    assert_eq!(rig.net.confirmed, 1);

    // A second confirm has nothing to confirm.
    let r = rig.go(ApiRequest::new(Method::Post, "/api/v1/config/confirm")
        .with_auth(Auth::Session(session.clone())));
    assert_eq!(r.status, 409);

    // A nodes section is not reachability-affecting: straight through.
    let r = rig.go(ApiRequest::new(Method::Put, "/api/v1/config/nodes")
        .with_auth(Auth::Session(session.clone()))
        .with_body(br#"{"t_probe_s":30}"#));
    assert_eq!(r.status, 200, "{}", body_text(&r));
    assert_eq!(body_json(&r)["staged"], false);
    assert!(rig.store.saves.contains(&Section::Nodes));
    assert_eq!(rig.config.nodes.t_probe_s, 30);

    // An invalid section is refused before anything is written.
    let r = rig.go(ApiRequest::new(Method::Put, "/api/v1/config/net")
        .with_auth(Auth::Session(session))
        .with_body(br#"{"ip_mode":"static","address":""}"#));
    assert_eq!(r.status, 400);
    assert!(body_text(&r).contains("net.address"));
    assert!(rig.net.staged.is_empty());
}

#[test]
fn export_contains_the_config_and_no_secrets() {
    let mut rig = Rig::new();
    let session = rig.setup();
    let r = rig.go(ApiRequest::new(Method::Post, "/api/v1/security/tokens")
        .with_auth(Auth::Session(session.clone()))
        .with_body(br#"{"name":"deploy"}"#));
    let token = body_json(&r)["token"].as_str().unwrap().to_string();
    rig.store.secrets.mqtt_password = String::from("brokerpassword");
    rig.store.secrets.device_key_pem = String::from("-----BEGIN PRIVATE KEY-----");

    let r =
        rig.go(
            ApiRequest::new(Method::Get, "/api/v1/config/export").with_auth(Auth::Session(session))
        );
    assert_eq!(r.status, 200);
    let text = body_text(&r);
    assert!(
        r.headers
            .iter()
            .any(|(k, v)| *k == "Content-Disposition" && v.contains(DEVICE))
    );
    for secret in [
        "admin_hash",
        "admin_salt",
        "api_tokens",
        "mqtt_password",
        "brokerpassword",
        "recovery_token",
        "device_key_pem",
        "PRIVATE KEY",
        &token,
        &rig.store.secrets.admin_hash,
    ] {
        assert!(!text.contains(secret), "export leaked {secret}");
    }
    // It is a usable config document.
    let round = Config::import_json(&text).expect("the export imports again");
    assert_eq!(round.schema_version, rig.config.schema_version);
}

#[test]
fn import_stages_the_reachability_sections_and_saves_the_rest() {
    let mut rig = Rig::new();
    let session = rig.setup();
    let mut next = rig.config.clone();
    next.net.hostname = String::from("granite-lab");
    next.nodes.t_probe_s = 20;
    next.sys.timezone = String::from("Europe/Brussels");
    let doc = next.export_json().unwrap();

    let r = rig.go(ApiRequest::new(Method::Post, "/api/v1/config/import")
        .with_auth(Auth::Session(session))
        .with_body(doc));
    assert_eq!(r.status, 200, "{}", body_text(&r));
    let v = body_json(&r);
    let staged: Vec<String> = serde_json::from_value(v["staged"].clone()).unwrap();
    let saved: Vec<String> = serde_json::from_value(v["saved"].clone()).unwrap();
    assert_eq!(staged, vec!["net"]);
    assert!(saved.contains(&String::from("nodes")));
    assert!(saved.contains(&String::from("sys")));
    assert_eq!(rig.config.net.hostname, "granite-lab");
}

#[test]
fn rules_round_trip_as_a_section_or_a_bare_array() {
    let mut rig = Rig::new();
    let session = rig.setup();

    let r =
        rig.go(
            ApiRequest::new(Method::Get, "/api/v1/rules").with_auth(Auth::Session(session.clone()))
        );
    assert_eq!(r.status, 200);
    let got = body_json(&r);
    assert_eq!(got["rules"].as_array().unwrap().len(), 3);

    let mut rules = got["rules"].clone();
    rules[0]["enabled"] = serde_json::Value::Bool(true);
    let r = rig.go(ApiRequest::new(Method::Put, "/api/v1/rules")
        .with_auth(Auth::Session(session.clone()))
        .with_body(serde_json::to_vec(&rules).unwrap()));
    assert_eq!(r.status, 200, "{}", body_text(&r));
    assert!(rig.config.rules.rules[0].enabled);
    assert!(rig.store.saves.contains(&Section::Rules));

    // A duplicate rule id is refused by the config validator.
    let dup = serde_json::json!([
        {"id": 1, "source": "probe_max", "op": ">", "threshold": 1, "action": "event", "target": "all"},
        {"id": 1, "source": "probe_max", "op": ">", "threshold": 2, "action": "event", "target": "all"},
    ]);
    let r = rig.go(ApiRequest::new(Method::Put, "/api/v1/rules")
        .with_auth(Auth::Session(session))
        .with_body(serde_json::to_vec(&dup).unwrap()));
    assert_eq!(r.status, 400);
    assert!(body_text(&r).contains("duplicate rule id"));
}

#[test]
fn the_modbus_allowlist_is_required_before_the_server_turns_on() {
    let mut rig = Rig::new();
    let session = rig.setup();

    let r = rig.go(
        ApiRequest::new(Method::Put, "/api/v1/security/modbus-allowlist")
            .with_auth(Auth::Session(session.clone()))
            .with_body(br#"{"enabled":true,"allow":[]}"#),
    );
    assert_eq!(r.status, 400);
    assert!(body_text(&r).contains("allow"));

    let r = rig.go(
        ApiRequest::new(Method::Put, "/api/v1/security/modbus-allowlist")
            .with_auth(Auth::Session(session.clone()))
            .with_body(br#"{"enabled":true,"allow":["10.0.0.0/8"]}"#),
    );
    assert_eq!(r.status, 200, "{}", body_text(&r));
    assert!(rig.config.sec.modbus.enabled);

    let r = rig.go(
        ApiRequest::new(Method::Get, "/api/v1/security/modbus-allowlist")
            .with_auth(Auth::Session(session)),
    );
    assert_eq!(body_json(&r)["allow"][0], "10.0.0.0/8");
}

// ---------------------------------------------------------------------
// Security, firmware, recovery
// ---------------------------------------------------------------------

#[test]
fn the_fleet_key_and_the_device_cert_are_validated_before_storage() {
    let mut rig = Rig::new();
    let session = rig.setup();

    let r = rig.go(ApiRequest::new(Method::Put, "/api/v1/security/fleet-key")
        .with_auth(Auth::Session(session.clone()))
        .with_body(br#"{"pem":"garbage"}"#));
    assert_eq!(r.status, 400);
    assert!(rig.config.sec.fleet_recovery_pubkey_pem.is_empty());

    let r = rig.go(ApiRequest::new(Method::Put, "/api/v1/security/fleet-key")
        .with_auth(Auth::Session(session.clone()))
        .with_body(br#"{"pem":"-----BEGIN PUBLIC KEY-----\nxx\n-----END PUBLIC KEY-----\n"}"#));
    assert_eq!(r.status, 200, "{}", body_text(&r));
    assert!(
        rig.config
            .sec
            .fleet_recovery_pubkey_pem
            .contains("PUBLIC KEY")
    );

    let r = rig.go(
        ApiRequest::new(Method::Put, "/api/v1/security/cert")
            .with_auth(Auth::Session(session))
            .with_body(
                br#"{"cert_pem":"-----BEGIN CERTIFICATE-----x","key_pem":"-----BEGIN PRIVATE KEY-----y"}"#,
            ),
    );
    assert_eq!(r.status, 200, "{}", body_text(&r));
    assert_eq!(body_json(&r)["cert_sha256"], "bb".repeat(32));
}

#[test]
fn mqtt_credentials_land_in_the_secrets_and_the_username_in_the_config() {
    let mut rig = Rig::new();
    let session = rig.setup();
    let auth = Auth::Session(session);

    // A password and a username in one call: the password is a secret,
    // the username is public material in the `mqtt` section.
    let r = rig.go(
        ApiRequest::new(Method::Put, "/api/v1/security/mqtt-credentials")
            .with_auth(auth.clone())
            .with_body(br#"{"username":"granite-1","password":"s3cret"}"#),
    );
    assert_eq!(r.status, 204, "{}", body_text(&r));
    assert!(r.body.is_empty());
    assert_eq!(rig.store.secrets.mqtt_password, "s3cret");
    assert_eq!(rig.config.mqtt.username, "granite-1");
    assert!(rig.store.saves.contains(&Section::Mqtt));
    assert!(
        rig.platform
            .security_events()
            .contains(&String::from("mqtt_credentials_set"))
    );

    // A client certificate without its key is refused; the pair is not.
    let r = rig.go(
        ApiRequest::new(Method::Put, "/api/v1/security/mqtt-credentials")
            .with_auth(auth.clone())
            .with_body(br#"{"client_cert_pem":"-----BEGIN CERTIFICATE-----x"}"#),
    );
    assert_eq!(r.status, 400, "{}", body_text(&r));
    assert!(rig.store.secrets.mqtt_client_cert_pem.is_empty());

    let r = rig.go(
        ApiRequest::new(Method::Put, "/api/v1/security/mqtt-credentials")
            .with_auth(auth.clone())
            .with_body(
                br#"{"client_cert_pem":"-----BEGIN CERTIFICATE-----x",
                     "client_key_pem":"-----BEGIN PRIVATE KEY-----y"}"#,
            ),
    );
    assert_eq!(r.status, 204, "{}", body_text(&r));
    assert!(
        rig.store
            .secrets
            .mqtt_client_cert_pem
            .contains("CERTIFICATE")
    );
    assert!(
        rig.store
            .secrets
            .mqtt_client_key_pem
            .contains("PRIVATE KEY")
    );
    // The password set earlier survived a call that did not mention it.
    assert_eq!(rig.store.secrets.mqtt_password, "s3cret");

    // An empty body changes nothing, and a non-string field is a 400.
    let r = rig.go(
        ApiRequest::new(Method::Put, "/api/v1/security/mqtt-credentials")
            .with_auth(auth.clone())
            .with_body(b"{}"),
    );
    assert_eq!(r.status, 400, "{}", body_text(&r));
    let r = rig.go(
        ApiRequest::new(Method::Put, "/api/v1/security/mqtt-credentials")
            .with_auth(auth.clone())
            .with_body(br#"{"password":7}"#),
    );
    assert_eq!(r.status, 400, "{}", body_text(&r));

    // An explicit empty string clears a stored secret.
    let r = rig.go(
        ApiRequest::new(Method::Put, "/api/v1/security/mqtt-credentials")
            .with_auth(auth.clone())
            .with_body(br#"{"password":""}"#),
    );
    assert_eq!(r.status, 204, "{}", body_text(&r));
    assert!(rig.store.secrets.mqtt_password.is_empty());

    // Unauthenticated callers get nowhere.
    let r = rig.go(
        ApiRequest::new(Method::Put, "/api/v1/security/mqtt-credentials")
            .with_body(br#"{"password":"nope"}"#),
    );
    assert_eq!(r.status, 401);
    assert!(rig.store.secrets.mqtt_password.is_empty());
}

#[test]
fn recovery_takes_a_token_or_a_fleet_signature_once_a_minute() {
    let mut rig = Rig::new();
    let session = rig.setup();
    let r = rig.go(ApiRequest::new(Method::Put, "/api/v1/security/fleet-key")
        .with_auth(Auth::Session(session))
        .with_body(br#"{"pem":"-----BEGIN PUBLIC KEY-----\nxx\n-----END PUBLIC KEY-----\n"}"#));
    assert_eq!(r.status, 200);

    // A wrong token is rejected and logged.
    let r = rig.go(ApiRequest::new(Method::Post, "/recover").with_body(br#"{"token":"NOPE"}"#));
    assert_eq!(r.status, 403);
    assert!(!rig.commands.effects.contains(&SideEffect::FactoryReset));

    // The rate limit is one attempt per minute.
    let r = rig.go(ApiRequest::new(Method::Post, "/recover").with_body(br#"{"token":"NOPE"}"#));
    assert_eq!(r.status, 429);

    // A fleet signature needs a fresh nonce from /id.
    rig.now_ms += api::RECOVER_INTERVAL_MS;
    let r = rig.go(ApiRequest::new(Method::Get, "/id"));
    let nonce = body_json(&r)["nonce"].as_str().unwrap().to_string();
    let body = format!(r#"{{"device":"{DEVICE}","nonce":"{nonce}","sig":"good-signature"}}"#);
    let r = rig.go(ApiRequest::new(Method::Post, "/recover").with_body(body.clone()));
    assert_eq!(r.status, 200, "{}", body_text(&r));
    assert!(rig.commands.effects.contains(&SideEffect::FactoryReset));

    // The nonce is single use.
    rig.now_ms += api::RECOVER_INTERVAL_MS;
    let r = rig.go(ApiRequest::new(Method::Post, "/recover").with_body(body));
    assert_eq!(r.status, 403);
    assert!(body_text(&r).contains("nonce"));

    // And the per-device token works.
    rig.now_ms += api::RECOVER_INTERVAL_MS;
    let r = rig
        .go(ApiRequest::new(Method::Post, "/recover")
            .with_body(br#"{"token":"AAAA-BBBB-CCCC-DDDD"}"#));
    assert_eq!(r.status, 200, "{}", body_text(&r));

    let events = rig.platform.security_events();
    assert_eq!(events.iter().filter(|e| *e == "recover_failed").count(), 2);
    assert_eq!(
        events.iter().filter(|e| *e == "recover_accepted").count(),
        2
    );
}

#[test]
fn a_firmware_upload_streams_into_the_sink() {
    let mut rig = Rig::new();
    let session = rig.setup();

    // Buffered body: begin, write, finish.
    let r = rig.go(ApiRequest::new(Method::Post, "/api/v1/firmware/upload")
        .with_auth(Auth::Session(session.clone()))
        .with_body(b"\xe9image-bytes"));
    assert_eq!(r.status, 200, "{}", body_text(&r));
    let v = body_json(&r);
    assert_eq!(v["slot"], "ota_1");
    assert_eq!(v["reboot_required"], true);
    assert!(rig.ota.begun);

    // Streamed body: the transport reports what it pushed.
    rig.ota = FakeOta {
        written: b"\xe9image".to_vec(),
        begun: true,
        aborted: false,
    };
    let mut req = ApiRequest::new(Method::Post, "/api/v1/firmware/upload")
        .with_auth(Auth::Session(session.clone()));
    req.upload = Some(api::UploadOutcome {
        bytes: 6,
        error: None,
    });
    let r = rig.go(req);
    assert_eq!(r.status, 200, "{}", body_text(&r));

    // A failed stream aborts the image.
    let mut req = ApiRequest::new(Method::Post, "/api/v1/firmware/upload")
        .with_auth(Auth::Session(session.clone()));
    req.upload = Some(api::UploadOutcome {
        bytes: 3,
        error: Some(String::from("connection reset")),
    });
    let r = rig.go(req);
    assert_eq!(r.status, 500);
    assert!(rig.ota.aborted);

    // An empty body is a client error, and the slots are readable.
    let r = rig.go(ApiRequest::new(Method::Post, "/api/v1/firmware/upload")
        .with_auth(Auth::Session(session.clone())));
    assert_eq!(r.status, 400);

    let r = rig
        .go(ApiRequest::new(Method::Get, "/api/v1/firmware")
            .with_auth(Auth::Session(session.clone())));
    assert_eq!(body_json(&r)["key_id"], "deadbeef");

    let r = rig.go(ApiRequest::new(Method::Post, "/api/v1/firmware/rollback")
        .with_auth(Auth::Session(session.clone())));
    assert_eq!(r.status, 200);
    assert!(rig.platform.rolled_back);

    let r = rig.go(ApiRequest::new(Method::Post, "/api/v1/firmware/mark-valid")
        .with_auth(Auth::Session(session)));
    assert_eq!(r.status, 200);
    assert!(rig.platform.marked_valid);
}

// ---------------------------------------------------------------------
// Shapes and transport contract
// ---------------------------------------------------------------------

#[test]
fn status_and_state_have_the_documented_shape() {
    let mut rig = Rig::new();
    let session = rig.setup();

    let r = rig
        .go(ApiRequest::new(Method::Get, "/api/v1/status")
            .with_auth(Auth::Session(session.clone())));
    let v = body_json(&r);
    for key in [
        "device",
        "mac",
        "fw",
        "cert_sha256",
        "boot_reason",
        "uptime_s",
        "free_heap",
        "state",
        "net",
        "mqtt",
        "ota",
        "modbus",
    ] {
        assert!(v.get(key).is_some(), "status is missing {key}");
    }
    assert_eq!(v["net"]["ip"], "192.168.1.10");
    assert_eq!(v["state"]["nodes"].as_array().unwrap().len(), 8);
    assert_eq!(v["ota"]["slots"][0]["label"], "ota_0");

    let r =
        rig.go(
            ApiRequest::new(Method::Get, "/api/v1/state").with_auth(Auth::Session(session.clone()))
        );
    let v = body_json(&r);
    assert_eq!(v["v"], 1);
    assert_eq!(v["nodes"][0]["node"], 1);
    assert_eq!(v["nodes"][0]["state"], "unknown");

    let r =
        rig.go(
            ApiRequest::new(Method::Get, "/api/v1/nodes").with_auth(Auth::Session(session.clone()))
        );
    let v = body_json(&r);
    assert_eq!(v["settings"]["order"][0], 1);
    assert_eq!(v["nodes"].as_array().unwrap().len(), 8);

    let r = rig.go(ApiRequest::new(Method::Get, "/api/v1/log/tail")
        .with_auth(Auth::Session(session))
        .with_query("lines=5"));
    assert_eq!(r.status, 200);
    assert_eq!(r.content_type, "text/plain; charset=utf-8");
}

#[test]
fn assets_are_streamed_and_methods_are_checked() {
    let mut rig = Rig::new();

    let r = rig.go(ApiRequest::new(Method::Get, "/"));
    assert_eq!(r.status, 200);
    assert_eq!(
        r.body,
        Body::Stream(StreamKind::Asset(String::from("/index.html")))
    );

    let r = rig.go(ApiRequest::new(Method::Post, "/index.html"));
    assert_eq!(r.status, 405);

    let r = rig.go(ApiRequest::new(Method::Get, "/api/v1/cmd"));
    assert_eq!(r.status, 403); // no password yet, so the gate speaks first

    let session = rig.setup();
    let r = rig.go(ApiRequest::new(Method::Get, "/api/v1/cmd").with_auth(Auth::Session(session)));
    assert_eq!(r.status, 405);

    let r = rig.go(ApiRequest::new(Method::Get, "/api/v1/nope"));
    assert_eq!(r.status, 404);
}

#[test]
fn credentials_come_out_of_the_headers() {
    assert_eq!(
        Auth::from_headers(Some("a=1; granite_session=abc; b=2"), None),
        Auth::Session(String::from("abc"))
    );
    assert_eq!(
        Auth::from_headers(Some("granite_session=abc"), Some("Bearer tok")),
        Auth::Bearer(String::from("tok"))
    );
    assert_eq!(Auth::from_headers(None, None), Auth::None);
    assert_eq!(Auth::from_headers(Some("other=1"), None), Auth::None);
}

#[test]
fn pbkdf2_matches_the_documented_parameters() {
    let salt = api::unhex("00112233445566778899aabbccddeeff").unwrap();
    let hash = api::pbkdf2_hex(b"correct horse", &salt, api::PBKDF2_ITERS);
    assert_eq!(hash.len(), api::HASH_BYTES * 2);
    assert!(api::verify_hash(
        b"correct horse",
        "00112233445566778899aabbccddeeff",
        &hash,
        api::PBKDF2_ITERS
    ));
    assert!(!api::verify_hash(
        b"wrong horse",
        "00112233445566778899aabbccddeeff",
        &hash,
        api::PBKDF2_ITERS
    ));
    // A corrupt stored value fails closed instead of panicking.
    assert!(!api::verify_hash(b"x", "zz", &hash, api::PBKDF2_ITERS));
    assert!(!api::verify_hash(b"x", "00", "", api::PBKDF2_ITERS));
}
