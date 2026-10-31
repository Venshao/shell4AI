use std::sync::Arc;

use axum::{
    extract::{Request, State},
    http::{header, HeaderValue, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use rmcp::{
    handler::server::{tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, ErrorData},
    schemars::JsonSchema,
    tool, tool_handler, tool_router, ServerHandler,
};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use tokio_util::sync::CancellationToken;

use crate::{service::HostService, ssh::error_json};

#[derive(Clone)]
struct SshMcpServer {
    service: Arc<HostService>,
    #[allow(dead_code)]
    tool_router: ToolRouter<Self>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ExecInput {
    /// Registered host alias, hostname, or IP address.
    host: String,
    /// Shell command to run on the remote host.
    command: String,
    /// Execution timeout in seconds.
    #[serde(default = "default_timeout")]
    timeout_seconds: f64,
    /// Maximum bytes captured independently from stdout and stderr.
    #[serde(default = "default_max_output")]
    max_output_bytes: usize,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ReadFileInput {
    /// Registered host alias, hostname, or IP address.
    host: String,
    /// Absolute or remote-user-relative file path.
    path: String,
    /// Maximum number of bytes returned.
    #[serde(default = "default_max_file")]
    max_bytes: usize,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ListDirInput {
    /// Registered host alias, hostname, or IP address.
    host: String,
    /// Remote directory path.
    #[serde(default = "default_path")]
    path: String,
}

fn default_timeout() -> f64 {
    60.0
}

fn default_max_output() -> usize {
    1024 * 1024
}

fn default_max_file() -> usize {
    512 * 1024
}

fn default_path() -> String {
    ".".into()
}

#[tool_router]
impl SshMcpServer {
    fn new(service: Arc<HostService>) -> Self {
        Self {
            service,
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        name = "list_hosts",
        description = "List SSH hosts registered in the desktop app. Credentials are never returned."
    )]
    async fn list_hosts(&self) -> Result<CallToolResult, ErrorData> {
        match self.service.list_hosts() {
            Ok(hosts) => {
                let values = hosts
                    .into_iter()
                    .filter_map(|view| {
                        let trusted = view.host_key_fingerprint.is_some();
                        let mut value = serde_json::to_value(view).ok()?;
                        if let Some(object) = value.as_object_mut() {
                            object.insert("trusted".into(), Value::Bool(trusted));
                        }
                        Some(value)
                    })
                    .collect::<Vec<_>>();
                Ok(CallToolResult::structured(Value::Array(values)))
            }
            Err(error) => Ok(tool_error(error_json(&error))),
        }
    }

    #[tool(
        name = "ssh_exec",
        description = "Execute a shell command on a registered SSH host. Credentials stay in the desktop app."
    )]
    async fn ssh_exec(
        &self,
        Parameters(input): Parameters<ExecInput>,
    ) -> Result<CallToolResult, ErrorData> {
        match self
            .service
            .exec(
                &input.host,
                &input.command,
                input.timeout_seconds,
                input.max_output_bytes,
            )
            .await
        {
            Ok(result) => Ok(tool_success(with_ok(result))),
            Err(error) => Ok(tool_error(error_json(&error))),
        }
    }

    #[tool(
        name = "ssh_read_file",
        description = "Read a UTF-8 text file from a registered SSH host over SFTP."
    )]
    async fn ssh_read_file(
        &self,
        Parameters(input): Parameters<ReadFileInput>,
    ) -> Result<CallToolResult, ErrorData> {
        match self
            .service
            .read_file(&input.host, &input.path, input.max_bytes)
            .await
        {
            Ok(result) => Ok(tool_success(with_ok(result))),
            Err(error) => Ok(tool_error(error_json(&error))),
        }
    }

    #[tool(
        name = "ssh_list_dir",
        description = "List a directory on a registered SSH host over SFTP."
    )]
    async fn ssh_list_dir(
        &self,
        Parameters(input): Parameters<ListDirInput>,
    ) -> Result<CallToolResult, ErrorData> {
        match self.service.list_dir(&input.host, &input.path).await {
            Ok(result) => Ok(tool_success(with_ok(result))),
            Err(error) => Ok(tool_error(error_json(&error))),
        }
    }
}

#[tool_handler]
impl ServerHandler for SshMcpServer {}

