//! Minimal MCP server (Streamable HTTP transport, JSON responses only) so that
//! Claude Code can read termnest's context. It listens on 127.0.0.1 only and
//! requires a bearer token.

use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;

use crate::error::{CoreError, CoreResult, ErrorKind};

pub const PROTOCOL_VERSION: &str = "2025-06-18";
const SUPPORTED_VERSIONS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];
const MAX_BODY: usize = 1024 * 1024;

pub type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;

#[derive(Debug, Clone)]
pub struct ToolDef {
    pub name: &'static str,
    pub description: &'static str,
    pub input_schema: Value,
}

/// What the server exposes: the app implements this on top of its state.
pub trait ToolProvider: Send + Sync + 'static {
    fn tools(&self) -> Vec<ToolDef>;
    /// Runs a tool; `Ok` text is the result, `Err` text is reported as a tool error.
    fn call(&self, name: String, args: Value) -> BoxFuture<Result<String, String>>;
}

fn rpc_result(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

/// Handles one JSON-RPC message. Notifications produce no response.
pub async fn handle_message(provider: &dyn ToolProvider, msg: Value) -> Option<Value> {
    let id = msg.get("id").cloned();
    let method = msg
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let params = msg.get("params").cloned().unwrap_or(Value::Null);
    let id = id?; // notification (or a response from the client): nothing to answer

    let response = match method.as_str() {
        "initialize" => {
            let requested = params
                .get("protocolVersion")
                .and_then(Value::as_str)
                .unwrap_or(PROTOCOL_VERSION);
            let version = if SUPPORTED_VERSIONS.contains(&requested) {
                requested
            } else {
                PROTOCOL_VERSION
            };
            rpc_result(
                id,
                json!({
                    "protocolVersion": version,
                    "capabilities": { "tools": { "listChanged": false } },
                    "serverInfo": { "name": "termnest", "version": env!("CARGO_PKG_VERSION") },
                    "instructions": "termnest is the user's SSH client. Use these tools to read which servers \
                        and terminal sessions are open, what a terminal printed, and files on servers via SFTP. \
                        Reading tools need no approval; exec and send_input act on servers only after the user approves them in termnest.",
                }),
            )
        }
        "ping" => rpc_result(id, json!({})),
        "tools/list" => {
            let tools: Vec<Value> = provider
                .tools()
                .into_iter()
                .map(|t| json!({ "name": t.name, "description": t.description, "inputSchema": t.input_schema }))
                .collect();
            rpc_result(id, json!({ "tools": tools }))
        }
        "tools/call" => {
            let name = params
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            if !provider.tools().iter().any(|t| t.name == name) {
                return Some(rpc_error(id, -32602, &format!("Unknown tool: {name}")));
            }
            let args = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            let (text, is_error) = match provider.call(name, args).await {
                Ok(t) => (t, false),
                Err(e) => (e, true),
            };
            rpc_result(
                id,
                json!({ "content": [{ "type": "text", "text": text }], "isError": is_error }),
            )
        }
        "" => rpc_error(id, -32600, "Invalid request"),
        _ => rpc_error(id, -32601, &format!("Method not found: {method}")),
    };
    Some(response)
}

/// Handles a request body: a single message or a batch.
pub async fn handle_body(provider: &dyn ToolProvider, body: &[u8]) -> Option<Value> {
    let parsed: Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(_) => return Some(rpc_error(Value::Null, -32700, "Parse error")),
    };
    match parsed {
        Value::Array(items) => {
            let mut out = Vec::new();
            for item in items {
                if let Some(r) = handle_message(provider, item).await {
                    out.push(r);
                }
            }
            (!out.is_empty()).then_some(Value::Array(out))
        }
        msg => handle_message(provider, msg).await,
    }
}

