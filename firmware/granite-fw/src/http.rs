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

use std::ffi::CString;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

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
use granite_core::msg::{Command, Reply};
use granite_core::observed::Observed;

/// Bytes read from the socket at a time while streaming an image.
pub const CHUNK: usize = 2048;

/// Largest firmware image accepted on the upload route.
pub const MAX_IMAGE: usize = 3 * 1024 * 1024;

/// How long the HTTP task waits for the actuator task to answer a
/// command before it gives up on the request.
pub const COMMAND_TIMEOUT: Duration = Duration::from_secs(20);

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

/// [`granite_core::api::Dispatcher`] over the command channel.
struct ChannelDispatcher<'a>(&'a CommandChannel);

impl api::Dispatcher for ChannelDispatcher<'_> {
    fn dispatch(&mut self, cmd: &Command) -> Reply {
        self.0.call(cmd)
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
    ///     store, platform, identity, net, ota, rng, commands,
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
        stack_size: 6144,
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
        Auth::Bearer(token) => secrets.api_tokens.iter().any(|t| {
            api::verify_hash(
                token.as_bytes(),
                &secrets.admin_salt,
                &t.hash,
                secrets.admin_iters,
            )
        }),
    }
}

/// Take every lock in the documented order and run the route table.
fn handle(ctx: &HttpCtx, request: ApiRequest, now: u64) -> ApiResponse {
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
    let mut commands = ChannelDispatcher(&ctx.commands);

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
    api::handle(request, &mut api_ctx)
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

    let mut out = request.into_response(status, None, &header_list)?;
    if !bytes.is_empty() {
        out.write_all(bytes)?;
    }
    out.flush()?;
    Ok(())
}
