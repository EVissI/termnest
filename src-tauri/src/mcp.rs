//! termnest's MCP tools (read-only) and the server lifecycle.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use serde::Serialize;
use serde_json::{json, Value};
use tauri::Emitter;
use termnest_core::layout::{Node, Pane};
use termnest_core::mcp::{BoxFuture, McpServer, ToolDef, ToolProvider};
use termnest_core::prompt::{PromptReply, PromptRequest};
use termnest_core::settings::Settings;
use termnest_core::{paths, project, sftp, CoreError, CoreResult, ErrorKind, SessionId};
use tokio::io::AsyncReadExt;
use tokio::sync::oneshot;

use crate::state::AppState;

pub const SELECTION_REQUEST_EVENT: &str = "mcp://selection-request";
const LOG_LIMIT: usize = 200;
const DEFAULT_READ_BYTES: u64 = 64 * 1024;
const MAX_READ_BYTES: u64 = 256 * 1024;

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

#[derive(Debug, Clone, Serialize)]
pub struct LogEntry {
    /// Unix time, seconds.
    pub time: u64,
    pub tool: String,
    pub session: Option<SessionId>,
    pub ok: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct McpStatus {
    pub enabled: bool,
    pub running: bool,
    pub port: u16,
    pub token: String,
    pub add_command: String,
    pub error: Option<String>,
}

/// MCP server state owned by the app.
#[derive(Default)]
pub struct McpHub {
    server: tokio::sync::Mutex<Option<McpServer>>,
    error: Mutex<Option<String>>,
    log: Mutex<VecDeque<LogEntry>>,
    selection: Mutex<HashMap<u64, oneshot::Sender<String>>>,
    next_request: AtomicU64,
    /// Sessions where the user allowed agent actions until the session closes.
    trusted: Mutex<HashSet<SessionId>>,
}

impl McpHub {
    fn token_path(dir: &std::path::Path) -> PathBuf {
        dir.join("mcp-token")
    }

    /// Reads the token, creating one (readable only by the user) on first use.
    pub fn token(dir: &std::path::Path) -> CoreResult<String> {
        let path = Self::token_path(dir);
        if let Ok(t) = std::fs::read_to_string(&path) {
            let t = t.trim().to_owned();
            if t.len() >= 32 {
                return Ok(t);
            }
        }
        Self::regenerate_token(dir)
    }

    pub fn regenerate_token(dir: &std::path::Path) -> CoreResult<String> {
        let path = Self::token_path(dir);
        let token = termnest_core::mcp::new_token();
        paths::atomic_write(&path, token.as_bytes())
            .map_err(|e| CoreError::storage("Не удалось сохранить токен MCP", e))?;
        let _ = termnest_core::keys::restrict_to_owner(&path);
        Ok(token)
    }

    pub fn log(&self) -> Vec<LogEntry> {
        lock(&self.log).iter().rev().cloned().collect()
    }

    fn record(&self, tool: &str, session: Option<SessionId>, ok: bool) {
        let time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let mut log = lock(&self.log);
        log.push_back(LogEntry {
            time,
            tool: tool.to_owned(),
            session,
            ok,
        });
        while log.len() > LOG_LIMIT {
            log.pop_front();
        }
    }

    /// Answer from a window to a selection request.
    pub fn selection_reply(&self, id: u64, text: String) {
        if text.trim().is_empty() {
            return; // wait for a window that has something selected
        }
        if let Some(tx) = lock(&self.selection).remove(&id) {
            let _ = tx.send(text);
        }
    }

    pub async fn status(&self, state: &AppState) -> McpStatus {
        let settings = state.settings().claude;
        let running = self.server.lock().await.is_some();
        let token = Self::token(&state.dir).unwrap_or_default();
        let url = format!("http://127.0.0.1:{}/mcp", settings.mcp_port);
        McpStatus {
            enabled: settings.mcp_enabled,
            running,
            port: settings.mcp_port,
            add_command: format!(
                "claude mcp add --transport http --scope user termnest {url} --header \"Authorization: Bearer {token}\""
            ),
            token,
            error: lock(&self.error).clone(),
        }
    }

