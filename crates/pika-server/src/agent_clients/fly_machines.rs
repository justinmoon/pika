use std::collections::HashMap;
use std::fmt;

use anyhow::Context;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};

pub struct FlyClient {
    client: reqwest::Client,
    api_token: String,
    app_name: String,
    api_base_url: String,
    region: String,
    image: String,
}

const DEFAULT_FLY_API_BASE_URL: &str = "https://api.machines.dev";

#[derive(Debug, Serialize)]
struct CreateVolumeRequest {
    name: String,
    region: String,
    size_gb: u32,
}

#[derive(Debug, Deserialize)]
pub struct Volume {
    pub id: String,
}

#[derive(Debug, Serialize)]
struct CreateMachineRequest {
    name: String,
    region: String,
    config: MachineConfig,
}

#[derive(Debug, Serialize)]
struct MachineConfig {
    image: String,
    env: HashMap<String, String>,
    guest: GuestConfig,
    mounts: Vec<MachineMount>,
}

#[derive(Debug, Serialize)]
struct GuestConfig {
    cpu_kind: String,
    cpus: u32,
    memory_mb: u32,
}

#[derive(Debug, Serialize)]
struct MachineMount {
    volume: String,
    path: String,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct Machine {
    pub id: String,
    #[serde(default)]
    pub state: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StopMachineOutcome {
    Stopped,
    AlreadyGone,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeleteMachineOutcome {
    Deleted,
    AlreadyGone,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeleteVolumeOutcome {
    Deleted,
    AlreadyGone,
    Conflict,
}

#[derive(Debug)]
pub struct FlyApiError {
    operation: &'static str,
    status_code: Option<u16>,
    detail: String,
}

impl FlyApiError {
    fn transport(operation: &'static str, err: reqwest::Error) -> Self {
        Self {
            operation,
            status_code: None,
            detail: err.to_string(),
        }
    }

    fn provider(operation: &'static str, status: StatusCode, detail: String) -> Self {
        Self {
            operation,
            status_code: Some(status.as_u16()),
            detail,
        }
    }

    pub fn status_code(&self) -> Option<u16> {
        self.status_code
    }

    pub fn is_retryable(&self) -> bool {
        let Some(status_code) = self.status_code else {
            return true;
        };
        matches!(status_code, 408 | 409 | 422 | 425 | 429) || status_code >= 500
    }
}

impl fmt::Display for FlyApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.status_code {
            Some(status) => write!(
                f,
                "{} failed: status={} detail={}",
                self.operation, status, self.detail
            ),
            None => write!(
                f,
                "{} failed: transport detail={}",
                self.operation, self.detail
            ),
        }
    }
}

impl std::error::Error for FlyApiError {}

impl FlyClient {
    pub fn from_env() -> anyhow::Result<Self> {
        let app_name = optional_non_empty_env("FLY_BOT_APP_NAME", "pika-bot");
        Self::from_env_with_app_name(&app_name)
    }

    pub fn from_env_with_app_name(app_name: &str) -> anyhow::Result<Self> {
        let api_token = required_non_empty_env("FLY_API_TOKEN")
            .context("FLY_API_TOKEN must be set (for example in .env)")?;
        let app_name = app_name.trim();
        if app_name.is_empty() {
            anyhow::bail!("fly app name must be non-empty");
        }
        let api_base_url =
            optional_non_empty_env("PIKA_FLY_API_BASE_URL", DEFAULT_FLY_API_BASE_URL);
        let region = optional_non_empty_env("FLY_BOT_REGION", "iad");
        let image = optional_non_empty_env("FLY_BOT_IMAGE", "registry.fly.io/pika-bot:latest");

        Ok(Self {
            client: reqwest::Client::new(),
            api_token,
            app_name: app_name.to_string(),
            api_base_url,
            region,
            image,
        })
    }

    pub fn app_name(&self) -> &str {
        &self.app_name
    }

    fn base_url(&self) -> String {
        format!(
            "{}/v1/apps/{}",
            self.api_base_url.trim_end_matches('/'),
            self.app_name
        )
    }

    pub async fn create_volume(&self, name: &str) -> anyhow::Result<Volume> {
        let url = format!("{}/volumes", self.base_url());
        let body = CreateVolumeRequest {
            name: name.to_string(),
            region: self.region.clone(),
            size_gb: 1,
        };
        let resp = self
            .client
            .post(&url)
            .bearer_auth(&self.api_token)
            .json(&body)
            .send()
            .await
            .context("send create volume request")?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("failed to create volume: {status} {text}");
        }
        resp.json().await.context("decode create volume response")
    }

    pub async fn create_machine(
        &self,
        name: &str,
        volume_id: &str,
        env: HashMap<String, String>,
        image_override: Option<&str>,
    ) -> anyhow::Result<Machine> {
        let url = format!("{}/machines", self.base_url());
        let image = image_override
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(self.image.as_str())
            .to_string();
        let body = CreateMachineRequest {
            name: name.to_string(),
            region: self.region.clone(),
            config: MachineConfig {
                image,
                env,
                guest: GuestConfig {
                    cpu_kind: "shared".to_string(),
                    cpus: 1,
                    memory_mb: 256,
                },
                mounts: vec![MachineMount {
                    volume: volume_id.to_string(),
                    path: "/app/state".to_string(),
                }],
            },
        };

        let resp = self
            .client
            .post(&url)
            .bearer_auth(&self.api_token)
            .json(&body)
            .send()
            .await
            .context("send create machine request")?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("failed to create machine: {status} {text}");
        }
        resp.json().await.context("decode create machine response")
    }

