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
//! One page is buffered at a time and dropped after its parse; nothing
//! here keeps a page or a form around.

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
use granite_core::switch::{md5, plan::Op};

/// Shortest gap between two requests \[ms\].
pub const MIN_GAP_MS: u32 = 100;

/// Request timeout. The slowest page measured was 460 ms; a reply that
/// is 10 s late means the web server is in trouble.
pub const TIMEOUT: Duration = Duration::from_secs(10);

/// Longest page the client will buffer. The biggest page used is about
/// 10 KB; anything much larger is not a page this firmware knows.
const MAX_PAGE: usize = 48 * 1024;

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
    /// The page was longer than [`MAX_PAGE`].
    TooLarge,
}

impl core::fmt::Display for ClientError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ClientError::Transport(m) => write!(f, "{m}"),
            ClientError::Status(s) => write!(f, "http {s}"),
            ClientError::Page(e) => write!(f, "{e}"),
            ClientError::Auth => f.write_str("the switch refused the credentials"),
            ClientError::TooLarge => f.write_str("the page is too large to be one we know"),
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
        }
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

    /// GET one page.
    pub fn get(&mut self, path: &str) -> Result<String, ClientError> {
        self.authed(Method::Get, path, None)
    }

    /// POST one form and return the page that comes back.
    pub fn post(&mut self, path: &str, form: &Form) -> Result<String, ClientError> {
        let body = forms::encode(form);
        self.authed(Method::Post, path, Some(body.as_bytes()))
    }

    /// POST one form, accepting a timeout as "it applied and the answer
    /// was cut off" (`docs/sodola.md`: an STP or IP change does that).
    /// `Ok(None)` is that case; the caller verifies by reading back.
    pub fn post_may_timeout(
        &mut self,
        path: &str,
        form: &Form,
    ) -> Result<Option<String>, ClientError> {
        match self.post(path, form) {
            Ok(page) => Ok(Some(page)),
            Err(ClientError::Transport(e)) => {
                log::warn!(
                    "switch: no reply to the POST of {path} ({e}); the change usually applied, \
                     verifying by reading back"
                );
                Ok(None)
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
    ) -> Result<String, ClientError> {
        let page = self.request(method, path, body)?;
        if !granite_core::switch::scan::is_login_redirect(&page) {
            return Ok(page);
        }
        if self.logged_in {
            // We logged in during this very attempt and still get the
            // redirect: the credentials are not accepted.
            self.logged_in = false;
            return Err(ClientError::Auth);
        }
        self.login()?;
        let page = self.request(method, path, body)?;
        if granite_core::switch::scan::is_login_redirect(&page) {
            self.logged_in = false;
            return Err(ClientError::Auth);
        }
        Ok(page)
    }

    /// The login POST that makes the switch accept the cookie.
    fn login(&mut self) -> Result<(), ClientError> {
        let form = forms::login(&self.user, &self.password, &self.token);
        let body = forms::encode(&form);
        let page = self.request(Method::Post, "login.cgi", Some(body.as_bytes()))?;
        // The CGI answers the successful login with a page whose script
        // replaces the location with "/".
        if !page.contains("replace(\"/\")") && !page.contains("replace('/')") {
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
    ) -> Result<String, ClientError> {
        self.space();
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
        conn.initiate_response()
            .map_err(|e| ClientError::Transport(format!("response of {path}: {e}")))?;
        self.requests = self.requests.saturating_add(1);
        let status = conn.status();
        if status != 200 {
            return Err(ClientError::Status(status));
        }

        let mut page = String::new();
        let mut buf = [0u8; CHUNK];
        loop {
            match conn.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if page.len() + n > MAX_PAGE {
                        return Err(ClientError::TooLarge);
                    }
                    page.push_str(&String::from_utf8_lossy(&buf[..n]));
                }
                Err(e) => {
                    return Err(ClientError::Transport(format!("read of {path}: {e}")));
                }
            }
        }
        Ok(page)
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

    /// `info.cgi`: the device facts and the link state.
    pub fn read_info(&mut self) -> Result<(SwitchInfo, Vec<PortLink>), ClientError> {
        let page = self.get("info.cgi")?;
        Ok(parse::info(&page)?)
    }

    /// `port.cgi`: the per-port configuration.
    pub fn read_ports(&mut self) -> Result<Vec<PortCfg>, ClientError> {
        let page = self.get("port.cgi")?;
        Ok(parse::port_config(&page)?)
    }

    /// `port.cgi?page=stats`: link state and the counters.
    pub fn read_stats(&mut self) -> Result<Vec<PortStats>, ClientError> {
        let page = self.get("port.cgi?page=stats")?;
        Ok(parse::stats(&page)?)
    }

    /// `vlan.cgi?page=static`.
    pub fn read_vlans(&mut self) -> Result<VlanTable, ClientError> {
        let page = self.get("vlan.cgi?page=static")?;
        Ok(parse::vlans(&page)?)
    }

    /// `vlan.cgi?page=port_based`.
    pub fn read_port_vlans(&mut self) -> Result<Vec<PortVlan>, ClientError> {
        let page = self.get("vlan.cgi?page=port_based")?;
        Ok(parse::port_vlans(&page)?)
    }

    /// `fwd.cgi?page=jumboframe`.
    pub fn read_jumbo(&mut self) -> Result<Jumbo, ClientError> {
        let page = self.get("fwd.cgi?page=jumboframe")?;
        Ok(parse::jumbo(&page)?)
    }

    /// `fwd.cgi?page=storm_ctrl`.
    pub fn read_storm(&mut self) -> Result<Vec<StormPort>, ClientError> {
        let page = self.get("fwd.cgi?page=storm_ctrl")?;
        Ok(parse::storm(&page)?)
    }

    /// `port.cgi?page=bw_ctrl`.
    pub fn read_rate_limits(&mut self) -> Result<Vec<RatePort>, ClientError> {
        let page = self.get("port.cgi?page=bw_ctrl")?;
        Ok(parse::rate_limits(&page)?)
    }

    /// `loop.cgi`.
    pub fn read_loop(&mut self) -> Result<LoopCfg, ClientError> {
        let page = self.get("loop.cgi")?;
        Ok(parse::loop_cfg(&page)?)
    }

    /// `ip.cgi`.
    pub fn read_ip(&mut self) -> Result<IpCfg, ClientError> {
        let page = self.get("ip.cgi")?;
        Ok(parse::ip(&page)?)
    }

    /// The whole MAC table, 30 rows at a time.
    ///
    /// The paging and the search filter are server-side state shared
    /// with any browser on the web UI, so the walk GETs the page first
    /// (which drops a stuck filter) and then pages through it. Those
    /// POSTs change the table's view and no configuration, which is why
    /// they are allowed without `switch.manage`.
    pub fn walk_macs(&mut self, max_pages: u32) -> Result<Vec<MacEntry>, ClientError> {
        const PATH: &str = "mac.cgi?page=fwd_tbl";
        self.get(PATH)?;
        let first = self.post(PATH, &forms::mac_perpage())?;
        let page = parse::mac_page(&first)?;
        let mut entries = page.entries;
        let pages = page.pages.min(max_pages);
        for index in 2..=pages {
            let body = self.post(PATH, &forms::mac_goto(index))?;
            let more = parse::mac_page(&body)?;
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
    pub fn apply(&mut self, op: &Op) -> Result<(), ClientError> {
        log::warn!("switch: applying {}", op.detail);
        if op.may_timeout {
            self.post_may_timeout(&op.path, &op.form)?;
        } else {
            self.post(&op.path, &op.form)?;
        }
        self.settle(op.settle_ms);
        Ok(())
    }

    /// `save.cgi`: the running configuration to flash. The one write
    /// that must never happen before a confirm (ADR 0006 component 18).
    pub fn save(&mut self) -> Result<(), ClientError> {
        self.post("save.cgi", &forms::save())?;
        log::warn!("switch: running configuration saved to flash");
        Ok(())
    }

    /// `reboot.cgi`: restart the switch, dropping everything unsaved.
    /// The reply is usually cut off by the reboot itself.
    pub fn reboot(&mut self) -> Result<(), ClientError> {
        self.post_may_timeout("reboot.cgi", &forms::reboot())?;
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
    pub fn factory_reset(&mut self) -> Result<(), ClientError> {
        self.post_may_timeout("reset.cgi", &forms::factory_reset())?;
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
