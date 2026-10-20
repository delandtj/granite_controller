//! HTTPS setup page and JSON API (ADR 0001 component 8).
//!
//! This module is glue and nothing else. Every route, every
//! authentication rule, the first-boot gate, the login lockout, the
//! recovery nonce and the JSON shapes live in
//! [`granite_core::api`], which is host-tested; what is here reads an
//! ESP-IDF request into an [`ApiRequest`], calls
//! [`granite_core::api::handle`], and writes the [`ApiResponse`] back.
//!
//! Two servers are started:
//!
//! - HTTPS on [`HttpCtx::https_port`] (443) with the device certificate
//!   the platform layer supplies as PEM. This needs
//!   `CONFIG_ESP_HTTPS_SERVER_ENABLE=y`; without it `esp-idf-svc` does
//!   not even expose the certificate fields.
//! - Plain HTTP on [`HttpCtx::http_port`] (80) which serves only `/id`
//!   (the unauthenticated identity endpoint that is the recovery path,
//!   ADR component 8) and a redirect to HTTPS for everything else.
//!
//! Bodies are capped at [`granite_core::api::MAX_BODY`] (64 KB). The one
//! exception is `POST /api/v1/firmware/upload`, which is authenticated
//! first and then streamed into the [`OtaSink`] in
//! [`CHUNK`]-byte pieces, so a 2.5 MB image never sits in RAM.
//!
//! ### Locking
//!
//! A request needs every platform object at once, so the guards are
//! taken in one fixed order and released together:
//! `auth -> store -> platform -> identity -> net -> ota -> rng ->
//! config -> observed`. Nothing else in the firmware may take two of
//! these at the same time in a different order.
//!
//! No guard is ever held while something is waited for. A `?wait=1` node
//! action is the one request that waits at all, and it waits in
//! [`handle`] *after* the guards are dropped: holding the configuration
//! write guard for seconds would park every blocking reader of the live
//! configuration, the MQTT worker's 5 s refresh first of all.

use std::ffi::CString;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use esp_idf_svc::io::Write as _;
use esp_idf_svc::http::Method as HttpMethod;
use esp_idf_svc::http::server::{Configuration, EspHttpConnection, EspHttpServer, Request};
use esp_idf_svc::io::EspIOError;
use esp_idf_svc::sys::esp_timer_get_time;
use esp_idf_svc::tls::X509;

use granite_core::api::{
    self, ApiCtx, ApiRequest, ApiResponse, Auth, AuthState, Body, Identity, Method, NetControl,
    OtaSink, Platform, Rng, Store, StreamKind, UploadOutcome,
};
use granite_core::config::Config;
use granite_core::msg::{Command, Event, EventKind, Reply};
use granite_core::observed::Observed;

use crate::platform::EventBus;

/// Bytes read from the socket at a time while streaming an image.
pub const CHUNK: usize = 2048;

/// Largest firmware image accepted on the upload route.
pub const MAX_IMAGE: usize = 3 * 1024 * 1024;

/// How long the HTTP task waits for the actuator task to answer a
/// command before it gives up on the request.
pub const COMMAND_TIMEOUT: Duration = Duration::from_secs(20);

/// Ceiling on `?wait=1` (ADR component 8), on top of
/// [`granite_core::api::ACTION_WAIT_MS`].
///
/// The trade-off, and why this is opt-in: ESP-IDF's HTTP server runs
/// every handler on its own single server task (`sdkconfig.defaults` sets
/// no `CONFIG_HTTPD_*`, so there is one), so a request parked here
/// answers nothing else - the setup page included - until the action
/// finishes. That is the whole cost: the wait runs with no guard held
/// (see [`handle`]), so the dispatcher, the MQTT worker, the sense task
/// and Modbus all carry on. Nothing in the server configuration shortens
/// it for us:
/// `esp-idf-svc` 0.53 hard-codes `recv_wait_timeout` and
/// `send_wait_timeout` to 5 s, and those are socket read/write deadlines,
/// not a handler deadline, while its `session_timeout` and `max_sessions`
/// never reach `httpd_config_t` at all. So the cap is ours to pick, and
/// the effective wait is `min(ACTION_WAIT_MS, WAIT_CAP)`: 30 s, which is
/// an `on` or a `reset` with room to spare but well under a
/// `t_soft_off_ms` of two minutes. A longer action keeps running and the
/// reply falls back to `accepted`; the outcome is in
/// `state.nodes[i].last_action` either way.
pub const WAIT_CAP: Duration = Duration::from_secs(30);

