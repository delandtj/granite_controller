//! The first-boot flow end to end over HTTP: set the admin password, log
//! in, power a node on, watch the host behind the plane slot answer and
//! the node state change. This is the regression test for the whole
//! stack: axum -> granite_core::api -> dispatch -> the plane client ->
//! the plane link -> a simulated plane running plane-logic.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use granite_sim::{Scenario, Sim};

struct Server {
    base: String,
    client: reqwest::Client,
    cookie: Option<String>,
    sim: Arc<Mutex<Sim>>,
}

impl Server {
    async fn start() -> Server {
        let mut scenario = Scenario::default();
        scenario.normalise();
        let sim = Arc::new(Mutex::new(Sim::new(
            scenario,
            "-----BEGIN CERTIFICATE-----\ntest\n-----END CERTIFICATE-----\n",
            "-----BEGIN PRIVATE KEY-----\ntest\n-----END PRIVATE KEY-----\n",
        )));
        let app = granite_sim::http::app(sim.clone(), None);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await
            .unwrap();
        });
        Server {
            base: format!("http://{addr}"),
            client: reqwest::Client::new(),
            cookie: None,
            sim,
        }
    }

    async fn call(
        &mut self,
        method: reqwest::Method,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> (u16, String) {
        let mut req = self.client.request(method, format!("{}{path}", self.base));
        if let Some(cookie) = &self.cookie {
            req = req.header("cookie", cookie);
        }
        if let Some(body) = body {
            req = req.json(&body);
        }
        let response = req.send().await.unwrap();
        let status = response.status().as_u16();
        if let Some(set) = response.headers().get("set-cookie")
            && let Ok(value) = set.to_str()
        {
            let pair = value.split(';').next().unwrap_or("").to_string();
            if pair.ends_with('=') {
                self.cookie = None;
            } else {
                self.cookie = Some(pair);
            }
        }
        (status, response.text().await.unwrap())
    }

    async fn get(&mut self, path: &str) -> (u16, serde_json::Value) {
        let (status, text) = self.call(reqwest::Method::GET, path, None).await;
        (status, parse(&text))
    }

    async fn post(&mut self, path: &str, body: serde_json::Value) -> (u16, serde_json::Value) {
        let (status, text) = self.call(reqwest::Method::POST, path, Some(body)).await;
        (status, parse(&text))
    }
}

fn parse(text: &str) -> serde_json::Value {
    serde_json::from_str(text).unwrap_or(serde_json::Value::String(String::from(text)))
}