    #[allow(dead_code)]
    pub async fn get_machine(&self, machine_id: &str) -> anyhow::Result<Machine> {
        let url = format!("{}/machines/{machine_id}", self.base_url());
        let resp = self
            .client
            .get(&url)
            .bearer_auth(&self.api_token)
            .send()
            .await
            .context("send get machine request")?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("failed to get machine: {status} {text}");
        }
        resp.json().await.context("decode get machine response")
    }

    pub async fn stop_machine(&self, machine_id: &str) -> Result<StopMachineOutcome, FlyApiError> {
        #[derive(Serialize)]
        struct StopMachineRequest {
            signal: String,
            timeout: String,
        }

        let url = format!("{}/machines/{machine_id}/stop", self.base_url());
        let resp = self
            .client
            .post(&url)
            .bearer_auth(&self.api_token)
            .json(&StopMachineRequest {
                signal: "SIGTERM".to_string(),
                timeout: "10s".to_string(),
            })
            .send()
            .await
            .map_err(|err| FlyApiError::transport("stop_machine", err))?;
        let status = resp.status();
        if status.is_success() {
            return Ok(StopMachineOutcome::Stopped);
        }
        if status == StatusCode::NOT_FOUND {
            return Ok(StopMachineOutcome::AlreadyGone);
        }
        let detail = resp.text().await.unwrap_or_default();
        Err(FlyApiError::provider("stop_machine", status, detail))
    }

    pub async fn delete_machine(
        &self,
        machine_id: &str,
    ) -> Result<DeleteMachineOutcome, FlyApiError> {
        let url = format!("{}/machines/{machine_id}?force=true", self.base_url());
        let resp = self
            .client
            .delete(&url)
            .bearer_auth(&self.api_token)
            .send()
            .await
            .map_err(|err| FlyApiError::transport("delete_machine", err))?;
        let status = resp.status();
        if status.is_success() {
            return Ok(DeleteMachineOutcome::Deleted);
        }
        if status == StatusCode::NOT_FOUND {
            return Ok(DeleteMachineOutcome::AlreadyGone);
        }
        let detail = resp.text().await.unwrap_or_default();
        Err(FlyApiError::provider("delete_machine", status, detail))
    }

    pub async fn delete_volume(&self, volume_id: &str) -> Result<DeleteVolumeOutcome, FlyApiError> {
        let url = format!("{}/volumes/{volume_id}", self.base_url());
        let resp = self
            .client
            .delete(&url)
            .bearer_auth(&self.api_token)
            .send()
            .await
            .map_err(|err| FlyApiError::transport("delete_volume", err))?;
        let status = resp.status();
        if status.is_success() {
            return Ok(DeleteVolumeOutcome::Deleted);
        }
        if status == StatusCode::NOT_FOUND {
            return Ok(DeleteVolumeOutcome::AlreadyGone);
        }
        if matches!(
            status,
            StatusCode::CONFLICT | StatusCode::UNPROCESSABLE_ENTITY
        ) {
            return Ok(DeleteVolumeOutcome::Conflict);
        }
        let detail = resp.text().await.unwrap_or_default();
        Err(FlyApiError::provider("delete_volume", status, detail))
    }
}

fn required_non_empty_env(key: &str) -> anyhow::Result<String> {
    let value = std::env::var(key).with_context(|| format!("{key} must be set"))?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        anyhow::bail!("{key} must be non-empty");
    }
    Ok(trimmed.to_string())
}