/// Control port of the HTTPS server. The plain server needs its own.
const HTTPS_CTRL_PORT: u16 = 32768;
/// Control port of the plain HTTP server.
const HTTP_CTRL_PORT: u16 = 32769;

// ---------------------------------------------------------------------
// The embedded setup page
// ---------------------------------------------------------------------

/// One gzipped asset from `firmware/web/dist/`.
struct GzAsset {
    /// Canonical request path.
    path: &'static str,
    /// Content type of the decompressed file.
    content_type: &'static str,
    /// The gzip stream, served as-is with `Content-Encoding: gzip`.
    gz: &'static [u8],
}

/// The setup page. Regenerate with `firmware/web/build.sh` after editing
/// anything in `firmware/web/`.
static ASSETS: &[GzAsset] = &[
    GzAsset {
        path: "/index.html",
        content_type: "text/html; charset=utf-8",
        gz: include_bytes!("../../web/dist/index.html.gz"),
    },
    GzAsset {
        path: "/app.js",
        content_type: "text/javascript; charset=utf-8",
        gz: include_bytes!("../../web/dist/app.js.gz"),
    },
    GzAsset {
        path: "/style.css",
        content_type: "text/css; charset=utf-8",
        gz: include_bytes!("../../web/dist/style.css.gz"),
    },
];

fn asset(path: &str) -> Option<&'static GzAsset> {
    ASSETS.iter().find(|a| a.path == path)
}

// ---------------------------------------------------------------------
// What the HTTP task is handed
// ---------------------------------------------------------------------

/// The system view (ADR components 6, 11, 12).
pub type SharedPlatform = Arc<Mutex<dyn Platform + Send>>;
/// Identity and recovery (ADR component 13).
pub type SharedIdentity = Arc<Mutex<dyn Identity + Send>>;
/// Commit-confirmed configuration (ADR component 6).
pub type SharedNetControl = Arc<Mutex<dyn NetControl + Send>>;
/// The OTA partition writer (ADR component 11).
pub type SharedOtaSink = Arc<Mutex<dyn OtaSink + Send>>;
/// Hardware randomness.
pub type SharedRng = Arc<Mutex<dyn Rng + Send>>;
/// NVS, through the API's narrow view of it (ADR component 10).
pub type SharedStore = Arc<Mutex<dyn Store + Send>>;

/// One command plus the channel its ack goes back on.
pub type CommandRequest = (Command, Sender<Reply>);

/// The HTTP side of the command channel from the ADR's data-flow
/// diagram: `HTTP api --> Command channel --> dispatcher`.
///
/// The actuator task owns the [`Receiver`]; for every message it runs
/// [`granite_core::dispatch::dispatch`] and sends the [`Reply`] back on
/// the channel that came with the command.
pub struct CommandChannel {
    tx: Mutex<Sender<CommandRequest>>,
    timeout: Duration,
}

impl CommandChannel {
    /// Build a channel pair. The receiver goes to the actuator task.
    pub fn new() -> (Arc<CommandChannel>, Receiver<CommandRequest>) {
        let (tx, rx) = channel();
        (
            Arc::new(CommandChannel {
                tx: Mutex::new(tx),
                timeout: COMMAND_TIMEOUT,
            }),
            rx,
        )
    }

    /// Hand one command to the dispatcher without waiting for the ack,
    /// with the channel the reply goes back on. Returns false when the
    /// dispatcher is gone.
    ///
    /// This is the MQTT worker's way in: it has to keep draining the
    /// broker while an action runs, so it resolves the ack against the
    /// matching `action_done` event instead of blocking on [`call`].
    pub fn send(&self, cmd: Command, reply: Sender<Reply>) -> bool {
        match self.tx.lock() {
            Ok(tx) => tx.send((cmd, reply)).is_ok(),
            Err(_) => false,
        }
    }

    /// Send one command and wait for its ack.
    pub fn call(&self, cmd: &Command) -> Reply {
        let (reply_tx, reply_rx) = channel();
        let sent = match self.tx.lock() {
            Ok(tx) => tx.send((cmd.clone(), reply_tx)).is_ok(),
            Err(_) => false,
        };
        if !sent {
            return Reply::err(&cmd.id, "the controller task is not accepting commands");
        }
        match reply_rx.recv_timeout(self.timeout) {
            Ok(reply) => reply,
            Err(_) => Reply::err(&cmd.id, "the controller task did not answer in time"),
        }
    }
}