struct Request {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Request {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// Reads one HTTP/1.1 request; `None` when the client closed the connection.
async fn read_request<R: tokio::io::AsyncBufRead + Unpin>(
    reader: &mut R,
) -> std::io::Result<Option<Request>> {
    let mut line = String::new();
    if reader.read_line(&mut line).await? == 0 {
        return Ok(None);
    }
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_owned();
    let path = parts.next().unwrap_or_default().to_owned();
    let mut headers = Vec::new();
    loop {
        let mut h = String::new();
        if reader.read_line(&mut h).await? == 0 {
            return Ok(None);
        }
        let h = h.trim_end();
        if h.is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            headers.push((k.trim().to_owned(), v.trim().to_owned()));
        }
        if headers.len() > 100 {
            return Err(std::io::Error::other("too many headers"));
        }
    }
    let len: usize = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.parse().ok())
        .unwrap_or(0);
    if len > MAX_BODY {
        return Err(std::io::Error::other("body too large"));
    }
    let mut body = vec![0; len];
    reader.read_exact(&mut body).await?;
    Ok(Some(Request {
        method,
        path,
        headers,
        body,
    }))
}

async fn respond(
    stream: &mut (impl AsyncWriteExt + Unpin),
    status: &str,
    body: Option<&Value>,
) -> std::io::Result<()> {
    let payload = body.map(|b| b.to_string()).unwrap_or_default();
    let content_type = if body.is_some() {
        "Content-Type: application/json\r\n"
    } else {
        ""
    };
    let head = format!(
        "HTTP/1.1 {status}\r\n{content_type}Content-Length: {}\r\nCache-Control: no-store\r\n\r\n",
        payload.len()
    );
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(payload.as_bytes()).await?;
    stream.flush().await
}

/// Constant-time comparison for the bearer token.
fn token_matches(header: Option<&str>, token: &str) -> bool {
    let Some(given) = header.and_then(|h| h.strip_prefix("Bearer ")) else {
        return false;
    };
    let (a, b) = (given.trim().as_bytes(), token.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Browsers send Origin; only local origins are accepted (DNS rebinding protection).
fn origin_allowed(origin: Option<&str>) -> bool {
    match origin {
        None => true,
        Some(o) => {
            let host = o.split("://").nth(1).unwrap_or(o);
            let host = host.split(['/', ':']).next().unwrap_or(host);
            matches!(host, "127.0.0.1" | "localhost" | "[::1]")
        }
    }
}

async fn serve_connection(
    stream: TcpStream,
    provider: Arc<dyn ToolProvider>,
    token: Arc<String>,
) -> std::io::Result<()> {
    let (read, mut write) = stream.into_split();
    let mut reader = BufReader::new(read);
    while let Some(req) = read_request(&mut reader).await? {
        if !origin_allowed(req.header("origin")) {
            respond(&mut write, "403 Forbidden", None).await?;
            continue;
        }
        if !token_matches(req.header("authorization"), &token) {
            respond(&mut write, "401 Unauthorized", None).await?;
            continue;
        }
        if req.path.split('?').next() != Some("/mcp") {
            respond(&mut write, "404 Not Found", None).await?;
            continue;
        }
        match req.method.as_str() {
            "POST" => match handle_body(provider.as_ref(), &req.body).await {
                Some(v) => respond(&mut write, "200 OK", Some(&v)).await?,
                None => respond(&mut write, "202 Accepted", None).await?,
            },
            // No server-initiated stream; sessions are not tracked.
            "GET" | "DELETE" => respond(&mut write, "405 Method Not Allowed", None).await?,
            _ => respond(&mut write, "405 Method Not Allowed", None).await?,
        }
    }
    Ok(())
}

/// A running server; dropping it (or calling `stop`) shuts it down.
pub struct McpServer {
    pub addr: SocketAddr,
    shutdown: Option<oneshot::Sender<()>>,
}

impl McpServer {
    pub async fn start(
        port: u16,
        token: String,
        provider: Arc<dyn ToolProvider>,
    ) -> CoreResult<Self> {
        let listener = TcpListener::bind(("127.0.0.1", port)).await.map_err(|e| {
            CoreError::new(
                ErrorKind::Storage,
                format!("MCP: не удалось занять порт {port}: {e}"),
            )
        })?;
        let addr = listener
            .local_addr()
            .map_err(|e| CoreError::new(ErrorKind::Storage, format!("MCP: {e}")))?;
        let (tx, mut rx) = oneshot::channel();
        let token = Arc::new(token);
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut rx => break,
                    accepted = listener.accept() => {
                        let Ok((stream, peer)) = accepted else { continue };
                        if !peer.ip().is_loopback() {
                            continue;
                        }
                        let provider = provider.clone();
                        let token = token.clone();
                        tokio::spawn(async move {
                            if let Err(e) = serve_connection(stream, provider, token).await {
                                log::debug!("mcp connection: {e}");
                            }
                        });
                    }
                }
            }
        });
        Ok(Self {
            addr,
            shutdown: Some(tx),
        })
    }

    pub fn stop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
    }
}