#[tokio::test(flavor = "multi_thread")]
async fn first_boot_then_power_a_node_on() {
    let mut s = Server::start().await;

    // The page itself is served.
    let (status, body) = s.call(reqwest::Method::GET, "/", None).await;
    assert_eq!(status, 200);
    assert!(body.contains("Granite controller"));
    assert!(body.contains("/app.js"));

    // /id is unauthenticated and says first setup is pending.
    let (status, id) = s.get("/id").await;
    assert_eq!(status, 200);
    let device = id["device"].as_str().unwrap().to_string();
    assert_eq!(id["password_set"], false);
    assert!(id["nonce"].as_str().unwrap().len() >= 32);

    // Nothing else works yet.
    let (status, _) = s.get("/api/v1/status").await;
    assert_eq!(status, 403);

    // Set the password; the recovery token comes back exactly once.
    let (status, set) = s
        .post(
            "/api/v1/security/password",
            serde_json::json!({"password": "correct horse battery"}),
        )
        .await;
    assert_eq!(status, 200, "{set}");
    let token = set["recovery_token"].as_str().unwrap().to_string();
    assert!(token.len() >= 20, "token {token}");
    let (status, again) = s
        .post(
            "/api/v1/security/password",
            serde_json::json!({"password": "something else"}),
        )
        .await;
    assert_eq!(status, 401, "{again}");

    // Log in.
    let (status, login) = s
        .post(
            "/api/v1/session",
            serde_json::json!({"password": "correct horse battery"}),
        )
        .await;
    assert_eq!(status, 200, "{login}");
    assert!(s.cookie.is_some());

    // The status page now answers, with the node table.
    let (status, st) = s.get("/api/v1/status").await;
    assert_eq!(status, 200);
    assert_eq!(st["device"], device.as_str());
    // The default scenario is one plane with a module in every slot:
    // nodes 1-8 (ADR 0004 decision 2).
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut nodes = 0;
    while Instant::now() < deadline {
        let (_, st) = s.get("/api/v1/state").await;
        nodes = st["nodes"].as_array().map(|a| a.len()).unwrap_or(0);
        if nodes == 8 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(nodes, 8, "the plane's eight slots never showed up");
    let (_, st) = s.get("/api/v1/status").await;
    assert_eq!(st["state"]["planes"].as_array().unwrap().len(), 1);
    assert_eq!(st["state"]["planes"][0]["address"], 48);
    // The rail is up (a plane comes up with its slots enabled) and the
    // host behind it is dark: exactly the case `on` has to press.
    let before = st["state"]["nodes"][2]["host"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        before == "off" || before == "unknown",
        "the host behind node 3 starts dark, got {before}"
    );

    // Power node 3 on. The plane runs the sequence: rail on, and a PWR
    // press if the host stays dark.
    let (status, reply) = s.post("/api/v1/nodes/3/on", serde_json::json!({})).await;
    assert_eq!(status, 200, "{reply}");
    assert_eq!(reply["ok"], true);
    assert_eq!(reply["result"], "accepted", "a plane job was queued");
    assert!(reply["job"].is_number(), "with its job id");

    let deadline = Instant::now() + Duration::from_secs(30);
    let mut power = String::new();
    let mut host = String::new();
    while Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(100)).await;
        let (_, st) = s.get("/api/v1/state").await;
        power = st["nodes"][2]["power"].as_str().unwrap_or("").to_string();
        host = st["nodes"][2]["host"].as_str().unwrap_or("").to_string();
        if power == "on" && host == "on" {
            break;
        }
    }
    assert_eq!(power, "on", "node 3's rail never came up");
    assert_eq!(host, "on", "the host behind node 3 never lit its LED");

    // The press really went to the plane: its own fake host is running,
    // and the slot draws current.
    {
        let mut sim = s.sim.lock().unwrap();
        let plane = sim.planes_mut().plane_mut(0x30).expect("plane 0x30");
        let slot = plane.slot_mut(2).expect("slot 2");
        assert!(slot.node.on, "the host behind slot 2 should be running");
        assert!(slot.pgd, "and its rail should carry it");
    }

    // An action event was recorded.
    let (status, log) = s
        .call(reqwest::Method::GET, "/api/v1/log/tail?lines=200", None)
        .await;
    assert_eq!(status, 200);
    assert!(log.contains("action_done"), "log was: {log}");

    // The job's ack came back on the simulator's ack topic.
    {
        let sim = s.sim.lock().unwrap();
        let acks = &sim.core.acks;
        assert!(
            acks.iter()
                .any(|r| r.ok && r.result.as_deref() == Some("done")),
            "no finished job ack: {acks:?}"
        );
    }

    // The same command through the raw MQTT-shaped route, on node 4.
    let (status, reply) = s
        .post(
            "/api/v1/cmd",
            serde_json::json!({"v":1,"id":"t-1","action":"on","target":4}),
        )
        .await;
    assert_eq!(status, 200, "{reply}");
    assert_eq!(reply["id"], "t-1");

    // And a staged network change shows up on the status page.
    let (status, staged) = s
        .call(
            reqwest::Method::PUT,
            "/api/v1/config/net",
            Some(serde_json::json!({"hostname": "granite-lab"})),
        )
        .await;
    assert_eq!(status, 200, "{staged}");
    let (_, st) = s.get("/api/v1/status").await;
    assert_eq!(st["staged"]["sections"][0], "net");
    let (status, _) = s
        .post("/api/v1/config/confirm", serde_json::json!({}))
        .await;
    assert_eq!(status, 200);
    let (_, st) = s.get("/api/v1/status").await;
    assert!(st["staged"].is_null());

    // Logging out invalidates the cookie.
    let (status, _) = s
        .call(reqwest::Method::DELETE, "/api/v1/session", None)
        .await;
    assert_eq!(status, 200);
    let (status, _) = s.get("/api/v1/status").await;
    assert_eq!(status, 401);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_fleet_recovery_round_trip_works_over_http() {
    let mut s = Server::start().await;
    let (private_pem, public_pem) = granite_sim::keys::generate();

    // Commission: password, then the fleet key.
    let (status, _) = s
        .post(
            "/api/v1/security/password",
            serde_json::json!({"password": "correct horse battery"}),
        )
        .await;
    assert_eq!(status, 200);
    let (status, _) = s
        .post(
            "/api/v1/session",
            serde_json::json!({"password": "correct horse battery"}),
        )
        .await;
    assert_eq!(status, 200);
    let (status, body) = s
        .call(
            reqwest::Method::PUT,
            "/api/v1/security/fleet-key",
            Some(serde_json::json!({"pem": public_pem})),
        )
        .await;
    assert_eq!(status, 200, "{body}");

    // Recovery, the way `granite-sim recover` does it.
    let (_, id) = s.get("/id").await;
    let device = id["device"].as_str().unwrap().to_string();
    let nonce = id["nonce"].as_str().unwrap().to_string();
    let key = {
        use p256::pkcs8::DecodePrivateKey as _;
        let secret = p256::SecretKey::from_pkcs8_pem(&private_pem).unwrap();
        p256::ecdsa::SigningKey::from(&secret)
    };
    let sig = granite_sim::keys::sign_recovery(&key, &device, &nonce);
    let (status, body) = s
        .post(
            "/recover",
            serde_json::json!({"device": device, "nonce": nonce, "sig": sig}),
        )
        .await;
    assert_eq!(status, 200, "{body}");

    // The board is back in first-setup mode and the session is gone.
    let (_, id) = s.get("/id").await;
    assert_eq!(id["password_set"], false);
    let (status, _) = s.get("/api/v1/status").await;
    assert_eq!(status, 403);
}