/// A `?wait=1` wait that [`handle`] still owes the client: the command
/// id to watch for and how long the core allowed.
struct PendingWait {
    id: String,
    timeout_ms: u32,
}

/// [`granite_core::api::Dispatcher`] over the command channel.
///
/// `completions` is a subscription to the event bus the dispatcher thread
/// publishes on, taken by [`handle`] *before* the command is sent and
/// only when the request carries `?wait=1`. Subscribing first is what
/// makes the wait race-free (the `action_done` cannot land between the
/// ack and the subscription) and keeps the dispatcher thread out of it:
/// the bus sends into an unbounded channel and drops a subscriber that
/// stopped reading, so it never waits on a request. `None` means this
/// request did not ask to wait.
struct ChannelDispatcher<'a> {
    commands: &'a CommandChannel,
    completions: Option<Receiver<Event>>,
    pending: Option<PendingWait>,
}

impl api::Dispatcher for ChannelDispatcher<'_> {
    fn dispatch(&mut self, cmd: &Command) -> Reply {
        self.commands.call(cmd)
    }

    /// Note the wait and answer "this transport cannot wait", so the core
    /// hands back its `accepted` reply right away.
    ///
    /// Nothing blocks here on purpose: [`granite_core::api::handle`] runs
    /// with every platform guard held (the configuration write guard
    /// included), and waiting seconds inside it would park the MQTT
    /// worker and anything else that reads the live configuration.
    /// [`handle`] drops the guards first and then runs [`wait_outcome`].
    /// [`COMMAND_TIMEOUT`] is untouched; it still bounds the ack of every
    /// command, waiting or not.
    fn wait_for(&mut self, cmd: &Command, timeout_ms: u32) -> Option<Reply> {
        if self.completions.is_some() {
            self.pending = Some(PendingWait {
                id: cmd.id.clone(),
                timeout_ms,
            });
        }
        None
    }
}

/// Read the event bus until command `pending.id` is done or the cap
/// expires, with no guard held. `None` means the `accepted` reply stands,
/// which is true either way - the action is queued.
fn wait_outcome(completions: &Receiver<Event>, pending: &PendingWait) -> Option<Reply> {
    let cap = WAIT_CAP.min(Duration::from_millis(u64::from(pending.timeout_ms)));
    let deadline = Instant::now() + cap;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        // Timeout, or a bus with no publisher left.
        let event = completions.recv_timeout(left).ok()?;
        if let Some(reply) = finished_reply(&event, &pending.id) {
            return Some(reply);
        }
    }
}

/// The final ack carried by `event`, if that event is command `id`
/// finishing.
///
/// The same rule as the MQTT ack (`mqtt::resolve_from_event`), so both
/// transports report the same outcome for the same command: `escalated`
/// is a step inside `off`, not the end of it, so the wait goes on.
fn finished_reply(event: &Event, id: &str) -> Option<Reply> {
    match &event.kind {
        EventKind::ActionDone {
            id: done, result, ..
        } if done == id && result != "escalated" => Some(Reply::ok(id, result.clone())),
        EventKind::ActionFailed {
            id: failed, error, ..
        } if failed == id => Some(Reply::err(id, error.clone())),
        _ => None,
    }
}

/// Everything the HTTP task needs. Cheap to clone: everything is an
/// `Arc` except the certificate, which is only read once at start.
#[derive(Clone)]
pub struct HttpCtx {
    /// The live configuration, shared with the rest of the firmware.
    pub config: Arc<RwLock<Config>>,
    /// The sensor snapshot, written by the sense task.
    pub observed: Arc<RwLock<Observed>>,
    /// Sessions, lockout and the recovery nonce.
    pub auth: Arc<Mutex<AuthState>>,
    /// NVS.
    pub store: SharedStore,
    /// System state.
    pub platform: SharedPlatform,
    /// Identity and recovery.
    pub identity: SharedIdentity,
    /// Commit-confirm.
    pub net: SharedNetControl,
    /// OTA writer.
    pub ota: SharedOtaSink,
    /// Randomness.
    pub rng: SharedRng,
    /// The command channel into the actuator task.
    pub commands: Arc<CommandChannel>,
    /// The event bus the dispatcher thread publishes on. Only `?wait=1`
    /// subscribes to it, to turn an accepted action into its outcome.
    pub events: Arc<EventBus>,
    /// Device certificate, PEM. Generated on first boot or uploaded.
    pub cert_pem: String,
    /// Its private key, PEM.
    pub key_pem: String,
    /// HTTPS port.
    pub https_port: u16,
    /// Plain HTTP port; serves `/id` and a redirect only.
    pub http_port: u16,
}