    /// Starts, restarts or stops the server to match the settings.
    pub async fn apply(&self, state: &Arc<AppState>, settings: &Settings) {
        let mut slot = self.server.lock().await;
        if let Some(mut s) = slot.take() {
            s.stop();
        }
        *lock(&self.error) = None;
        if !settings.claude.mcp_enabled {
            return;
        }
        let token = match Self::token(&state.dir) {
            Ok(t) => t,
            Err(e) => {
                *lock(&self.error) = Some(e.message);
                return;
            }
        };
        let provider = Arc::new(Tools {
            state: Arc::downgrade(state),
        });
        match McpServer::start(settings.claude.mcp_port, token, provider).await {
            Ok(server) => *slot = Some(server),
            Err(e) => *lock(&self.error) = Some(e.message),
        }
    }
}

struct Tools {
    state: Weak<AppState>,
}

fn int_arg(args: &Value, name: &str) -> Result<u64, String> {
    args.get(name)
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("Параметр `{name}` обязателен (целое число)"))
}

fn str_arg(args: &Value, name: &str) -> Result<String, String> {
    args.get(name)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| format!("Параметр `{name}` обязателен (строка)"))
}

fn session_schema(extra: Value) -> Value {
    let mut props =
        json!({ "session": { "type": "integer", "description": "Session id from list_sessions" } });
    if let (Some(p), Some(e)) = (props.as_object_mut(), extra.as_object()) {
        for (k, v) in e {
            p.insert(k.clone(), v.clone());
        }
    }
    let mut required = vec!["session".to_owned()];
    if extra.get("path").is_some() {
        required.push("path".to_owned());
    }
    json!({ "type": "object", "properties": props, "required": required })
}

fn panes(node: &Node, out: &mut Vec<Pane>) {
    match node {
        Node::Pane(p) => out.push(p.clone()),
        Node::Split { first, second, .. } => {
            panes(first, out);
            panes(second, out);
        }
    }
}

impl Tools {
    fn state(&self) -> Result<Arc<AppState>, String> {
        self.state
            .upgrade()
            .ok_or_else(|| "termnest закрывается".to_owned())
    }

    fn list_hosts(state: &AppState) -> Value {
        let hosts = state.hosts.list();
        let items: Vec<Value> = hosts
            .hosts
            .iter()
            .map(|h| {
                json!({
                    "name": h.name,
                    "target": format!("{}@{}:{}", h.user, h.host, h.port),
                    "group": h.group,
                    "project": project::resolve(None, Some(h), &hosts.groups),
                    "note": h.note,
                })
            })
            .collect();
        json!({ "hosts": items })
    }

    fn list_sessions(state: &AppState) -> Value {
        let layout = state.layout().clone();
        let hosts = state.hosts.list();
        let mut items = Vec::new();
        for (wi, w) in layout.windows.iter().enumerate() {
            for t in &w.tabs {
                let mut ps = Vec::new();
                panes(&t.root, &mut ps);
                for p in ps {
                    let Some(id) = p.session else { continue };
                    let host = p
                        .spec
                        .as_ref()
                        .and_then(|s| s.host_id.as_ref())
                        .and_then(|hid| hosts.hosts.iter().find(|h| &h.id == hid));
                    let status = state.sessions.status(id).ok();
                    items.push(json!({
                        "session": id,
                        "server": p.spec.as_ref().map(|s| s.label.clone()),
                        "target": p.spec.as_ref().map(|s| format!("{}@{}:{}", s.user, s.host, s.port)),
                        "state": status.as_ref().map(|s| s.state),
                        "project": project::resolve(p.project.as_deref(), host, &hosts.groups),
                        "window": wi + 1,
                        "tab": t.title.clone(),
                        "focused": w.active == Some(t.id) && t.focused == p.id,
                    }));
                }
            }
        }
        json!({ "sessions": items })
    }

    async fn read_selection(state: &AppState) -> Result<String, String> {
        let id = state.mcp.next_request.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        lock(&state.mcp.selection).insert(id, tx);
        let _ = state.app.emit(SELECTION_REQUEST_EVENT, id);
        let result = tokio::time::timeout(Duration::from_millis(1500), rx).await;
        lock(&state.mcp.selection).remove(&id);
        match result {
            Ok(Ok(text)) => Ok(text),
            _ => Ok("В терминалах termnest ничего не выделено.".to_owned()),
        }
    }

