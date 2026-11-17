//! The one thing in the firmware that talks to the block switch (ADR
//! 0006 component 16).
//!
//! An `EspHttpConnection` per request, the way `platform::ota` pulls an
//! image, plus everything the Sodola CGI demands:
//!
//! - cookie `admin=md5(user + password)`, accepted only after a POST to
//!   `login.cgi`; the switch forgets logins when it reboots, so a page
//!   that comes back as the login redirect triggers one re-login and a
//!   retry,
//! - a `Referer` header on every request, or the CGI answers with an
//!   empty body,
//! - `Content-Length` on every POST: without it esp-idf-svc sends a
//!   chunked body, which this CGI does not read,
//! - one request at a time, with [`MIN_GAP_MS`] between them, because
//!   the switch needs about 400 ms per page and the soak test only ever
//!   had one request outstanding,
//! - a settle wait after a port write (the write applies about a second
//!   later and a second write inside that window is silently dropped),
//! - writes that can cut the answer off (STP, the management address)
//!   are allowed to time out and are verified by reading back.
//!
//! One page is buffered at a time and parsed in place; nothing here
//! keeps a page or a form around. The buffer is not the client's: the
//! worker owns one [`PageBuf`] of [`PAGE_BUF`] bytes, allocated when it
//! starts and reused for every page of both switches, and hands it to
//! every call (ADR 0001 component 27, "Switch page buffer"). Nothing
//! grows: a page longer than the buffer is [`ClientError::TooLarge`].
//!
//! Every request also takes the memory gate ([`crate::platform::mem`])
//! from the connect to the end of its parse, so a page read and a
//! firmware image never want the heap at the same time. A request that
//! cannot have the token within [`REQUEST_BUDGET`] is
//! [`ClientError::Busy`].
//!
//! The client also carries the [`Layout`] the pages are read with (ADR
//! 0006 component 24). It starts as the widest in the profile table,
//! because the layout is not known before `info.cgi` has been read, and
//! the worker narrows it with [`SwitchClient::set_layout`] as soon as
//! the (model, firmware) pair resolves to a row.

use std::thread;
use std::time::Duration;

use esp_idf_svc::http::Method;
use esp_idf_svc::http::client::{Configuration as HttpConfiguration, EspHttpConnection};
use granite_core::switch::forms::{self, Form};
use granite_core::switch::model::{
    IpCfg, Jumbo, LoopCfg, MacEntry, PortCfg, PortStats, PortVlan, RatePort, StormPort, SwitchInfo,
    VlanTable,
};
use granite_core::switch::parse::{self, ParseError, PortLink};
use granite_core::switch::profile::Layout;
use granite_core::switch::{md5, plan::Op};

/// Shortest gap between two requests \[ms\].
pub const MIN_GAP_MS: u32 = 100;

/// Request timeout. The slowest page measured was 460 ms; a reply that
/// is 5 s late means the web server is in trouble.
///
/// It must stay well under `CONFIG_ESP_TASK_WDT_TIMEOUT_S` (10 s): the
/// worker is under the task watchdog and [`SwitchClient::request`]
/// feeds it once per request, so a request may never block for a whole
/// watchdog period. At 10 s it did, and a switch that stalled on one
/// page reset the controller (coredump of 2026-11-15: the worker in
/// `esp_http_client_read` under `read_ports`).
pub const TIMEOUT: Duration = Duration::from_secs(5);

/// Longest one request may take from its first byte out to its last
/// byte in. [`TIMEOUT`] bounds each step (connect, headers, every read
/// of the body) and the watchdog is fed between the steps, so without
/// this a reply that dribbles in would hold the worker for good.
pub const REQUEST_BUDGET: Duration = Duration::from_secs(15);

/// The one page buffer, in bytes (ADR 0001 component 27).
///
/// The largest page of the verified profile is 10.3 KB, so this is that
/// plus half again. It is allocated once, when the worker starts, and
/// every page of both switches is read into it; a longer page is
/// refused as [`ClientError::TooLarge`] and nothing grows.
pub const PAGE_BUF: usize = 16 * 1024;

/// Read buffer. Pages are a few KB, read in these steps.
const CHUNK: usize = 1024;

/// How long the switch takes to come back from a reboot, plus margin.
pub const REBOOT_WAIT: Duration = Duration::from_secs(10);