impl HttpCtx {
    /// A fresh, empty session store. The caller builds the rest of the
    /// struct field by field; a constructor taking every field at once
    /// would only hide which argument is which:
    ///
    /// ```ignore
    /// let ctx = HttpCtx {
    ///     config: config.clone(),
    ///     observed: observed.clone(),
    ///     auth: HttpCtx::new_auth(),
    ///     store, platform, identity, net, ota, rng, commands, events,
    ///     cert_pem, key_pem,
    ///     https_port: 443,
    ///     http_port: 80,
    /// };
    /// let handle = http::start(ctx)?;
    /// ```
    pub fn new_auth() -> Arc<Mutex<AuthState>> {
        Arc::new(Mutex::new(AuthState::new()))
    }
}

/// The running servers. Dropping this stops them, so `main` keeps it.
pub struct HttpHandle {
    /// The HTTPS server.
    pub https: EspHttpServer<'static>,
    /// The plain HTTP server: `/id` and a redirect.
    pub http: EspHttpServer<'static>,
}

// ---------------------------------------------------------------------
// Start
// ---------------------------------------------------------------------

/// Start both servers.
///
/// The certificate and the key are leaked on purpose: ESP-IDF's HTTPS
/// server keeps pointers to them for the lifetime of the server, and the
/// server lives as long as the firmware does.
pub fn start(ctx: HttpCtx) -> anyhow::Result<HttpHandle> {
    let cert = leak_pem(&ctx.cert_pem).ok_or_else(|| {
        anyhow::anyhow!("the device certificate PEM contains a NUL byte")
    })?;
    let key = leak_pem(&ctx.key_pem)
        .ok_or_else(|| anyhow::anyhow!("the device key PEM contains a NUL byte"))?;

    let mut https_conf = Configuration {
        https_port: ctx.https_port,
        http_port: 0,
        ctrl_port: HTTPS_CTRL_PORT,
        uri_match_wildcard: true,
        max_uri_handlers: 8,
        stack_size: 12288,
        max_open_sockets: 4,
        ..Default::default()
    };
    https_conf.server_certificate = Some(cert);
    https_conf.private_key = Some(key);

    let mut https = EspHttpServer::new(&https_conf)?;
    register_api(&mut https, &ctx)?;

    // The plain server gets no certificate, which starts the ESP-IDF
    // HTTPS server in insecure mode on `http_port`.
    let http_conf = Configuration {
        https_port: 0,
        http_port: ctx.http_port,
        ctrl_port: HTTP_CTRL_PORT,
        uri_match_wildcard: true,
        max_uri_handlers: 4,
        // `/id` runs the same core handler as on 443; at 6144 it overflowed
        // in identity_response -> new_nonce (rev C, 2026-10-16).
        stack_size: 10240,
        max_open_sockets: 2,
        ..Default::default()
    };
    let mut http = EspHttpServer::new(&http_conf)?;
    register_plain(&mut http, &ctx)?;

    log::info!(
        "https on {}, plain /id and redirect on {}",
        ctx.https_port,
        ctx.http_port
    );
    Ok(HttpHandle { https, http })
}

fn leak_pem(pem: &str) -> Option<X509<'static>> {
    let c = CString::new(pem).ok()?;
    let bytes: &'static [u8] = Box::leak(c.into_bytes_with_nul().into_boxed_slice());
    Some(X509::pem_until_nul(bytes))
}

/// Wildcard handlers, one per method, all funnelling into [`serve`].
fn register_api(server: &mut EspHttpServer<'static>, ctx: &HttpCtx) -> Result<(), EspIOError> {
    for method in [
        HttpMethod::Get,
        HttpMethod::Post,
        HttpMethod::Put,
        HttpMethod::Delete,
        // The core route table answers HEAD for every GET; a monitoring
        // tool that probes with HEAD gets the headers and no body.
        HttpMethod::Head,
    ] {
        let ctx = ctx.clone();
        server.fn_handler("/*", method, move |request| serve(&ctx, request))?;
    }
    Ok(())
}

