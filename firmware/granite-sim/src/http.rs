//! axum in front of [`granite_core::api::handle`].
//!
//! The handler is deliberately thin, exactly like the ESP-IDF one: parse
//! the request into an [`ApiRequest`], call `handle`, write the
//! [`ApiResponse`] back. No route, no authentication rule and no body
//! cap lives here that does not also live on the board.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Bytes;
use axum::extract::{ConnectInfo, DefaultBodyLimit, OriginalUri, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method as HttpMethod, StatusCode};
use axum::response::Response;
use axum::routing::any;
use granite_core::api::{self, ApiRequest, ApiResponse, Auth, Body, Method, StreamKind};

use crate::assets;
use crate::sim::Sim;

/// Shared state of the server.
pub type Shared = Arc<Mutex<Sim>>;

/// Largest firmware image the simulator accepts in one request.
pub const MAX_IMAGE: usize = 8 * 1024 * 1024;

/// How the simulator should serve.
#[derive(Debug, Clone)]
pub struct ServeOptions {
    /// Port to bind.
    pub port: u16,
    /// Address to bind.
    pub bind: String,
    /// Serve HTTPS with the simulator's certificate.
    pub tls: bool,
    /// Serve the page from this directory instead of the embedded copy,
    /// so the browser picks up edits without a rebuild.
    pub assets_dir: Option<PathBuf>,
}

impl Default for ServeOptions {
    fn default() -> Self {
        ServeOptions {
            port: 8443,
            bind: String::from("127.0.0.1"),
            tls: true,
            assets_dir: None,
        }
    }
}

/// The router: one handler for everything, as on the board.
pub fn app(sim: Shared, assets_dir: Option<PathBuf>) -> Router {
    Router::new()
        .fallback(any(route))
        .layer(DefaultBodyLimit::max(MAX_IMAGE))
        .with_state(AppState { sim, assets_dir })
}

/// What the handler needs.
#[derive(Clone)]
pub struct AppState {
    /// The simulator.
    pub sim: Shared,
    /// Optional on-disk asset directory.
    pub assets_dir: Option<PathBuf>,
}

fn method_of(m: &HttpMethod) -> Method {
    match *m {
        HttpMethod::GET => Method::Get,
        HttpMethod::HEAD => Method::Head,
        HttpMethod::POST => Method::Post,
        HttpMethod::PUT => Method::Put,
        HttpMethod::DELETE => Method::Delete,
        HttpMethod::OPTIONS => Method::Options,
        _ => Method::Other,
    }
}

async fn route(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    method: HttpMethod,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let header = |name: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(String::from)
    };
    let auth = Auth::from_headers(
        header("cookie").as_deref(),
        header("authorization").as_deref(),
    );
    let mut req = ApiRequest {
        method: method_of(&method),
        path: String::from(uri.path()),
        query: String::from(uri.query().unwrap_or("")),
        body: body.to_vec(),
        auth,
        peer: peer.ip().to_string(),
        upload: None,
    };

    let is_upload = req.method == Method::Post
        && api::route_of(&req.path) == Some(api::Route::FirmwareUpload);

    let response = {
        let mut sim = state.sim.lock().expect("the simulator lock is never poisoned");
        if is_upload {
            // Authenticate first, then stream the body into the sink in
            // chunks, the way the ESP-IDF handler has to.
            if !sim.is_authenticated(&req) {
                let probe = ApiRequest {
                    body: Vec::new(),
                    ..req.clone()
                };
                return into_response(&state, sim.handle(probe));
            }
            let outcome = sim.stream_to_ota(&req.body);
            req.body = Vec::new();
            req.upload = Some(outcome);
        }
        sim.handle(req)
    };
    into_response(&state, response)
}

fn into_response(state: &AppState, response: ApiResponse) -> Response {
    let ApiResponse {
        status,
        content_type,
        body,
        set_cookie,
        headers,
    } = response;

    let (status, bytes, content_type) = match body {
        Body::Empty => (status, Vec::new(), ""),
        Body::Bytes(b) => (status, b, content_type),
        Body::Stream(StreamKind::Asset(path)) => match load_asset(state, &path) {
            Some((bytes, ct)) => (status, bytes, ct),
            None => (404, b"not found\n".to_vec(), "text/plain; charset=utf-8"),
        },
    };

    let mut builder = Response::builder()
        .status(StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR));
    if !content_type.is_empty() && !bytes.is_empty() {
        builder = builder.header("content-type", content_type);
    }
    if let Some(cookie) = set_cookie {
        builder = builder.header("set-cookie", cookie);
    }
    for (name, value) in headers {
        if let (Ok(n), Ok(v)) = (
            HeaderName::from_bytes(name.as_bytes()),
            HeaderValue::from_str(&value),
        ) {
            builder = builder.header(n, v);
        }
    }
    // The setup page talks to its own origin only: no framing, no
    // sniffing, no referrer.
    builder
        .header("x-content-type-options", "nosniff")
        .header("x-frame-options", "DENY")
        .header("referrer-policy", "no-referrer")
        .body(axum::body::Body::from(bytes))
        .unwrap_or_else(|_| Response::new(axum::body::Body::empty()))
}

fn load_asset(state: &AppState, path: &str) -> Option<(Vec<u8>, &'static str)> {
    let embedded = assets::asset(path);
    if let Some(dir) = &state.assets_dir {
        let name = path.trim_start_matches('/');
        if !name.is_empty()
            && !name.contains("..")
            && let Ok(bytes) = std::fs::read(dir.join(name))
        {
            let ct = embedded
                .map(|a| a.content_type)
                .unwrap_or("application/octet-stream");
            return Some((bytes, ct));
        }
    }
    embedded.map(|a| (a.bytes.to_vec(), a.content_type))
}

/// Serve until the process is interrupted.
pub async fn serve(sim: Shared, options: ServeOptions) -> anyhow::Result<()> {
    let addr: SocketAddr = format!("{}:{}", options.bind, options.port).parse()?;
    let router = app(sim.clone(), options.assets_dir.clone());
    let service = router.into_make_service_with_connect_info::<SocketAddr>();

    let (scheme, device) = {
        let sim = sim.lock().expect("lock");
        ("https", sim.identity.device_id.clone())
    };
    if options.tls {
        let (cert, key) = {
            let sim = sim.lock().expect("lock");
            (sim.identity.cert_pem.clone(), sim.identity.key_pem.clone())
        };
        let config = axum_server::tls_rustls::RustlsConfig::from_pem(
            cert.into_bytes(),
            key.into_bytes(),
        )
        .await?;
        println!("granite-sim {device} on {scheme}://{addr} (self-signed certificate)");
        axum_server::bind_rustls(addr, config).serve(service).await?;
    } else {
        println!("granite-sim {device} on http://{addr}");
        axum_server::bind(addr).serve(service).await?;
    }
    Ok(())
}