/// What can go wrong talking to the switch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientError {
    /// The socket, DNS or the HTTP client itself.
    Transport(String),
    /// A status other than 200.
    Status(u16),
    /// The page came back but is not one this firmware knows.
    Page(ParseError),
    /// The switch keeps redirecting to `login.cgi`: the credentials are
    /// wrong.
    Auth,
    /// The page did not fit in the worker's [`PAGE_BUF`] byte buffer.
    TooLarge,
    /// Another heavy user holds the memory gate (ADR 0001 component
    /// 27), so this request was not made at all. Not the switch's
    /// fault and not counted as one: the caller tries again later.
    Busy(&'static str),
}

impl core::fmt::Display for ClientError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ClientError::Transport(m) => write!(f, "{m}"),
            ClientError::Status(s) => write!(f, "http {s}"),
            ClientError::Page(e) => write!(f, "{e}"),
            ClientError::Auth => f.write_str("the switch refused the credentials"),
            ClientError::TooLarge => f.write_str("the page is too large to be one we know"),
            ClientError::Busy(who) => write!(
                f,
                "busy: {} is using the memory a page needs",
                if who.is_empty() { "another task" } else { who }
            ),
        }
    }
}

impl From<ParseError> for ClientError {
    fn from(e: ParseError) -> Self {
        ClientError::Page(e)
    }
}

impl ClientError {
    /// True when this looks like "the switch is not there", which is
    /// what the reachability counter counts.
    pub fn is_unreachable(&self) -> bool {
        matches!(self, ClientError::Transport(_))
    }
}

/// The one page buffer the worker owns (ADR 0001 component 27).
///
/// Fixed capacity, allocated once in [`PageBuf::new`] and never grown.
/// A request fills it, the parser reads it as a `&str`, and the next
/// request overwrites it; no page outlives its parse.
pub struct PageBuf {
    /// The bytes, with [`PAGE_BUF`] of capacity from the start.
    buf: Vec<u8>,
    /// The lossy copy of a page that is not valid UTF-8. Normally
    /// `None`: every page of a switch this firmware knows is ASCII.
    lossy: Option<String>,
}

impl Default for PageBuf {
    fn default() -> Self {
        Self::new()
    }
}

impl PageBuf {
    /// Allocate the buffer. The one allocation of the page path.
    pub fn new() -> Self {
        PageBuf {
            buf: Vec::with_capacity(PAGE_BUF),
            lossy: None,
        }
    }

    /// The page, as the parsers want it.
    pub fn as_str(&self) -> &str {
        match self.lossy.as_deref() {
            Some(lossy) => lossy,
            // Checked in `finish`, which every request calls before it
            // returns; an unchecked buffer would be empty anyway.
            None => core::str::from_utf8(&self.buf).unwrap_or(""),
        }
    }

    /// Bytes in the buffer.
    pub fn len(&self) -> usize {
        self.buf.len()
    }

    /// True while nothing has been read into it.
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    /// Start a new page.
    fn reset(&mut self) {
        self.buf.clear();
        self.lossy = None;
    }

    /// Append what a read returned. [`ClientError::TooLarge`] rather
    /// than a bigger buffer: the capacity is the contract.
    fn push(&mut self, bytes: &[u8]) -> Result<(), ClientError> {
        if self.buf.len() + bytes.len() > PAGE_BUF {
            return Err(ClientError::TooLarge);
        }
        self.buf.extend_from_slice(bytes);
        Ok(())
    }

    /// Validate the page as UTF-8 in place.
    ///
    /// A page that is valid - every real page - is parsed straight out
    /// of the buffer. One that is not gets a lossy copy, sized exactly
    /// and reserved with `try_reserve_exact`, so a page of rubbish is
    /// an error of this request instead of an allocation failure that
    /// aborts the firmware.
    fn finish(&mut self) -> Result<(), ClientError> {
        self.lossy = None;
        if core::str::from_utf8(&self.buf).is_ok() {
            return Ok(());
        }
        let mut need = 0usize;
        for chunk in self.buf.utf8_chunks() {
            need += chunk.valid().len();
            if !chunk.invalid().is_empty() {
                // One replacement character per invalid sequence, the
                // way `String::from_utf8_lossy` counts them.
                need += '\u{FFFD}'.len_utf8();
            }
        }
        let mut lossy = String::new();
        if lossy.try_reserve_exact(need).is_err() {
            return Err(ClientError::Transport(format!(
                "the page is not utf-8 and there is no memory for a copy of {need} bytes"
            )));
        }
        for chunk in self.buf.utf8_chunks() {
            lossy.push_str(chunk.valid());
            if !chunk.invalid().is_empty() {
                lossy.push('\u{FFFD}');
            }
        }
        log::warn!(
            "switch: a page of {} bytes is not valid utf-8; parsing a lossy copy",
            self.buf.len()
        );
        self.lossy = Some(lossy);
        Ok(())
    }
}