/// Plain HTTP: `/id` when the configuration allows it, a redirect to
/// HTTPS for everything else.
fn register_plain(server: &mut EspHttpServer<'static>, ctx: &HttpCtx) -> Result<(), EspIOError> {
    let id_ctx = ctx.clone();
    server.fn_handler("/id", HttpMethod::Get, move |request| {
        let allowed = id_ctx
            .config
            .read()
            .map(|c| c.sec.id_on_plain_http)
            .unwrap_or(false);
        if allowed {
            serve(&id_ctx, request)
        } else {
            redirect(&id_ctx, request)
        }
    })?;
    let redirect_ctx = ctx.clone();
    server.fn_handler("/*", HttpMethod::Get, move |request| {
        redirect(&redirect_ctx, request)
    })?;
    Ok(())
}

fn redirect(ctx: &HttpCtx, request: Request<&mut EspHttpConnection>) -> Result<(), EspIOError> {
    let host = request
        .header("Host")
        .map(|h| h.split(':').next().unwrap_or(h).to_string())
        .unwrap_or_default();
    let location = if host.is_empty() {
        String::from("/")
    } else if ctx.https_port == 443 {
        format!("https://{host}/")
    } else {
        format!("https://{host}:{}/", ctx.https_port)
    };
    let mut response = request.into_response(
        301,
        Some("Moved Permanently"),
        &[("Location", location.as_str())],
    )?;
    response.write_all(b"use HTTPS\n")?;
    Ok(())
}

// ---------------------------------------------------------------------
// One request
// ---------------------------------------------------------------------

/// Monotonic milliseconds, the clock the core compares against.
fn now_ms() -> u64 {
    (unsafe { esp_timer_get_time() } / 1000) as u64
}