fn optional_non_empty_env(key: &str, default: &str) -> String {
    match std::env::var(key) {
        Ok(value) if !value.trim().is_empty() => value.trim().to_string(),
        _ => default.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use std::collections::HashMap;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    #[derive(Debug)]
    struct CapturedRequest {
        method: String,
        path: String,
        headers: HashMap<String, String>,
        body: String,
    }

    fn spawn_one_shot_server(
        status_line: &str,
        response_body: &str,
    ) -> (String, mpsc::Receiver<CapturedRequest>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
        let addr = listener.local_addr().expect("read mock server addr");
        let (tx, rx) = mpsc::channel();
        let status_line = status_line.to_string();
        let response_body = response_body.to_string();

        thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept mock request");
            let req = read_http_request(&mut stream);
            tx.send(req).expect("send captured request");

            let response = format!(
                "HTTP/1.1 {status_line}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                response_body.len(),
                response_body
            );
            stream
                .write_all(response.as_bytes())
                .expect("write mock response");
        });

        (format!("http://{addr}"), rx)
    }

    fn read_http_request(stream: &mut std::net::TcpStream) -> CapturedRequest {
        let mut buf = Vec::new();
        let mut header_end = None;
        let mut content_length = 0usize;

        loop {
            let mut chunk = [0u8; 4096];
            let n = stream.read(&mut chunk).expect("read request bytes");
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
            if header_end.is_none() {
                header_end = buf
                    .windows(4)
                    .position(|window| window == b"\r\n\r\n")
                    .map(|idx| idx + 4);
                if let Some(end) = header_end {
                    let headers = String::from_utf8_lossy(&buf[..end]);
                    for line in headers.lines() {
                        if let Some((key, value)) = line.split_once(':') {
                            if key.eq_ignore_ascii_case("content-length") {
                                content_length = value.trim().parse::<usize>().unwrap_or(0);
                            }
                        }
                    }
                }
            }
            if let Some(end) = header_end {
                if buf.len() >= end + content_length {
                    break;
                }
            }
        }

        let end = header_end.expect("request headers must be present");
        let headers_raw = String::from_utf8_lossy(&buf[..end]);
        let mut lines = headers_raw.lines();
        let request_line = lines.next().expect("request line");
        let mut parts = request_line.split_whitespace();
        let method = parts.next().expect("method").to_string();
        let path = parts.next().expect("path").to_string();
        let mut headers = HashMap::new();
        for line in lines {
            if line.trim().is_empty() {
                break;
            }
            if let Some((key, value)) = line.split_once(':') {
                headers.insert(key.trim().to_ascii_lowercase(), value.trim().to_string());
            }
        }
        let body = String::from_utf8(buf[end..end + content_length].to_vec()).expect("utf8 body");

        CapturedRequest {
            method,
            path,
            headers,
            body,
        }
    }

    fn test_client(base_url: String) -> FlyClient {
        FlyClient {
            client: reqwest::Client::new(),
            api_token: "fly-token".to_string(),
            app_name: "pika-test".to_string(),
            api_base_url: base_url,
            region: "iad".to_string(),
            image: "registry.fly.io/pika-bot:test".to_string(),
        }
    }

    #[tokio::test]
    async fn create_volume_contract_request_shape() {
        let (base_url, rx) = spawn_one_shot_server("200 OK", r#"{"id":"vol-123"}"#);
        let fly = test_client(base_url);

        let volume = fly
            .create_volume("state-volume")
            .await
            .expect("create volume succeeds");
        assert_eq!(volume.id, "vol-123");

        let req = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("captured request");
        assert_eq!(req.method, "POST");
        assert_eq!(req.path, "/v1/apps/pika-test/volumes");
        assert_eq!(
            req.headers.get("authorization").map(String::as_str),
            Some("Bearer fly-token")
        );

        let json: Value = serde_json::from_str(&req.body).expect("parse json body");
        assert_eq!(json["name"], "state-volume");
        assert_eq!(json["region"], "iad");
        assert_eq!(json["size_gb"], 1);
    }

    #[tokio::test]
    async fn create_machine_contract_request_shape() {
        let (base_url, rx) =
            spawn_one_shot_server("200 OK", r#"{"id":"machine-abc","state":"started"}"#);
        let fly = test_client(base_url);

        let mut env = HashMap::new();
        env.insert("PIKA_OWNER_PUBKEY".to_string(), "pubkey123".to_string());

        let machine = fly
            .create_machine("bot-machine", "vol-123", env, None)
            .await
            .expect("create machine succeeds");
        assert_eq!(machine.id, "machine-abc");
        assert_eq!(machine.state, "started");

        let req = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("captured request");
        assert_eq!(req.method, "POST");
        assert_eq!(req.path, "/v1/apps/pika-test/machines");
        assert_eq!(
            req.headers.get("authorization").map(String::as_str),
            Some("Bearer fly-token")
        );

        let json: Value = serde_json::from_str(&req.body).expect("parse json body");
        assert_eq!(json["name"], "bot-machine");
        assert_eq!(json["region"], "iad");
        assert_eq!(json["config"]["image"], "registry.fly.io/pika-bot:test");
        assert_eq!(json["config"]["guest"]["cpu_kind"], "shared");
        assert_eq!(json["config"]["guest"]["cpus"], 1);
        assert_eq!(json["config"]["guest"]["memory_mb"], 256);
        assert_eq!(json["config"]["mounts"][0]["volume"], "vol-123");
        assert_eq!(json["config"]["mounts"][0]["path"], "/app/state");
        assert_eq!(json["config"]["env"]["PIKA_OWNER_PUBKEY"], "pubkey123");
    }

    #[tokio::test]
    async fn create_machine_uses_image_override_when_provided() {
        let (base_url, rx) =
            spawn_one_shot_server("200 OK", r#"{"id":"machine-override","state":"started"}"#);
        let fly = test_client(base_url);

        let machine = fly
            .create_machine(
                "bot-machine",
                "vol-123",
                HashMap::new(),
                Some("registry.example.com/pika@sha256:abcd"),
            )
            .await
            .expect("create machine succeeds");
        assert_eq!(machine.id, "machine-override");

        let req = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("captured request");
        let json: Value = serde_json::from_str(&req.body).expect("parse json body");
        assert_eq!(
            json["config"]["image"],
            "registry.example.com/pika@sha256:abcd"
        );
    }

    #[tokio::test]
    async fn get_machine_contract_request_shape() {
        let (base_url, rx) =
            spawn_one_shot_server("200 OK", r#"{"id":"machine-xyz","state":"stopped"}"#);
        let fly = test_client(base_url);

        let machine = fly
            .get_machine("machine-xyz")
            .await
            .expect("get machine succeeds");
        assert_eq!(machine.id, "machine-xyz");
        assert_eq!(machine.state, "stopped");

        let req = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("captured request");
        assert_eq!(req.method, "GET");
        assert_eq!(req.path, "/v1/apps/pika-test/machines/machine-xyz");
        assert_eq!(
            req.headers.get("authorization").map(String::as_str),
            Some("Bearer fly-token")
        );
        assert!(req.body.is_empty());
    }

    #[tokio::test]
    async fn stop_machine_contract_request_shape() {
        let (base_url, rx) = spawn_one_shot_server("200 OK", "{}");
        let fly = test_client(base_url);

        let outcome = fly
            .stop_machine("machine-xyz")
            .await
            .expect("stop machine succeeds");
        assert_eq!(outcome, StopMachineOutcome::Stopped);

        let req = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("captured request");
        assert_eq!(req.method, "POST");
        assert_eq!(req.path, "/v1/apps/pika-test/machines/machine-xyz/stop");
        assert_eq!(
            req.headers.get("authorization").map(String::as_str),
            Some("Bearer fly-token")
        );

        let json: Value = serde_json::from_str(&req.body).expect("parse json body");
        assert_eq!(json["signal"], "SIGTERM");
        assert_eq!(json["timeout"], "10s");
    }

    #[tokio::test]
    async fn stop_machine_404_is_already_gone() {
        let (base_url, _rx) = spawn_one_shot_server("404 Not Found", "machine missing");
        let fly = test_client(base_url);

        let outcome = fly
            .stop_machine("machine-missing")
            .await
            .expect("404 should map to already gone");
        assert_eq!(outcome, StopMachineOutcome::AlreadyGone);
    }

    #[tokio::test]
    async fn delete_machine_contract_request_shape() {
        let (base_url, rx) = spawn_one_shot_server("200 OK", "{}");
        let fly = test_client(base_url);

        let outcome = fly
            .delete_machine("machine-xyz")
            .await
            .expect("delete machine succeeds");
        assert_eq!(outcome, DeleteMachineOutcome::Deleted);

        let req = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("captured request");
        assert_eq!(req.method, "DELETE");
        assert_eq!(
            req.path,
            "/v1/apps/pika-test/machines/machine-xyz?force=true"
        );
        assert_eq!(
            req.headers.get("authorization").map(String::as_str),
            Some("Bearer fly-token")
        );
        assert!(req.body.is_empty());
    }

    #[tokio::test]
    async fn delete_machine_404_is_already_gone() {
        let (base_url, _rx) = spawn_one_shot_server("404 Not Found", "missing");
        let fly = test_client(base_url);

        let outcome = fly
            .delete_machine("machine-missing")
            .await
            .expect("404 should map to already gone");
        assert_eq!(outcome, DeleteMachineOutcome::AlreadyGone);
    }

    #[tokio::test]
    async fn delete_volume_contract_request_shape() {
        let (base_url, rx) = spawn_one_shot_server("200 OK", "{}");
        let fly = test_client(base_url);

        let outcome = fly
            .delete_volume("vol-123")
            .await
            .expect("delete volume succeeds");
        assert_eq!(outcome, DeleteVolumeOutcome::Deleted);

        let req = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("captured request");
        assert_eq!(req.method, "DELETE");
        assert_eq!(req.path, "/v1/apps/pika-test/volumes/vol-123");
        assert_eq!(
            req.headers.get("authorization").map(String::as_str),
            Some("Bearer fly-token")
        );
    }

    #[tokio::test]
    async fn delete_volume_404_is_already_gone() {
        let (base_url, _rx) = spawn_one_shot_server("404 Not Found", "missing");
        let fly = test_client(base_url);

        let outcome = fly
            .delete_volume("vol-missing")
            .await
            .expect("404 should map to already gone");
        assert_eq!(outcome, DeleteVolumeOutcome::AlreadyGone);
    }

    #[tokio::test]
    async fn delete_volume_409_is_conflict() {
        let (base_url, _rx) = spawn_one_shot_server("409 Conflict", "volume in use");
        let fly = test_client(base_url);

        let outcome = fly
            .delete_volume("vol-attached")
            .await
            .expect("409 should map to conflict");
        assert_eq!(outcome, DeleteVolumeOutcome::Conflict);
    }

    #[tokio::test]
    async fn delete_volume_422_is_conflict() {
        let (base_url, _rx) = spawn_one_shot_server("422 Unprocessable Entity", "attached");
        let fly = test_client(base_url);

        let outcome = fly
            .delete_volume("vol-attached")
            .await
            .expect("422 should map to conflict");
        assert_eq!(outcome, DeleteVolumeOutcome::Conflict);
    }

    #[tokio::test]
    async fn create_volume_surfaces_error_body() {
        let (base_url, _rx) = spawn_one_shot_server("500 Internal Server Error", "no quota");
        let fly = test_client(base_url);

        let err = fly
            .create_volume("state-volume")
            .await
            .expect_err("expected create_volume failure");
        let msg = err.to_string();
        assert!(msg.contains("failed to create volume"));
        assert!(msg.contains("500 Internal Server Error"));
        assert!(msg.contains("no quota"));
    }

    #[tokio::test]
    async fn create_machine_surfaces_error_body() {
        let (base_url, _rx) = spawn_one_shot_server("422 Unprocessable Entity", "invalid config");
        let fly = test_client(base_url);

        let err = fly
            .create_machine("bot-machine", "vol-bad", HashMap::new(), None)
            .await
            .expect_err("expected create_machine failure");
        let msg = err.to_string();
        assert!(msg.contains("failed to create machine"));
        assert!(msg.contains("422 Unprocessable Entity"));
        assert!(msg.contains("invalid config"));
    }

    #[tokio::test]
    async fn get_machine_surfaces_error_body() {
        let (base_url, _rx) = spawn_one_shot_server("404 Not Found", "machine not found");
        let fly = test_client(base_url);

        let err = fly
            .get_machine("machine-missing")
            .await
            .expect_err("expected get_machine failure");
        let msg = err.to_string();
        assert!(msg.contains("failed to get machine"));
        assert!(msg.contains("404 Not Found"));
        assert!(msg.contains("machine not found"));
    }

    #[tokio::test]
    async fn delete_machine_surfaces_status_code_and_retryability() {
        let (base_url, _rx) = spawn_one_shot_server("503 Service Unavailable", "fly outage");
        let fly = test_client(base_url);

        let err = fly
            .delete_machine("machine-xyz")
            .await
            .expect_err("expected delete_machine failure");
        assert_eq!(err.status_code(), Some(503));
        assert!(err.is_retryable());
        assert!(err.to_string().contains("delete_machine failed"));
    }
}