/// Take the memory gate for one request and its parse (ADR 0001
/// component 27). Dropped by the caller when the parse is done.
fn heavy() -> Result<crate::platform::mem::Heavy, ClientError> {
    crate::platform::mem::heavy("switch request", REQUEST_BUDGET)
        .ok_or_else(|| ClientError::Busy(crate::platform::mem::heavy_holder()))
}

/// A serial, spaced, re-logging-in HTTP client for one switch.
pub struct SwitchClient {
    base: String,
    user: String,
    password: String,
    token: String,
    logged_in: bool,
    next_ms: u64,
    /// Requests sent since the client was built, for the log.
    requests: u32,
    /// The layout the per-port pages are read with: the resolved
    /// profile's, or the widest in the table while none is resolved.
    layout: &'static Layout,
}

impl SwitchClient {
    /// A client for `host` with these credentials. Nothing is sent yet.
    pub fn new(host: &str, user: &str, password: &str) -> Self {
        SwitchClient {
            base: format!("http://{}/", host.trim()),
            user: String::from(user),
            password: String::from(password),
            token: md5::session_token(user, password),
            logged_in: false,
            next_ms: 0,
            requests: 0,
            layout: Layout::widest(),
        }
    }

    /// Read the per-port pages with this layout from now on: the
    /// resolved profile's, or the widest in the table for a switch
    /// without a row.
    pub fn set_layout(&mut self, layout: &'static Layout) {
        self.layout = layout;
    }