    async fn sftp_read(
        state: &AppState,
        session: SessionId,
        path: &str,
        max: u64,
    ) -> CoreResult<String> {
        let s = state.sessions.sftp(session).await?;
        let file = s
            .open(path)
            .await
            .map_err(|e| sftp::map_sftp_error(e, &format!("Чтение {path}")))?;
        let mut buf = Vec::new();
        let mut limited = file.take(max + 1);
        limited
            .read_to_end(&mut buf)
            .await
            .map_err(|e| CoreError::new(ErrorKind::Remote, format!("Чтение {path}: {e}")))?;
        let truncated = buf.len() as u64 > max;
        buf.truncate(max as usize);
        if buf.contains(&0) {
            return Ok(format!("{path}: двоичный файл, содержимое не показано."));
        }
        let mut text = String::from_utf8_lossy(&buf).into_owned();
        if truncated {
            text.push_str(&format!("\n\n[обрезано: показаны первые {max} байт]"));
        }
        Ok(text)
    }

    async fn run(&self, name: &str, args: &Value) -> Result<String, String> {
        let state = self.state()?;
        let text = |v: Value| serde_json::to_string_pretty(&v).unwrap_or_default();
        match name {
            "list_hosts" => Ok(text(Self::list_hosts(&state))),
            "list_sessions" => Ok(text(Self::list_sessions(&state))),
            "read_output" => {
                let session = int_arg(args, "session")?;
                let lines = args
                    .get("lines")
                    .and_then(Value::as_u64)
                    .unwrap_or(200)
                    .clamp(1, 2000);
                state
                    .sessions
                    .recent_output(session, lines as usize)
                    .map_err(|e| e.message)
            }
            "read_selection" => Self::read_selection(&state).await,
            "sftp_list" => {
                let session = int_arg(args, "session")?;
                let path = str_arg(args, "path")?;
                let s = state.sessions.sftp(session).await.map_err(|e| e.message)?;
                let real = sftp::realpath(&s, &path).await.map_err(|e| e.message)?;
                let entries = sftp::list(&s, &real).await.map_err(|e| e.message)?;
                Ok(text(json!({ "path": real, "entries": entries })))
            }
            "sftp_read" => {
                let session = int_arg(args, "session")?;
                let path = str_arg(args, "path")?;
                let max = args
                    .get("max_bytes")
                    .and_then(Value::as_u64)
                    .unwrap_or(DEFAULT_READ_BYTES)
                    .clamp(1, MAX_READ_BYTES);
                Self::sftp_read(&state, session, &path, max)
                    .await
                    .map_err(|e| e.message)
            }
            "server_info" => {
                let session = int_arg(args, "session")?;
                let cmd = "sh -c 'uname -srm; (. /etc/os-release 2>/dev/null && echo \"$PRETTY_NAME\"); uptime'";
                let out = state
                    .sessions
                    .exec(session, cmd)
                    .await
                    .map_err(|e| e.message)?;
                Ok(out.stdout.trim().to_owned())
            }
            "exec" | "send_input" => {
                let session = int_arg(args, "session")?;
                let text = str_arg(args, if name == "exec" { "command" } else { "text" })?;
                let spec = state.sessions.spec(session).map_err(|e| e.message)?;
                if !Self::confirm(&state, session, &spec.label, name, &text).await {
                    return Err("Пользователь отклонил действие в termnest.".into());
                }
                if name == "exec" {
                    let out = state
                        .sessions
                        .exec(session, &text)
                        .await
                        .map_err(|e| e.message)?;
                    Ok(format!(
                        "exit code: {}\n--- stdout ---\n{}\n--- stderr ---\n{}",
                        out.exit_code
                            .map(|c| c.to_string())
                            .unwrap_or_else(|| "?".into()),
                        out.stdout.trim_end(),
                        out.stderr.trim_end()
                    ))
                } else {
                    let press_enter = args
                        .get("press_enter")
                        .and_then(Value::as_bool)
                        .unwrap_or(true);
                    let mut data = text.into_bytes();
                    if press_enter {
                        data.push(b'\r');
                    }
                    state.sessions.write(session, data).map_err(|e| e.message)?;
                    Ok("Отправлено в терминал. Результат смотрите через read_output.".into())
                }
            }
            _ => Err(format!("Неизвестный инструмент {name}")),
        }
    }