fn serve(ctx: &HttpCtx, mut request: Request<&mut EspHttpConnection>) -> Result<(), EspIOError> {
    // An accepted request is proof the board is reachable, which is half
    // of the static-config dead-man (ADR component 6: no gateway answer
    // *and* no accepted connection for t_deadman). Counted before
    // authentication: the TCP connection is the evidence, not the
    // password, and an operator staring at the login page must not lose
    // the address under them.
    crate::platform::net::note_connection();
    let now = now_ms();
    let uri = request.uri().to_string();
    let (path, query) = match uri.split_once('?') {
        Some((p, q)) => (p.to_string(), q.to_string()),
        None => (uri.clone(), String::new()),
    };
    let auth = Auth::from_headers(request.header("Cookie"), request.header("Authorization"));
    let peer = peer_of(&mut request);
    let content_length: usize = request
        .header("Content-Length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);

    let mut api_request = ApiRequest {
        method: method_of(request.method()),
        path,
        query,
        body: Vec::new(),
        auth,
        peer,
        upload: None,
    };

    let is_upload = api_request.method == Method::Post
        && api::route_of(&api_request.path) == Some(api::Route::FirmwareUpload);

    if is_upload {
        if !authenticated(ctx, &api_request, now) {
            // Do not touch the OTA partition for an unauthenticated
            // request; let the route table produce the 401.
            let response = handle(ctx, api_request, now);
            return write_response(request, response);
        }
        if content_length > MAX_IMAGE {
            let response = ApiResponse::error(413, "image is larger than the OTA partition");
            return write_response(request, response);
        }
        api_request.upload = Some(stream_image(ctx, &mut request, content_length));
    } else if content_length > api::MAX_BODY {
        let response = ApiResponse::error(413, "request body is larger than 64 KB");
        return write_response(request, response);
    } else if content_length > 0 {
        match read_body(&mut request, content_length) {
            Ok(body) => api_request.body = body,
            Err(e) => return write_response(request, ApiResponse::error(400, e)),
        }
    }

    let response = handle(ctx, api_request, now);
    write_response(request, response)
}

fn method_of(method: HttpMethod) -> Method {
    match method {
        HttpMethod::Get => Method::Get,
        HttpMethod::Head => Method::Head,
        HttpMethod::Post => Method::Post,
        HttpMethod::Put => Method::Put,
        HttpMethod::Delete => Method::Delete,
        HttpMethod::Options => Method::Options,
        _ => Method::Other,
    }
}

/// The peer address, for the security events the ADR wants logged.
/// ESP-IDF only exposes the socket, so the address comes from lwIP.
fn peer_of(request: &mut Request<&mut EspHttpConnection>) -> String {
    use esp_idf_svc::handle::RawHandle as _;
    use esp_idf_svc::sys::{httpd_req_to_sockfd, lwip_getpeername, sockaddr, sockaddr_in};

    let handle = (**request.connection()).handle();
    unsafe {
        let fd = httpd_req_to_sockfd(handle);
        if fd < 0 {
            return String::new();
        }
        let mut addr: sockaddr_in = core::mem::zeroed();
        let mut len = core::mem::size_of::<sockaddr_in>() as u32;
        if lwip_getpeername(
            fd,
            (&mut addr as *mut sockaddr_in).cast::<sockaddr>(),
            &mut len,
        ) != 0
        {
            return String::new();
        }
        let octets = u32::from_be(addr.sin_addr.s_addr).to_be_bytes();
        format!("{}.{}.{}.{}", octets[0], octets[1], octets[2], octets[3])
    }
}

fn read_body(
    request: &mut Request<&mut EspHttpConnection>,
    content_length: usize,
) -> Result<Vec<u8>, String> {
    let mut body = Vec::with_capacity(content_length.min(api::MAX_BODY));
    let mut buf = [0u8; CHUNK];
    while body.len() < content_length {
        let want = (content_length - body.len()).min(CHUNK);
        match request.read(&mut buf[..want]) {
            Ok(0) => break,
            Ok(n) => body.extend_from_slice(&buf[..n]),
            Err(e) => return Err(format!("short read: {e:?}")),
        }
    }
    Ok(body)
}

/// Push the request body straight into the OTA partition.
fn stream_image(
    ctx: &HttpCtx,
    request: &mut Request<&mut EspHttpConnection>,
    content_length: usize,
) -> UploadOutcome {
    let mut outcome = UploadOutcome::default();
    let mut sink = match ctx.ota.lock() {
        Ok(sink) => sink,
        Err(_) => {
            outcome.error = Some(String::from("the OTA writer is unavailable"));
            return outcome;
        }
    };
    let total = (content_length > 0).then_some(content_length as u64);
    if let Err(e) = sink.begin(total) {
        outcome.error = Some(e);
        return outcome;
    }
    let mut buf = [0u8; CHUNK];
    let mut written: u64 = 0;
    loop {
        if content_length > 0 && written >= content_length as u64 {
            break;
        }
        match request.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                if written + n as u64 > MAX_IMAGE as u64 {
                    outcome.error = Some(String::from("image is larger than the OTA partition"));
                    break;
                }
                if let Err(e) = sink.write(&buf[..n]) {
                    outcome.error = Some(e);
                    break;
                }
                written += n as u64;
            }
            Err(e) => {
                outcome.error = Some(format!("upload interrupted: {e:?}"));
                break;
            }
        }
    }
    outcome.bytes = written;
    if outcome.error.is_none() && content_length > 0 && written < content_length as u64 {
        outcome.error = Some(format!(
            "upload truncated at {written} of {content_length} bytes"
        ));
    }
    outcome
}

/// Does this request carry a valid credential? The same rule
/// [`ApiCtx::is_authenticated`] applies; it is repeated here because the
/// upload route has to know before it writes to flash.
fn authenticated(ctx: &HttpCtx, request: &ApiRequest, now: u64) -> bool {
    let Ok(mut auth) = ctx.auth.lock() else {
        return false;
    };
    let Ok(mut store) = ctx.store.lock() else {
        return false;
    };
    let secrets = store.load_secrets();
    if secrets.admin_password_missing() {
        return false;
    }
    match &request.auth {
        Auth::None => false,
        Auth::Session(id) => auth.has_session(id, now),
        // The routed path (`ApiCtx::is_authenticated`) upgrades an old
        // token hash; this pre-check only needs the answer.
        Auth::Bearer(token) => api::find_token(&secrets, token).is_some(),
    }
}