    /// The layout the pages are currently read with.
    pub fn layout(&self) -> &'static Layout {
        self.layout
    }

    /// True when this client is for exactly this host and login, which
    /// is how the worker notices a configuration change.
    pub fn matches(&self, host: &str, user: &str, password: &str) -> bool {
        self.base == format!("http://{}/", host.trim())
            && self.user == user
            && self.password == password
    }

    /// The host this client talks to.
    pub fn host(&self) -> &str {
        self.base
            .trim_start_matches("http://")
            .trim_end_matches('/')
    }

    /// Follow the switch to another address, as an IP change needs.
    pub fn set_host(&mut self, host: &str) {
        self.base = format!("http://{}/", host.trim());
        // A different address is a different session as far as this
        // client is concerned.
        self.logged_in = false;
    }

    /// Requests sent so far.
    pub const fn requests(&self) -> u32 {
        self.requests
    }

    /// Forget the session, so the next request logs in first.
    pub fn forget_session(&mut self) {
        self.logged_in = false;
    }

    // -----------------------------------------------------------------
    // The transport
    // -----------------------------------------------------------------

    /// GET one page into `page`.
    pub fn get(&mut self, path: &str, page: &mut PageBuf) -> Result<(), ClientError> {
        self.authed(Method::Get, path, None, page)
    }

    /// POST one form; the page that comes back lands in `page`.
    pub fn post(&mut self, path: &str, form: &Form, page: &mut PageBuf) -> Result<(), ClientError> {
        let body = forms::encode(form);
        self.authed(Method::Post, path, Some(body.as_bytes()), page)
    }

    /// POST one form, accepting a timeout as "it applied and the answer
    /// was cut off" (`docs/sodola.md`: an STP or IP change does that).
    /// `Ok(false)` is that case; the caller verifies by reading back.
    pub fn post_may_timeout(
        &mut self,
        path: &str,
        form: &Form,
        page: &mut PageBuf,
    ) -> Result<bool, ClientError> {
        match self.post(path, form, page) {
            Ok(()) => Ok(true),
            Err(ClientError::Transport(e)) => {
                log::warn!(
                    "switch: no reply to the POST of {path} ({e}); the change usually applied, \
                     verifying by reading back"
                );
                Ok(false)
            }
            Err(other) => Err(other),
        }
    }

    /// One request, logging in once if the switch has forgotten us.
    fn authed(
        &mut self,
        method: Method,
        path: &str,
        body: Option<&[u8]>,
        page: &mut PageBuf,
    ) -> Result<(), ClientError> {
        self.request(method, path, body, page)?;
        if !granite_core::switch::scan::is_login_redirect(page.as_str()) {
            return Ok(());
        }
        if self.logged_in {
            // We logged in during this very attempt and still get the
            // redirect: the credentials are not accepted.
            self.logged_in = false;
            return Err(ClientError::Auth);
        }
        self.login(page)?;
        self.request(method, path, body, page)?;
        if granite_core::switch::scan::is_login_redirect(page.as_str()) {
            self.logged_in = false;
            return Err(ClientError::Auth);
        }
        Ok(())
    }

    /// The login POST that makes the switch accept the cookie. Uses the
    /// same buffer: its answer is read, checked and thrown away.
    fn login(&mut self, page: &mut PageBuf) -> Result<(), ClientError> {
        let form = forms::login(&self.user, &self.password, &self.token);
        let body = forms::encode(&form);
        self.request(Method::Post, "login.cgi", Some(body.as_bytes()), page)?;
        // The CGI answers the successful login with a page whose script
        // replaces the location with "/".
        let answer = page.as_str();
        if !answer.contains("replace(\"/\")") && !answer.contains("replace('/')") {
            return Err(ClientError::Auth);
        }
        self.logged_in = true;
        log::info!("switch: logged in to {} as {}", self.host(), self.user);
        Ok(())
    }

    /// One HTTP request, spaced from the previous one.
    fn request(
        &mut self,
        method: Method,
        path: &str,
        body: Option<&[u8]>,
        page: &mut PageBuf,
    ) -> Result<(), ClientError> {
        self.space();
        // Fed per step of a request, not per pass of the worker loop: a
        // pass reads several pages in a row, and every step of each
        // (connect, headers, a read of the body) may take up to
        // `TIMEOUT`.
        crate::platform::feed_task_watchdog();
        let started = crate::platform::now_ms();
        let url = format!("{}{}", self.base, path.trim_start_matches('/'));
        let cookie = format!("admin={}", self.token);
        let length = body.map(|b| b.len().to_string());

        let mut conn = EspHttpConnection::new(&HttpConfiguration {
            buffer_size: Some(CHUNK),
            buffer_size_tx: Some(CHUNK),
            timeout: Some(TIMEOUT),
            // The CGI never redirects with a status code; it answers 200
            // with a script that replaces the location.
            follow_redirects_policy: esp_idf_svc::http::client::FollowRedirectsPolicy::FollowNone,
            ..Default::default()
        })
        .map_err(|e| ClientError::Transport(format!("http client: {e}")))?;

        let mut headers: Vec<(&str, &str)> = alloc_headers(&cookie, &self.base);
        if let Some(len) = length.as_deref() {
            headers.push(("Content-Type", "application/x-www-form-urlencoded"));
            // Without this esp-idf-svc sends a chunked body, which this
            // CGI does not read.
            headers.push(("Content-Length", len));
        }

        conn.initiate_request(method, &url, &headers)
            .map_err(|e| ClientError::Transport(format!("request {path}: {e}")))?;
        if let Some(body) = body {
            conn.write_all(body)
                .map_err(|e| ClientError::Transport(format!("body of {path}: {e}")))?;
        }
        crate::platform::feed_task_watchdog();
        conn.initiate_response()
            .map_err(|e| ClientError::Transport(format!("response of {path}: {e}")))?;
        self.requests = self.requests.saturating_add(1);
        let status = conn.status();
        if status != 200 {
            return Err(ClientError::Status(status));
        }

        page.reset();
        let mut buf = [0u8; CHUNK];
        loop {
            crate::platform::feed_task_watchdog();
            if crate::platform::now_ms() - started > REQUEST_BUDGET.as_millis() as u64 {
                return Err(ClientError::Transport(format!(
                    "read of {path}: not finished after {} s",
                    REQUEST_BUDGET.as_secs()
                )));
            }
            match conn.read(&mut buf) {
                Ok(0) => break,
                // Into the worker's fixed buffer: nothing here
                // allocates, and a page that does not fit is not one
                // this firmware knows (ADR 0001 component 27).
                Ok(n) => page.push(&buf[..n])?,
                Err(e) => {
                    return Err(ClientError::Transport(format!("read of {path}: {e}")));
                }
            }
        }
        page.finish()
    }

    /// Hold the minimum gap between two requests.
    fn space(&mut self) {
        let now = crate::platform::now_ms();
        if now < self.next_ms {
            thread::sleep(Duration::from_millis(self.next_ms - now));
        }
        self.next_ms = crate::platform::now_ms() + u64::from(MIN_GAP_MS);
    }

    /// Make the next request wait `ms` longer: the settle time after a
    /// port write.
    pub fn settle(&mut self, ms: u32) {
        if ms == 0 {
            return;
        }
        self.next_ms = crate::platform::now_ms() + u64::from(ms);
    }

    // -----------------------------------------------------------------
    // The pages, as the worker wants them
    // -----------------------------------------------------------------

    // Every one of these holds the memory gate for the request and the
    // parse, and gives it back when it returns (ADR 0001 component
    // 27). The parse reads the page out of the worker's buffer, so no
    // page outlives it.

    /// `info.cgi`: the device facts and the link state.
    pub fn read_info(
        &mut self,
        page: &mut PageBuf,
    ) -> Result<(SwitchInfo, Vec<PortLink>), ClientError> {
        let _heavy = heavy()?;
        self.get("info.cgi", page)?;
        Ok(parse::info(page.as_str())?)
    }

    /// `port.cgi`: the per-port configuration.
    pub fn read_ports(&mut self, page: &mut PageBuf) -> Result<Vec<PortCfg>, ClientError> {
        let _heavy = heavy()?;
        self.get("port.cgi", page)?;
        Ok(parse::port_config(self.layout, page.as_str())?)
    }

    /// `port.cgi?page=stats`: link state and the counters.
    pub fn read_stats(&mut self, page: &mut PageBuf) -> Result<Vec<PortStats>, ClientError> {
        let _heavy = heavy()?;
        self.get("port.cgi?page=stats", page)?;
        Ok(parse::stats(self.layout, page.as_str())?)
    }

    /// `vlan.cgi?page=static`.
    pub fn read_vlans(&mut self, page: &mut PageBuf) -> Result<VlanTable, ClientError> {
        let _heavy = heavy()?;
        self.get("vlan.cgi?page=static", page)?;
        Ok(parse::vlans(page.as_str())?)
    }

    /// `vlan.cgi?page=port_based`.
    pub fn read_port_vlans(&mut self, page: &mut PageBuf) -> Result<Vec<PortVlan>, ClientError> {
        let _heavy = heavy()?;
        self.get("vlan.cgi?page=port_based", page)?;
        Ok(parse::port_vlans(self.layout, page.as_str())?)
    }

    /// `fwd.cgi?page=jumboframe`.
    pub fn read_jumbo(&mut self, page: &mut PageBuf) -> Result<Jumbo, ClientError> {
        let _heavy = heavy()?;
        self.get("fwd.cgi?page=jumboframe", page)?;
        Ok(parse::jumbo(page.as_str())?)
    }

    /// `fwd.cgi?page=storm_ctrl`.
    pub fn read_storm(&mut self, page: &mut PageBuf) -> Result<Vec<StormPort>, ClientError> {
        let _heavy = heavy()?;
        self.get("fwd.cgi?page=storm_ctrl", page)?;
        Ok(parse::storm(self.layout, page.as_str())?)
    }

    /// `port.cgi?page=bw_ctrl`.
    pub fn read_rate_limits(&mut self, page: &mut PageBuf) -> Result<Vec<RatePort>, ClientError> {
        let _heavy = heavy()?;
        self.get("port.cgi?page=bw_ctrl", page)?;
        Ok(parse::rate_limits(self.layout, page.as_str())?)
    }

    /// `loop.cgi`.
    pub fn read_loop(&mut self, page: &mut PageBuf) -> Result<LoopCfg, ClientError> {
        let _heavy = heavy()?;
        self.get("loop.cgi", page)?;
        Ok(parse::loop_cfg(page.as_str())?)
    }

    /// `ip.cgi`.
    pub fn read_ip(&mut self, page: &mut PageBuf) -> Result<IpCfg, ClientError> {
        let _heavy = heavy()?;
        self.get("ip.cgi", page)?;
        Ok(parse::ip(page.as_str())?)
    }

    /// The whole MAC table, 30 rows at a time.
    ///
    /// The paging and the search filter are server-side state shared
    /// with any browser on the web UI, so the walk GETs the page first
    /// (which drops a stuck filter) and then pages through it. Those
    /// POSTs change the table's view and no configuration, which is why
    /// they are allowed without `switch.manage`.
    /// The gate is taken per page of the walk, not for the whole walk:
    /// ten pages are four seconds, and an update waiting for the token
    /// should not wait that long.
    pub fn walk_macs(
        &mut self,
        max_pages: u32,
        page: &mut PageBuf,
    ) -> Result<Vec<MacEntry>, ClientError> {
        const PATH: &str = "mac.cgi?page=fwd_tbl";
        let (mut entries, pages) = {
            let _heavy = heavy()?;
            self.get(PATH, page)?;
            self.post(PATH, &forms::mac_perpage(), page)?;
            let first = parse::mac_page(self.layout, page.as_str())?;
            (first.entries, first.pages.min(max_pages))
        };
        for index in 2..=pages {
            let _heavy = heavy()?;
            self.post(PATH, &forms::mac_goto(index), page)?;
            let more = parse::mac_page(self.layout, page.as_str())?;
            entries.extend(more.entries);
        }
        entries.sort_by_key(|e| (e.port, e.vlan, e.mac.clone()));
        entries.dedup();
        Ok(entries)
    }

    // -----------------------------------------------------------------
    // Writes
    // -----------------------------------------------------------------

    /// Carry out one planned op, honouring its settle time and whether
    /// it is allowed to go unanswered.
    pub fn apply(&mut self, op: &Op, page: &mut PageBuf) -> Result<(), ClientError> {
        let _heavy = heavy()?;
        log::warn!("switch: applying {}", op.detail);
        if op.may_timeout {
            self.post_may_timeout(&op.path, &op.form, page)?;
        } else {
            self.post(&op.path, &op.form, page)?;
        }
        self.settle(op.settle_ms);
        Ok(())
    }

    /// `save.cgi`: the running configuration to flash. The one write
    /// that must never happen before a confirm (ADR 0006 component 18).
    pub fn save(&mut self, page: &mut PageBuf) -> Result<(), ClientError> {
        let _heavy = heavy()?;
        self.post("save.cgi", &forms::save(), page)?;
        log::warn!("switch: running configuration saved to flash");
        Ok(())
    }

    /// `reboot.cgi`: restart the switch, dropping everything unsaved.
    /// The reply is usually cut off by the reboot itself.
    pub fn reboot(&mut self, page: &mut PageBuf) -> Result<(), ClientError> {
        let _heavy = heavy()?;
        self.post_may_timeout("reboot.cgi", &forms::reboot(), page)?;
        // A rebooted switch has forgotten every login.
        self.logged_in = false;
        log::warn!(
            "switch: rebooting {}; unsaved changes are gone and every port is down for about 3 s",
            self.host()
        );
        Ok(())
    }

    /// `reset.cgi`: factory default. The switch comes back on
    /// 192.168.2.1 with admin/admin and is then unreachable.
    pub fn factory_reset(&mut self, page: &mut PageBuf) -> Result<(), ClientError> {
        let _heavy = heavy()?;
        self.post_may_timeout("reset.cgi", &forms::factory_reset(), page)?;
        self.logged_in = false;
        log::warn!(
            "switch: factory reset sent to {}; it comes back on 192.168.2.1 with admin/admin",
            self.host()
        );
        Ok(())
    }
}

/// The two headers every request carries. A `Referer` is not optional:
/// without it the CGI answers with an empty body.
fn alloc_headers<'a>(cookie: &'a str, base: &'a str) -> Vec<(&'a str, &'a str)> {
    vec![("Cookie", cookie), ("Referer", base)]
}