    /// Asks the user in termnest; no answer within two minutes means "no".
    async fn confirm(
        state: &AppState,
        session: SessionId,
        server: &str,
        action: &str,
        command: &str,
    ) -> bool {
        if lock(&state.mcp.trusted).contains(&session) {
            return true;
        }
        let request = PromptRequest::AgentAction {
            server: server.to_owned(),
            session,
            action: if action == "exec" {
                "exec".into()
            } else {
                "input".into()
            },
            command: command.to_owned(),
        };
        let reply = tokio::time::timeout(
            Duration::from_secs(120),
            termnest_core::prompt::ask(state.prompts.as_ref(), request),
        )
        .await;
        match reply {
            Ok(PromptReply::Confirm) => true,
            Ok(PromptReply::ConfirmSession) => {
                lock(&state.mcp.trusted).insert(session);
                true
            }
            _ => false,
        }
    }
}

impl ToolProvider for Tools {
    fn tools(&self) -> Vec<ToolDef> {
        vec![
            ToolDef {
                name: "list_hosts",
                description: "Saved SSH servers in termnest: name, user@host:port, group, linked local project.",
                input_schema: json!({ "type": "object", "properties": {} }),
            },
            ToolDef {
                name: "list_sessions",
                description: "Open terminal sessions: session id, server, state, linked local project, window and whether it is the focused panel.",
                input_schema: json!({ "type": "object", "properties": {} }),
            },
            ToolDef {
                name: "read_output",
                description: "Recent output of a terminal session as plain text (escape sequences removed).",
                input_schema: session_schema(json!({
                    "lines": { "type": "integer", "description": "Number of last lines, default 200, max 2000" }
                })),
            },
            ToolDef {
                name: "read_selection",
                description: "Text currently selected in a termnest terminal (empty if nothing is selected).",
                input_schema: json!({ "type": "object", "properties": {} }),
            },
            ToolDef {
                name: "sftp_list",
                description: "List a directory on the session's server over SFTP (same SSH connection).",
                input_schema: session_schema(json!({ "path": { "type": "string", "description": "Remote directory, e.g. /var/log" } })),
            },
            ToolDef {
                name: "sftp_read",
                description: "Read a text file on the session's server over SFTP, up to max_bytes (default 64 KiB, max 256 KiB).",
                input_schema: session_schema(json!({
                    "path": { "type": "string", "description": "Remote file path" },
                    "max_bytes": { "type": "integer" }
                })),
            },
            ToolDef {
                name: "server_info",
                description: "OS, kernel and uptime of the session's server (fixed read-only command).",
                input_schema: session_schema(json!({})),
            },
            ToolDef {
                name: "exec",
                description: "Run a command on the session's server in a separate channel and get its output. \
                    The user must approve it in termnest; it may be refused.",
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "session": { "type": "integer" },
                        "command": { "type": "string" }
                    },
                    "required": ["session", "command"]
                }),
            },
            ToolDef {
                name: "send_input",
                description: "Type text into the session's terminal (Enter is pressed unless press_enter is false). \
                    The user must approve it in termnest; it may be refused.",
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "session": { "type": "integer" },
                        "text": { "type": "string" },
                        "press_enter": { "type": "boolean" }
                    },
                    "required": ["session", "text"]
                }),
            },
        ]
    }

    fn call(&self, name: String, args: Value) -> BoxFuture<Result<String, String>> {
        let state = self.state.clone();
        Box::pin(async move {
            let tools = Tools { state };
            let session = args.get("session").and_then(Value::as_u64);
            let result = tools.run(&name, &args).await;
            if let Some(s) = tools.state.upgrade() {
                s.mcp.record(&name, session, result.is_ok());
            }
            result
        })
    }
}