fn with_ok(mut value: Value) -> Value {
    if let Some(object) = value.as_object_mut() {
        let exit_ok = object
            .get("exit_code")
            .and_then(Value::as_i64)
            .map(|code| code == 0);
        object.insert("ok".into(), exit_ok.map(Value::Bool).unwrap_or(Value::Bool(true)));
    }
    value
}

fn tool_success(value: Value) -> CallToolResult {
    CallToolResult::structured(value)
}

fn tool_error(value: Value) -> CallToolResult {
    CallToolResult::structured_error(value)
}

pub fn build_mcp_router(
    service: Arc<HostService>,
    token: String,
    cancellation_token: CancellationToken,
) -> Router {
    let config = rmcp::transport::streamable_http_server::StreamableHttpServerConfig::default()
        .with_json_response(true)
        .with_cancellation_token(cancellation_token);
    let service = rmcp::transport::streamable_http_server::StreamableHttpService::<
        SshMcpServer,
        rmcp::transport::streamable_http_server::session::local::LocalSessionManager,
    >::new(
        move || Ok(SshMcpServer::new(service.clone())),
        Arc::new(Default::default()),
        config,
    );

    let protected = Router::new()
        .nest_service("/mcp", service)
        .route_layer(middleware::from_fn_with_state(token, require_bearer));
    Router::new()
        .route("/health", get(health))
        .merge(protected)
}

async fn health() -> Json<Value> {
    Json(json!({"ok": true, "service": "ssh-mcp"}))
}

async fn require_bearer(
    State(expected): State<String>,
    request: Request,
    next: Next,
) -> Response {
    let supplied = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| {
            value
                .strip_prefix("Bearer ")
                .or_else(|| value.strip_prefix("bearer "))
        })
        .or_else(|| {
            request
                .headers()
                .get("x-api-key")
                .and_then(|value| value.to_str().ok())
        });

    if supplied.is_none_or(|value| !tokens_match(value, &expected)) {
        let mut response = (
            StatusCode::UNAUTHORIZED,
            Json(json!({"ok": false, "error": "invalid MCP token"})),
        )
            .into_response();
        response.headers_mut().insert(
            header::WWW_AUTHENTICATE,
            HeaderValue::from_static("Bearer"),
        );
        return response;
    }
    next.run(request).await
}

fn tokens_match(supplied: &str, expected: &str) -> bool {
    let supplied = Sha256::digest(supplied.as_bytes());
    let expected = Sha256::digest(expected.as_bytes());
    bool::from(supplied.ct_eq(&expected))
}

#[cfg(test)]
mod tests {
    use std::future::IntoFuture;
    use std::sync::Mutex;

    use async_trait::async_trait;
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;

    use super::*;
    use crate::{
        crypto::SecretCipher,
        domain::HostInput,
        repository::HostRepository,
        ssh::{ServerHostKey, SshExecutor},
    };

    struct FakeExecutor {
        calls: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl SshExecutor for FakeExecutor {
        async fn probe(&self, _record: &crate::domain::HostRecord) -> crate::error::AppResult<ServerHostKey> {
            unreachable!()
        }
        async fn test_connection(&self, _record: &crate::domain::HostRecord, _password: &str) -> crate::error::AppResult<()> {
            Ok(())
        }
        async fn exec(
            &self,
            _record: &crate::domain::HostRecord,
            _password: &str,
            _command: &str,
            _timeout_seconds: f64,
            _max_output_bytes: usize,
        ) -> crate::error::AppResult<Value> {
            self.calls.lock().unwrap().push("exec".into());
            Ok(json!({
                "exit_code": 0,
                "stdout": "ok\n",
                "stderr": "",
                "truncated": false,
                "elapsed_ms": 1,
            }))
        }
        async fn read_text_file(
            &self,
            _record: &crate::domain::HostRecord,
            _password: &str,
            path: &str,
            _max_bytes: usize,
        ) -> crate::error::AppResult<Value> {
            Ok(json!({"path": path, "content": "", "truncated": false, "size": 0}))
        }
        async fn list_dir(
            &self,
            _record: &crate::domain::HostRecord,
            _password: &str,
            path: &str,
        ) -> crate::error::AppResult<Value> {
            Ok(json!({"path": path, "entries": []}))
        }
        async fn disconnect(&self, _host_id: &str) {}
        async fn close_all(&self) {}
    }