/// Take every lock in the documented order and run the route table.
///
/// The guards live in the inner block and nothing in that block ever
/// waits for the dispatcher to *finish* an action (see
/// [`ChannelDispatcher::wait_for`]): a `?wait=1` action leaves the block
/// as a [`PendingWait`] and the wait runs underneath it, with every guard
/// dropped.
fn handle(ctx: &HttpCtx, request: ApiRequest, now: u64) -> ApiResponse {
    // Before any lock and before the command is queued: a `?wait=1`
    // request needs the completion events of its own action (see
    // [`ChannelDispatcher`]). Every other request skips this and the bus
    // never grows a subscriber for it.
    let completions = api::wants_wait(&request).then(|| ctx.events.subscribe());

    let (response, pending, completions) = {
        let mut auth = match ctx.auth.lock() {
            Ok(g) => g,
            Err(_) => return ApiResponse::error(503, "session state is unavailable"),
        };
        let mut store = match ctx.store.lock() {
            Ok(g) => g,
            Err(_) => return ApiResponse::error(503, "storage is unavailable"),
        };
        let mut platform = match ctx.platform.lock() {
            Ok(g) => g,
            Err(_) => return ApiResponse::error(503, "the platform is unavailable"),
        };
        let mut identity = match ctx.identity.lock() {
            Ok(g) => g,
            Err(_) => return ApiResponse::error(503, "identity is unavailable"),
        };
        let mut net = match ctx.net.lock() {
            Ok(g) => g,
            Err(_) => return ApiResponse::error(503, "the network control is unavailable"),
        };
        let mut ota = match ctx.ota.lock() {
            Ok(g) => g,
            Err(_) => return ApiResponse::error(503, "the OTA writer is unavailable"),
        };
        let mut rng = match ctx.rng.lock() {
            Ok(g) => g,
            Err(_) => return ApiResponse::error(503, "the RNG is unavailable"),
        };
        let mut config = match ctx.config.write() {
            Ok(g) => g,
            Err(_) => return ApiResponse::error(503, "the configuration is unavailable"),
        };
        let observed = match ctx.observed.read() {
            Ok(g) => g.clone(),
            Err(_) => Observed::new(),
        };
        let mut commands = ChannelDispatcher {
            commands: &ctx.commands,
            completions,
            pending: None,
        };

        let mut api_ctx = ApiCtx {
            now_ms: now,
            config: &mut config,
            observed: &observed,
            auth: &mut auth,
            store: &mut *store,
            platform: &mut *platform,
            identity: &mut *identity,
            net: &mut *net,
            ota: &mut *ota,
            rng: &mut *rng,
            commands: &mut commands,
        };
        let response = api::handle(request, &mut api_ctx);
        // `api_ctx` is not used again, so its borrow of `commands` is
        // over and the recorded wait can be taken back out.
        (
            response,
            commands.pending.take(),
            commands.completions.take(),
        )
    };

    // Every guard is gone; only now does `?wait=1` hold the reply, and
    // the outcome replaces the queue ack (400 when the action failed,
    // which is what a refusal already did). The single httpd task is
    // still parked for the duration - that is the cost of the parameter
    // and the reason it is opt-in, see [`WAIT_CAP`].
    match (pending, completions) {
        (Some(pending), Some(completions)) => {
            wait_outcome(&completions, &pending).map_or(response, api::reply_response)
        }
        _ => response,
    }
}

fn write_response(
    request: Request<&mut EspHttpConnection>,
    response: ApiResponse,
) -> Result<(), EspIOError> {
    let ApiResponse {
        status,
        content_type,
        body,
        set_cookie,
        headers,
    } = response;

    // Resolve a streamed asset into the gzip blob to send.
    let (status, content_type, bytes, gzip): (u16, &str, &[u8], bool) = match &body {
        Body::Empty => (status, content_type, &[], false),
        Body::Bytes(b) => (status, content_type, b.as_slice(), false),
        Body::Stream(StreamKind::Asset(path)) => match asset(path) {
            Some(a) => (status, a.content_type, a.gz, true),
            None => (404, "text/plain; charset=utf-8", b"not found\n", false),
        },
    };

    let length = bytes.len().to_string();
    let mut header_list: Vec<(&str, &str)> = Vec::with_capacity(headers.len() + 6);
    if !content_type.is_empty() && !bytes.is_empty() {
        header_list.push(("Content-Type", content_type));
    }
    if gzip {
        header_list.push(("Content-Encoding", "gzip"));
        header_list.push(("Cache-Control", "no-cache"));
    }
    header_list.push(("Content-Length", length.as_str()));
    if let Some(cookie) = &set_cookie {
        header_list.push(("Set-Cookie", cookie.as_str()));
    }
    for (name, value) in &headers {
        header_list.push((name, value.as_str()));
    }
    // The page only ever talks to its own origin.
    header_list.push(("X-Content-Type-Options", "nosniff"));
    header_list.push(("X-Frame-Options", "DENY"));
    header_list.push(("Referrer-Policy", "no-referrer"));

    // HEAD is a GET without the body (RFC 9110 9.3.2). The core route
    // table answers it with the body a GET would return, so the headers
    // above are the right ones - Content-Length included - and only the
    // bytes are dropped here.
    let head_only = request.method() == HttpMethod::Head;
    let mut out = request.into_response(status, None, &header_list)?;
    if !head_only && !bytes.is_empty() {
        out.write_all(bytes)?;
    }
    out.flush()?;
    Ok(())
}