impl Drop for McpServer {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Random 32-byte token, hex encoded.
pub fn new_token() -> String {
    let bytes: [u8; 32] = rand::random();
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Echo;

    impl ToolProvider for Echo {
        fn tools(&self) -> Vec<ToolDef> {
            vec![ToolDef {
                name: "echo",
                description: "Echoes `text`",
                input_schema: json!({ "type": "object", "properties": { "text": { "type": "string" } } }),
            }]
        }

        fn call(&self, _name: String, args: Value) -> BoxFuture<Result<String, String>> {
            Box::pin(async move {
                match args.get("text").and_then(Value::as_str) {
                    Some(t) => Ok(t.to_owned()),
                    None => Err("text is required".to_owned()),
                }
            })
        }
    }

    async fn post(addr: SocketAddr, token: &str, body: &str) -> (String, String) {
        let mut s = TcpStream::connect(addr).await.unwrap();
        let req = format!(
            "POST /mcp HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        s.write_all(req.as_bytes()).await.unwrap();
        s.shutdown().await.unwrap();
        let mut resp = String::new();
        s.read_to_string(&mut resp).await.unwrap();
        let (head, body) = resp.split_once("\r\n\r\n").unwrap();
        (head.lines().next().unwrap().to_owned(), body.to_owned())
    }

    #[tokio::test]
    async fn protocol_over_http() {
        let server = McpServer::start(0, "secret".into(), Arc::new(Echo))
            .await
            .unwrap();
        let addr = server.addr;

        let (status, _) = post(addr, "wrong", "{}").await;
        assert!(status.contains("401"));

        let (status, body) = post(
            addr,
            "secret",
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26"}}"#,
        )
        .await;
        assert!(status.contains("200"));
        let v: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["result"]["protocolVersion"], "2025-03-26");
        assert_eq!(v["result"]["serverInfo"]["name"], "termnest");

        let (status, body) = post(
            addr,
            "secret",
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
        )
        .await;
        assert!(status.contains("202"));
        assert!(body.is_empty());

        let (_, body) = post(
            addr,
            "secret",
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
        )
        .await;
        let v: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["result"]["tools"][0]["name"], "echo");

        let (_, body) = post(
            addr,
            "secret",
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"echo","arguments":{"text":"hi"}}}"#,
        )
        .await;
        let v: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["result"]["content"][0]["text"], "hi");
        assert_eq!(v["result"]["isError"], false);

        let (_, body) = post(
            addr,
            "secret",
            r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"echo","arguments":{}}}"#,
        )
        .await;
        let v: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["result"]["isError"], true);

        let (_, body) = post(
            addr,
            "secret",
            r#"{"jsonrpc":"2.0","id":5,"method":"nope"}"#,
        )
        .await;
        let v: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["error"]["code"], -32601);
    }

    #[test]
    fn origin_and_token_checks() {
        assert!(origin_allowed(None));
        assert!(origin_allowed(Some("http://localhost:3000")));
        assert!(!origin_allowed(Some("https://evil.example")));
        assert!(token_matches(Some("Bearer abc"), "abc"));
        assert!(!token_matches(Some("Bearer abd"), "abc"));
        assert!(!token_matches(None, "abc"));
        assert_eq!(new_token().len(), 64);
    }
}