    async fn service() -> (tempfile::TempDir, Arc<HostService>) {
        let dir = tempfile::tempdir().expect("tempdir");
        let cipher = SecretCipher::new(&[8_u8; 32]).expect("cipher");
        let repository = Arc::new(HostRepository::open(dir.path(), cipher).expect("repo"));
        let record = repository
            .create(&HostInput {
                alias: "prod".into(),
                hostname: "10.0.0.10".into(),
                port: 22,
                username: "root".into(),
                password: Some("secret".into()),
                enabled: true,
            })
            .expect("host");
        repository
            .set_host_key(&record.id, "SHA256:test", "ssh-ed25519 AAAA")
            .expect("trust");
        let executor = Arc::new(FakeExecutor {
            calls: Mutex::new(Vec::new()),
        });
        (dir, Arc::new(HostService::new(repository, executor, None)))
    }

    #[tokio::test]
    async fn health_is_public_but_mcp_requires_bearer_token() {
        let (_dir, service) = service().await;
        let app = build_mcp_router(service, "secret".into(), CancellationToken::new());
        let health = app
            .clone()
            .oneshot(Request::builder().uri("/health").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(health.status(), StatusCode::OK);
        let unauthorized = app
            .oneshot(Request::builder().uri("/mcp").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn token_comparison_accepts_only_exact_value() {
        assert!(tokens_match("secret", "secret"));
        assert!(!tokens_match("secret2", "secret"));
    }

    #[tokio::test]
    async fn streamable_http_handshake_lists_passwordless_tools() {
        let (_dir, service) = service().await;
        let cancellation = CancellationToken::new();
        let app = build_mcp_router(service, "secret".into(), cancellation.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let address = listener.local_addr().expect("address");
        let server = tokio::spawn(axum::serve(listener, app).into_future());
        let client = reqwest::Client::new();
        let url = format!("http://{address}/mcp");

        let initialize = client
            .post(&url)
            .bearer_auth("secret")
            .header("Accept", "application/json, text/event-stream")
            .json(&json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "protocolVersion": "2025-03-26",
                    "capabilities": {},
                    "clientInfo": {"name": "test", "version": "1.0"}
                }
            }))
            .send()
            .await
            .expect("initialize");
        assert_eq!(initialize.status(), StatusCode::OK);
        let session = initialize
            .headers()
            .get("mcp-session-id")
            .expect("session header")
            .to_str()
            .expect("session text")
            .to_owned();

        let initialized = client
            .post(&url)
            .bearer_auth("secret")
            .header("mcp-session-id", &session)
            .header("MCP-Protocol-Version", "2025-03-26")
            .header("Accept", "application/json, text/event-stream")
            .json(&json!({
                "jsonrpc": "2.0",
                "method": "notifications/initialized"
            }))
            .send()
            .await
            .expect("initialized notification");
        assert!(
            initialized.status().is_success(),
            "initialized failed: {}",
            initialized.status()
        );

        let tools = client
            .post(&url)
            .bearer_auth("secret")
            .header("mcp-session-id", &session)
            .header("MCP-Protocol-Version", "2025-03-26")
            .header("Accept", "application/json, text/event-stream")
            .json(&json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/list",
                "params": {}
            }))
            .send()
            .await
            .expect("tools list");
        assert_eq!(tools.status(), StatusCode::OK);
        let body = tools.text().await.expect("tools body");
        let payload = body
            .lines()
            .find_map(|line| line.strip_prefix("data: ").filter(|data| data.starts_with('{')))
            .expect("json-rpc payload");
        let value: Value = serde_json::from_str(payload).expect("parse tools list");
        let listed = value["result"]["tools"]
            .as_array()
            .expect("tools array")
            .iter()
            .map(|tool| tool["name"].as_str().unwrap_or_default().to_owned())
            .collect::<Vec<_>>();
        for name in ["list_hosts", "ssh_exec", "ssh_read_file", "ssh_list_dir"] {
            assert!(listed.iter().any(|item| item == name), "missing tool {name}: {body}");
        }
        for tool in value["result"]["tools"].as_array().expect("tools array") {
            let properties = tool["inputSchema"]["properties"]
                .as_object()
                .cloned()
                .unwrap_or_default();
            assert!(
                !properties.contains_key("password"),
                "tool {} exposes password: {tool}",
                tool["name"]
            );
        }

        cancellation.cancel();
        server.abort();
    }
}