// ---------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------

// The `?wait=1` plumbing is the one piece of this file with no ESP-IDF in
// it. granite-fw only builds for the board, so these run with
// `cargo test --target riscv32imac-esp-espidf` on a devboard; a host
// `cargo test` never sees them. They are type-checked by
// `cargo clippy --all-targets`.
#[cfg(test)]
mod tests {
    use super::*;
    use granite_core::actuator::ActionKind;
    use granite_core::api::Dispatcher as _;
    use granite_core::msg::CommandKind;

    fn done(id: &str, result: &str) -> Event {
        Event::new(
            0,
            EventKind::ActionDone {
                id: id.into(),
                node: Some(1),
                action: ActionKind::On,
                result: result.into(),
            },
        )
    }

    fn failed(id: &str, error: &str) -> Event {
        Event::new(
            0,
            EventKind::ActionFailed {
                id: id.into(),
                node: Some(1),
                action: ActionKind::On,
                error: error.into(),
            },
        )
    }

    /// `escalated` is a step inside `off`, not the end of it, and another
    /// command's completion is not ours.
    #[test]
    fn only_this_commands_final_event_resolves_the_wait() {
        assert!(finished_reply(&done("x", "escalated"), "x").is_none());
        assert!(finished_reply(&done("y", "on"), "x").is_none());
        let reply = finished_reply(&done("x", "on"), "x").expect("on is final");
        assert!(reply.ok);
        assert_eq!(reply.result.as_deref(), Some("on"));
        let reply =
            finished_reply(&failed("x", "power_on_timeout"), "x").expect("a failure ends it");
        assert!(!reply.ok);
        assert_eq!(reply.error.as_deref(), Some("power_on_timeout"));
    }

    /// `wait_for` never blocks: it records the wait and answers "this
    /// transport cannot wait", so the core keeps its `accepted` reply and
    /// [`handle`] can drop every guard before waiting. A request that did
    /// not ask to wait records nothing.
    #[test]
    fn wait_for_records_the_wait_and_does_not_block() {
        let (commands, _rx) = CommandChannel::new();
        let cmd = Command::new("x", CommandKind::of_action(ActionKind::On));

        let (_tx, completions) = channel();
        let mut waiting = ChannelDispatcher {
            commands: &commands,
            completions: Some(completions),
            pending: None,
        };
        assert!(waiting.wait_for(&cmd, 1_000).is_none());
        let pending = waiting.pending.take().expect("the wait was recorded");
        assert_eq!(pending.id, "x");
        assert_eq!(pending.timeout_ms, 1_000);

        let mut not_waiting = ChannelDispatcher {
            commands: &commands,
            completions: None,
            pending: None,
        };
        assert!(not_waiting.wait_for(&cmd, 1_000).is_none());
        assert!(not_waiting.pending.is_none());
    }

    /// The wait walks past events that are not the answer, and gives up
    /// at the cap.
    #[test]
    fn wait_outcome_picks_its_own_completion_and_times_out_otherwise() {
        let (tx, completions) = channel();
        tx.send(done("other", "on")).expect("buffered");
        tx.send(done("x", "escalated")).expect("buffered");
        tx.send(done("x", "off")).expect("buffered");
        let pending = PendingWait {
            id: String::from("x"),
            timeout_ms: 1_000,
        };
        let reply = wait_outcome(&completions, &pending).expect("the third event is ours");
        assert_eq!(reply.result.as_deref(), Some("off"));
        // Nothing left on the bus: the cap runs out and the caller keeps
        // the `accepted` reply.
        let short = PendingWait {
            id: String::from("x"),
            timeout_ms: 10,
        };
        assert!(wait_outcome(&completions, &short).is_none());
    }
}
