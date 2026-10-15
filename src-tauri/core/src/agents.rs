//! Claude Code agents started from termnest (SPEC 12.4-12.6): a task per error,
//! run with the Claude Code CLI in the project folder (or its own git worktree),
//! with the event stream parsed for the "Agents" panel.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::{oneshot, Semaphore};

use crate::error::{CoreError, CoreResult, ErrorKind};
use crate::paths::atomic_write;
use crate::session::SessionId;
use crate::settings::AgentMode;

pub type AgentId = String;

const MAX_EVENTS: usize = 600;
const MAX_EVENT_TEXT: usize = 4000;

/// termnest renders agent messages as GitHub-flavored Markdown (HTML is stripped).
pub const OUTPUT_FORMAT_PROMPT: &str = "Your messages are shown in the termnest app, which renders \
GitHub-flavored Markdown only. Format every reply as Markdown: headings, lists, `inline code`, fenced \
code blocks with a language tag for commands and code, tables when useful. Do not use HTML tags. \
Reply in the language the user writes in.";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentState {
    Queued,
    Running,
    Done,
    Failed,
    Stopped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    /// What the user sent.
    Prompt,
    /// Agent's text.
    Text,
    /// A tool call (file read/edit, MCP call, ...).
    Tool,
    ToolError,
    /// Final answer of a run.
    Result,
    Error,
    Info,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentEvent {
    pub time: u64,
    pub kind: EventKind,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentTask {
    pub id: AgentId,
    pub title: String,
    /// Linked project; `None` for a plain question.
    #[serde(default)]
    pub project: Option<String>,
    /// A question: the agent answers and must not change anything.
    #[serde(default)]
    pub ask: bool,
    /// Where the agent works: the project folder, its worktree, or an empty folder.
    pub cwd: String,
    #[serde(default)]
    pub worktree: Option<String>,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub server: Option<String>,
    #[serde(default)]
    pub session: Option<SessionId>,
    pub state: AgentState,
    pub created: u64,
    pub updated: u64,
    /// Claude Code conversation id, used to continue it.
    #[serde(default)]
    pub claude_session: Option<String>,
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(default)]
    pub result: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub cost_usd: Option<f64>,
    #[serde(default)]
    pub events: Vec<AgentEvent>,
}

/// Task without the event log, for lists and update notifications.
#[derive(Debug, Clone, Serialize)]
pub struct AgentSummary {
    pub id: AgentId,
    pub title: String,
    pub project: Option<String>,
    pub ask: bool,
    pub cwd: String,
    pub worktree: Option<String>,
    pub branch: Option<String>,
    pub server: Option<String>,
    pub session: Option<SessionId>,
    pub state: AgentState,
    pub created: u64,
    pub updated: u64,
    pub files: Vec<String>,
    pub result: Option<String>,
    pub error: Option<String>,
    pub cost_usd: Option<f64>,
    pub can_continue: bool,
}

impl AgentTask {
    pub fn summary(&self) -> AgentSummary {
        AgentSummary {
            id: self.id.clone(),
            title: self.title.clone(),
            project: self.project.clone(),
            ask: self.ask,
            cwd: self.cwd.clone(),
            worktree: self.worktree.clone(),
            branch: self.branch.clone(),
            server: self.server.clone(),
            session: self.session,
            state: self.state,
            created: self.created,
            updated: self.updated,
            files: self.files.clone(),
            result: self.result.clone(),
            error: self.error.clone(),
            cost_usd: self.cost_usd,
            can_continue: self.claude_session.is_some(),
        }
    }
}

pub trait AgentSink: Send + Sync + 'static {
    fn agent_changed(&self, task: &AgentSummary, event: Option<&AgentEvent>);
}

/// How to run the CLI; read from the settings at start.
#[derive(Debug, Clone)]
pub struct AgentConfig {
    pub cli: String,
    pub mode: AgentMode,
    /// MCP config file connecting the agent to termnest, if the server is on.
    pub mcp_config: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct AgentRequest {
    pub title: String,
    /// `None` (or `ask`) runs a question in an empty folder.
    pub project: Option<String>,
    pub ask: bool,
    pub prompt: String,
    pub server: Option<String>,
    pub session: Option<SessionId>,
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let mut s: String = text.chars().take(max).collect();
    s.push_str(" ...");
    s
}

// ---- Stream parsing ---------------------------------------------------------------------

#[derive(Debug, PartialEq)]
pub enum Parsed {
    SessionId(String),
    Event(EventKind, String),
    FileChanged(String),
    Result {
        text: String,
        is_error: bool,
        cost: Option<f64>,
    },
}

fn tool_summary(name: &str, input: &Value) -> String {
    let field = |k: &str| input.get(k).and_then(Value::as_str).map(str::to_owned);
    let detail = match name {
        "Read" | "Edit" | "Write" | "MultiEdit" | "NotebookEdit" => {
            field("file_path").or_else(|| field("notebook_path"))
        }
        "Bash" | "PowerShell" => field("command"),
        "Grep" | "Glob" => field("pattern"),
        "WebFetch" => field("url"),
        _ => None,
    };
    match detail {
        Some(d) => format!("{name}: {}", clip(&d, 300)),
        None => name.to_owned(),
    }
}

/// Parses one line of `--output-format stream-json`.
pub fn parse_line(line: &str) -> Vec<Parsed> {
    let Ok(v) = serde_json::from_str::<Value>(line.trim()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    match v.get("type").and_then(Value::as_str) {
        Some("system") => {
            if let Some(id) = v.get("session_id").and_then(Value::as_str) {
                out.push(Parsed::SessionId(id.to_owned()));
            }
        }
        Some("assistant") => {
            let content = v.pointer("/message/content").and_then(Value::as_array);
            for block in content.into_iter().flatten() {
                match block.get("type").and_then(Value::as_str) {
                    Some("text") => {
                        let text = block
                            .get("text")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .trim();
                        if !text.is_empty() {
                            out.push(Parsed::Event(EventKind::Text, text.to_owned()));
                        }
                    }
                    Some("tool_use") => {
                        let name = block.get("name").and_then(Value::as_str).unwrap_or("tool");
                        let input = block.get("input").cloned().unwrap_or(Value::Null);
                        if matches!(name, "Edit" | "Write" | "MultiEdit" | "NotebookEdit") {
                            if let Some(p) = input
                                .get("file_path")
                                .or_else(|| input.get("notebook_path"))
                                .and_then(Value::as_str)
                            {
                                out.push(Parsed::FileChanged(p.to_owned()));
                            }
                        }
                        out.push(Parsed::Event(EventKind::Tool, tool_summary(name, &input)));
                    }
                    _ => {}
                }
            }
        }
        Some("user") => {
            let content = v.pointer("/message/content").and_then(Value::as_array);
            for block in content.into_iter().flatten() {
                if block.get("type").and_then(Value::as_str) == Some("tool_result")
                    && block.get("is_error").and_then(Value::as_bool) == Some(true)
                {
                    let text = match block.get("content") {
                        Some(Value::String(s)) => s.clone(),
                        Some(other) => other.to_string(),
                        None => String::new(),
                    };
                    out.push(Parsed::Event(EventKind::ToolError, clip(&text, 500)));
                }
            }
        }
        Some("result") => {
            if let Some(id) = v.get("session_id").and_then(Value::as_str) {
                out.push(Parsed::SessionId(id.to_owned()));
            }
            let is_error = v.get("is_error").and_then(Value::as_bool).unwrap_or(false)
                || v.get("subtype")
                    .and_then(Value::as_str)
                    .is_some_and(|s| s != "success");
            let text = v
                .get("result")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .unwrap_or_else(|| {
                    v.get("subtype")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_owned()
                });
            out.push(Parsed::Result {
                text,
                is_error,
                cost: v.get("total_cost_usd").and_then(Value::as_f64),
            });
        }
        _ => {}
    }
    out
}

// ---- Process helpers --------------------------------------------------------------------

fn command(program: &str) -> Command {
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    // A Claude Code session that started termnest must not leak into the agent.
    for var in [
        "CLAUDECODE",
        "CLAUDE_CODE_ENTRYPOINT",
        "CLAUDE_CODE_SSE_PORT",
    ] {
        cmd.env_remove(var);
    }
    cmd
}

/// What to do when the CLI is not logged in.
pub fn auth_hint(text: &str) -> Option<String> {
    let lower = text.to_lowercase();
    let auth = [
        "authenticate",
        "oauth",
        "not logged in",
        "login",
        "api key",
        "unauthorized",
    ]
    .iter()
    .any(|m| lower.contains(m));
    auth.then(|| {
        "Claude Code не авторизован: в терминале Windows выполните `claude`, внутри - `/login`, затем повторите запрос.".to_owned()
    })
}

/// Stops the CLI together with any processes it started (a `.cmd` wrapper, node, tools).
async fn kill_tree(child: &mut tokio::process::Child) {
    #[cfg(windows)]
    if let Some(pid) = child.id() {
        let _ = command("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await;
    }
    let _ = child.kill().await;
}

async fn git(dir: &Path, args: &[&str]) -> CoreResult<String> {
    let out = command("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .await
        .map_err(|e| CoreError::new(ErrorKind::NotFound, format!("git не найден: {e}")))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(CoreError::new(
            ErrorKind::Remote,
            format!(
                "git {}: {}",
                args.join(" "),
                String::from_utf8_lossy(&out.stderr).trim()
            ),
        ))
    }
}

/// Checks that the CLI starts; returns its version line.
pub async fn cli_version(cli: &str) -> CoreResult<String> {
    let out = command(cli)
        .arg("--version")
        .stdin(Stdio::null())
        .output()
        .await
        .map_err(|e| {
            CoreError::new(
                ErrorKind::NotFound,
                format!("Claude Code CLI не найден ({cli}): {e}. Установите Claude Code или укажите путь в настройках."),
            )
        })?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

// ---- Manager ----------------------------------------------------------------------------

pub struct AgentManager {
    dir: PathBuf,
    tasks: Mutex<BTreeMap<AgentId, AgentTask>>,
    stops: Mutex<HashMap<AgentId, oneshot::Sender<()>>>,
    /// One agent at a time per folder that is not a separate worktree.
    folder_locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    limit: Arc<Semaphore>,
    limit_size: Mutex<usize>,
    sink: Arc<dyn AgentSink>,
    runtime: tokio::runtime::Handle,
}

impl AgentManager {
    /// Loads saved tasks from `dir`; tasks that were running when the app exited are
    /// marked as stopped.
    pub fn load(
        dir: PathBuf,
        max_agents: usize,
        sink: Arc<dyn AgentSink>,
        runtime: tokio::runtime::Handle,
    ) -> Arc<Self> {
        let mut tasks = BTreeMap::new();
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for entry in rd.filter_map(|e| e.ok()) {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("json") {
                    continue;
                }
                let Ok(text) = std::fs::read_to_string(&path) else {
                    continue;
                };
                let Ok(mut task) = serde_json::from_str::<AgentTask>(&text) else {
                    continue;
                };
                if matches!(task.state, AgentState::Running | AgentState::Queued) {
                    task.state = AgentState::Stopped;
                    task.error = Some("Прервано: termnest был закрыт".into());
                }
                tasks.insert(task.id.clone(), task);
            }
        }
        let max = max_agents.max(1);
        Arc::new(Self {
            dir,
            tasks: Mutex::new(tasks),
            stops: Mutex::new(HashMap::new()),
            folder_locks: Mutex::new(HashMap::new()),
            limit: Arc::new(Semaphore::new(max)),
            limit_size: Mutex::new(max),
            sink,
            runtime,
        })
    }

    pub fn set_limit(&self, max: usize) {
        let max = max.max(1);
        let mut size = lock(&self.limit_size);
        if max > *size {
            self.limit.add_permits(max - *size);
        } else if max < *size {
            self.limit.forget_permits(*size - max);
        }
        *size = max;
    }

    pub fn list(&self) -> Vec<AgentSummary> {
        let mut all: Vec<_> = lock(&self.tasks).values().map(AgentTask::summary).collect();
        all.sort_by_key(|t| std::cmp::Reverse(t.created));
        all
    }

    pub fn get(&self, id: &str) -> CoreResult<AgentTask> {
        lock(&self.tasks)
            .get(id)
            .cloned()
            .ok_or_else(|| CoreError::new(ErrorKind::NotFound, "Задача агента не найдена"))
    }

    fn save(&self, id: &str) {
        let Some(task) = lock(&self.tasks).get(id).cloned() else {
            return;
        };
        if let Ok(text) = serde_json::to_string_pretty(&task) {
            let _ = atomic_write(&self.dir.join(format!("{id}.json")), text.as_bytes());
        }
    }

    /// Updates a task, notifies the UI and optionally appends an event.
    fn update(&self, id: &str, f: impl FnOnce(&mut AgentTask), event: Option<(EventKind, String)>) {
        let (summary, ev) = {
            let mut tasks = lock(&self.tasks);
            let Some(task) = tasks.get_mut(id) else {
                return;
            };
            f(task);
            task.updated = now();
            let ev = event.map(|(kind, text)| {
                let e = AgentEvent {
                    time: task.updated,
                    kind,
                    text: clip(&text, MAX_EVENT_TEXT),
                };
                task.events.push(e.clone());
                if task.events.len() > MAX_EVENTS {
                    let extra = task.events.len() - MAX_EVENTS;
                    task.events.drain(..extra);
                }
                e
            });
            (task.summary(), ev)
        };
        self.sink.agent_changed(&summary, ev.as_ref());
    }

    fn set_state(&self, id: &str, state: AgentState) {
        self.update(id, |t| t.state = state, None);
        self.save(id);
    }

    fn is_running(&self, id: &str) -> bool {
        lock(&self.tasks)
            .get(id)
            .is_some_and(|t| matches!(t.state, AgentState::Running | AgentState::Queued))
    }

    /// Creates the task (with its own worktree when the project is a git repository)
    /// and starts the agent.
    pub async fn start(
        self: &Arc<Self>,
        req: AgentRequest,
        cfg: AgentConfig,
    ) -> CoreResult<AgentId> {
        if let Some(p) = &req.project {
            if !Path::new(p).is_dir() {
                return Err(CoreError::new(
                    ErrorKind::NotFound,
                    format!("Папка проекта не найдена: {p}"),
                ));
            }
        }
        cli_version(&cfg.cli).await?;

        let id: AgentId = {
            let full = crate::ids::new_id();
            full[full.len() - 8..].to_owned()
        };
        let mut info = None;
        let ask = req.ask || req.project.is_none();
        let (cwd, worktree, branch) = if ask {
            // A question needs no code: the agent works in an empty folder of its own.
            let path = self.dir.join("scratch").join(&id);
            std::fs::create_dir_all(&path)
                .map_err(|e| CoreError::storage("Не удалось создать папку агента", e))?;
            (path.display().to_string(), None, None)
        } else if let Some(project_s) = req
            .project
            .clone()
            .filter(|p| Path::new(p).join(".git").exists())
        {
            let project = PathBuf::from(&project_s);
            let branch = format!("termnest/fix-{id}");
            let path = self.dir.join("worktrees").join(&id);
            let path_s = path.display().to_string();
            match git(
                &project,
                &["worktree", "add", "-b", &branch, &path_s, "HEAD"],
            )
            .await
            {
                Ok(_) => (path_s.clone(), Some(path_s), Some(branch)),
                Err(e) => {
                    info = Some(format!(
                        "Worktree не создан ({}), агент работает прямо в папке проекта",
                        e.message
                    ));
                    (project_s, None, None)
                }
            }
        } else {
            info = Some("Проект не git-репозиторий: агент работает прямо в папке, параллельно с другими не запускается".into());
            (req.project.clone().unwrap_or_default(), None, None)
        };

        let created = now();
        let task = AgentTask {
            id: id.clone(),
            title: req.title,
            project: req.project,
            ask,
            cwd,
            worktree,
            branch,
            server: req.server,
            session: req.session,
            state: AgentState::Queued,
            created,
            updated: created,
            claude_session: None,
            files: Vec::new(),
            result: None,
            error: None,
            cost_usd: None,
            events: Vec::new(),
        };
        lock(&self.tasks).insert(id.clone(), task);
        if let Some(text) = info {
            self.update(&id, |_| {}, Some((EventKind::Info, text)));
        }
        self.spawn_run(id.clone(), req.prompt, cfg);
        Ok(id)
    }

    /// Continues the same conversation with a follow-up message.
    pub fn follow_up(self: &Arc<Self>, id: &str, text: String, cfg: AgentConfig) -> CoreResult<()> {
        let task = self.get(id)?;
        if self.is_running(id) {
            return Err(CoreError::new(
                ErrorKind::InvalidInput,
                "Агент ещё работает",
            ));
        }
        if task.claude_session.is_none() {
            return Err(CoreError::new(
                ErrorKind::InvalidInput,
                "Разговор не начался, продолжить нельзя",
            ));
        }
        self.update(
            id,
            |t| {
                t.state = AgentState::Queued;
                t.error = None;
            },
            None,
        );
        self.spawn_run(id.to_owned(), text, cfg);
        Ok(())
    }

    pub fn stop(&self, id: &str) {
        if let Some(tx) = lock(&self.stops).remove(id) {
            let _ = tx.send(());
        }
    }

    /// Forgets a finished task and its log (the worktree is kept unless removed).
    pub fn delete(&self, id: &str) -> CoreResult<()> {
        if self.is_running(id) {
            return Err(CoreError::new(
                ErrorKind::InvalidInput,
                "Сначала остановите агента",
            ));
        }
        lock(&self.tasks).remove(id);
        let _ = std::fs::remove_file(self.dir.join(format!("{id}.json")));
        Ok(())
    }

    /// Changes made by the agent: `git diff` plus untracked files.
    pub async fn diff(&self, id: &str) -> CoreResult<String> {
        let task = self.get(id)?;
        let cwd = PathBuf::from(&task.cwd);
        if !cwd.join(".git").exists() {
            return Ok("Папка не является git-репозиторием: diff недоступен.".into());
        }
        let mut out = git(&cwd, &["diff", "HEAD"]).await?;
        let untracked = git(&cwd, &["ls-files", "--others", "--exclude-standard"]).await?;
        if !untracked.trim().is_empty() {
            out.push_str("\n# Новые файлы:\n");
            for f in untracked.lines() {
                out.push_str(&format!("#   {f}\n"));
            }
        }
        if out.trim().is_empty() {
            out = "Изменений нет.".into();
        }
        Ok(out)
    }

    /// Removes the task's worktree (the branch stays).
    pub async fn remove_worktree(&self, id: &str) -> CoreResult<()> {
        if self.is_running(id) {
            return Err(CoreError::new(
                ErrorKind::InvalidInput,
                "Сначала остановите агента",
            ));
        }
        let task = self.get(id)?;
        let (Some(wt), Some(project)) = (task.worktree.clone(), task.project.clone()) else {
            return Ok(());
        };
        git(Path::new(&project), &["worktree", "remove", "--force", &wt]).await?;
        self.update(
            id,
            |t| {
                t.worktree = None;
                t.cwd = project.clone();
            },
            Some((
                EventKind::Info,
                format!(
                    "Worktree удалён, ветка {} сохранена",
                    task.branch.unwrap_or_default()
                ),
            )),
        );
        self.save(id);
        Ok(())
    }

    fn folder_lock(&self, cwd: &str) -> Arc<tokio::sync::Mutex<()>> {
        lock(&self.folder_locks)
            .entry(cwd.to_lowercase())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    }

    fn spawn_run(self: &Arc<Self>, id: AgentId, prompt: String, cfg: AgentConfig) {
        let (tx, rx) = oneshot::channel();
        lock(&self.stops).insert(id.clone(), tx);
        let this = self.clone();
        self.runtime.spawn(async move {
            this.run(id.clone(), prompt, cfg, rx).await;
            lock(&this.stops).remove(&id);
            this.save(&id);
        });
    }

    async fn run(
        self: &Arc<Self>,
        id: AgentId,
        prompt: String,
        cfg: AgentConfig,
        mut stop: oneshot::Receiver<()>,
    ) {
        let Ok(task) = self.get(&id) else { return };
        self.update(
            &id,
            |t| t.state = AgentState::Queued,
            Some((EventKind::Prompt, prompt.clone())),
        );

        // Wait for a free slot (and for the folder when there is no worktree).
        let limit = self.limit.clone();
        let permit = tokio::select! {
            p = limit.acquire_owned() => p.ok(),
            _ = &mut stop => None,
        };
        let Some(_permit) = permit else {
            self.update(
                &id,
                |t| t.state = AgentState::Stopped,
                Some((EventKind::Info, "Остановлено в очереди".into())),
            );
            return;
        };
        let folder = if task.worktree.is_none() {
            Some(self.folder_lock(&task.cwd))
        } else {
            None
        };
        let _folder_guard = match &folder {
            Some(lock) => tokio::select! {
                g = lock.lock() => Some(g),
                _ = &mut stop => {
                    self.update(&id, |t| t.state = AgentState::Stopped, Some((EventKind::Info, "Остановлено в очереди".into())));
                    return;
                }
            },
            None => None,
        };

        self.set_state(&id, AgentState::Running);
        let mut cmd = command(&cfg.cli);
        cmd.arg("-p")
            .args(["--output-format", "stream-json", "--verbose"])
            .arg("--permission-mode")
            .arg(match (task.ask, cfg.mode) {
                // A question: nothing that needs permission is allowed.
                (true, _) => "dontAsk",
                (false, AgentMode::Edit) => "acceptEdits",
                (false, AgentMode::Propose) => "plan",
            })
            .arg("--append-system-prompt")
            .arg(OUTPUT_FORMAT_PROMPT);
        if let Some(sid) = &task.claude_session {
            cmd.arg("--resume").arg(sid);
        }
        if let Some(mcp) = &cfg.mcp_config {
            cmd.arg("--mcp-config")
                .arg(mcp)
                .arg("--allowedTools")
                .arg("mcp__termnest");
        }
        cmd.current_dir(&task.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                self.update(
                    &id,
                    |t| {
                        t.state = AgentState::Failed;
                        t.error = Some(format!("Не удалось запустить Claude Code: {e}"));
                    },
                    None,
                );
                return;
            }
        };
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(prompt.as_bytes()).await;
            let _ = stdin.shutdown().await;
        }
        let stdout = child.stdout.take();
        let mut stderr = child.stderr.take();
        let stderr_task = tokio::spawn(async move {
            let mut buf = Vec::new();
            if let Some(err) = stderr.as_mut() {
                let _ = err.read_to_end(&mut buf).await;
            }
            String::from_utf8_lossy(&buf).into_owned()
        });

        let mut final_result: Option<(String, bool)> = None;
        let mut stopped = false;
        if let Some(out) = stdout {
            let mut lines = BufReader::new(out).lines();
            loop {
                tokio::select! {
                    line = lines.next_line() => match line {
                        Ok(Some(line)) => {
                            for p in parse_line(&line) {
                                match p {
                                    Parsed::SessionId(sid) => self.update(&id, |t| t.claude_session = Some(sid), None),
                                    Parsed::FileChanged(f) => self.update(&id, |t| {
                                        if !t.files.contains(&f) {
                                            t.files.push(f);
                                        }
                                    }, None),
                                    Parsed::Event(kind, text) => self.update(&id, |_| {}, Some((kind, text))),
                                    Parsed::Result { text, is_error, cost } => {
                                        self.update(&id, |t| {
                                            t.result = Some(text.clone());
                                            t.cost_usd = Some(t.cost_usd.unwrap_or(0.0) + cost.unwrap_or(0.0));
                                        }, Some((if is_error { EventKind::Error } else { EventKind::Result }, text.clone())));
                                        final_result = Some((text, is_error));
                                    }
                                }
                            }
                        }
                        _ => break,
                    },
                    _ = &mut stop => {
                        stopped = true;
                        kill_tree(&mut child).await;
                        break;
                    }
                }
            }
        }
        let status = child.wait().await.ok();
        // A leftover grandchild may keep stderr open: do not wait for it forever.
        let stderr_text =
            match tokio::time::timeout(std::time::Duration::from_secs(3), stderr_task).await {
                Ok(Ok(text)) => text,
                _ => String::new(),
            };

        if stopped {
            self.update(
                &id,
                |t| t.state = AgentState::Stopped,
                Some((EventKind::Info, "Остановлено пользователем".into())),
            );
            return;
        }
        match final_result {
            Some((_, false)) => self.update(
                &id,
                |t| {
                    t.state = AgentState::Done;
                    t.error = None;
                },
                None,
            ),
            Some((text, true)) => {
                let hint = auth_hint(&text);
                let error = match &hint {
                    Some(h) => format!("{text}\n{h}"),
                    None => text.clone(),
                };
                self.update(
                    &id,
                    |t| {
                        t.state = AgentState::Failed;
                        t.error = Some(error);
                    },
                    hint.map(|h| (EventKind::Info, h)),
                )
            }
            None => {
                let mut msg = stderr_text.trim().to_owned();
                if msg.is_empty() {
                    msg = format!(
                        "Claude Code завершился без результата (код {})",
                        status
                            .and_then(|s| s.code())
                            .map(|c| c.to_string())
                            .unwrap_or_else(|| "?".into())
                    );
                }
                self.update(
                    &id,
                    |t| {
                        t.state = AgentState::Failed;
                        t.error = Some(clip(&msg, 1500));
                    },
                    Some((EventKind::Error, clip(&msg, 1500))),
                );
            }
        }
    }
}

// ---- Prompt helpers ---------------------------------------------------------------------

/// Lines that look like they may contain secrets, masked for the warning.
pub fn secret_lines(text: &str) -> Vec<String> {
    const MARKERS: [&str; 12] = [
        "password",
        "passwd",
        "secret",
        "token",
        "api_key",
        "apikey",
        "api-key",
        "authorization:",
        "bearer ",
        "-----begin",
        "private_key",
        "aws_secret",
    ];
    let mut out = Vec::new();
    for line in text.lines() {
        let lower = line.to_lowercase();
        let marker = MARKERS.iter().any(|m| lower.contains(m));
        let long_blob = line.split_whitespace().any(|w| {
            w.len() >= 32
                && w.chars()
                    .all(|c| c.is_ascii_alphanumeric() || "+/=_-".contains(c))
                && w.chars().any(|c| c.is_ascii_digit())
        });
        if marker || long_blob {
            let shown: String = line.trim().chars().take(80).collect();
            out.push(shown);
        }
        if out.len() >= 20 {
            break;
        }
    }
    out
}

/// Text the agent receives for a terminal: an error to fix in a project, or a question.
pub struct PromptInput<'a> {
    pub server: &'a str,
    pub target: &'a str,
    pub os: Option<&'a str>,
    pub cwd: Option<&'a str>,
    /// The user's comment (fix) or question (ask).
    pub comment: &'a str,
    pub output: &'a str,
    pub session: Option<SessionId>,
    pub mcp: bool,
    pub mode: AgentMode,
    /// A question without a project: answer, change nothing.
    pub ask: bool,
}

pub fn build_prompt(p: &PromptInput<'_>) -> String {
    let mut s = String::new();
    let mut server = format!("{} ({})", p.server, p.target);
    if let Some(os) = p.os.filter(|o| !o.is_empty()) {
        server.push_str(&format!(", {os}"));
    }
    if p.ask {
        s.push_str(&format!("Вопрос о терминале на сервере {server}.\n"));
        let question = p.comment.trim();
        s.push_str(&format!(
            "\nВопрос пользователя: {}\n",
            if question.is_empty() {
                "что здесь происходит и что с этим делать?"
            } else {
                question
            }
        ));
    } else {
        s.push_str(&format!(
            "В терминале на сервере {server} возникла ошибка. Текущая папка - локальный проект с кодом этого сервиса.\n"
        ));
        if !p.comment.trim().is_empty() {
            s.push_str(&format!(
                "\nКомментарий пользователя: {}\n",
                p.comment.trim()
            ));
        }
    }
    s.push_str("\nВывод терминала");
    if let Some(cwd) = p.cwd.filter(|c| !c.is_empty()) {
        s.push_str(&format!(" (папка на сервере: {cwd})"));
    }
    s.push_str(":\n```\n");
    s.push_str(p.output.trim_end());
    s.push_str("\n```\n\n");
    if p.ask {
        s.push_str("Ответь на вопрос, опираясь на вывод терминала. Ничего не меняй и не выполняй на сервере: если нужна команда, просто предложи её.");
    } else {
        match p.mode {
            AgentMode::Edit => s.push_str("Задача: найди причину ошибки в коде проекта и исправь её в файлах проекта."),
            AgentMode::Propose => {
                s.push_str("Задача: найди причину ошибки в коде проекта и предложи исправление (diff), файлы не меняй.")
            }
        }
        s.push_str(" Ничего не выполняй на сервере.");
    }
    if p.mcp {
        if let Some(id) = p.session {
            s.push_str(&format!(
                " Для дополнительного контекста можно использовать инструменты termnest (MCP): read_output для сессии {id}, sftp_read, sftp_list, server_info."
            ));
        }
    }
    if !p.ask {
        s.push_str("\nВ конце кратко опиши причину и что изменено.");
    }
    s.push('\n');
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_stream_json() {
        let init = r#"{"type":"system","subtype":"init","session_id":"abc","cwd":"D:\\p"}"#;
        assert_eq!(parse_line(init), vec![Parsed::SessionId("abc".into())]);

        let assistant = r#"{"type":"assistant","message":{"content":[
            {"type":"text","text":"Смотрю конфиг"},
            {"type":"tool_use","id":"1","name":"Edit","input":{"file_path":"src/app.py","old_string":"a","new_string":"b"}},
            {"type":"tool_use","id":"2","name":"Bash","input":{"command":"pytest -q"}}]}}"#
            .replace('\n', "");
        assert_eq!(
            parse_line(&assistant),
            vec![
                Parsed::Event(EventKind::Text, "Смотрю конфиг".into()),
                Parsed::FileChanged("src/app.py".into()),
                Parsed::Event(EventKind::Tool, "Edit: src/app.py".into()),
                Parsed::Event(EventKind::Tool, "Bash: pytest -q".into()),
            ]
        );

        let tool_err = r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"2","content":"denied","is_error":true}]}}"#;
        assert_eq!(
            parse_line(tool_err),
            vec![Parsed::Event(EventKind::ToolError, "denied".into())]
        );

        let result = r#"{"type":"result","subtype":"success","is_error":false,"result":"Готово","session_id":"abc","total_cost_usd":0.05}"#;
        assert_eq!(
            parse_line(result),
            vec![
                Parsed::SessionId("abc".into()),
                Parsed::Result {
                    text: "Готово".into(),
                    is_error: false,
                    cost: Some(0.05)
                }
            ]
        );
        assert!(parse_line("not json").is_empty());
    }

    #[test]
    fn auth_errors_get_a_login_hint() {
        assert!(auth_hint(
            "Failed to authenticate: OAuth session expired and could not be refreshed"
        )
        .is_some());
        assert!(auth_hint("Tool failed: permission denied").is_none());
    }

    #[test]
    fn detects_secret_like_lines() {
        let text = "ok line\nexport API_KEY=abc\nAuthorization: Bearer x\nsha 3f8a9c0d1e2b4f6a7c8d9e0f1a2b3c4d5e6f7a8b\nplain words here";
        let found = secret_lines(text);
        assert_eq!(found.len(), 3);
        assert!(found[0].contains("API_KEY"));
    }

    #[test]
    fn prompt_mentions_mcp_only_when_enabled() {
        let mut p = PromptInput {
            server: "web",
            target: "root@web:22",
            os: Some("Ubuntu 24.04"),
            cwd: Some("/srv/app"),
            comment: "после деплоя",
            output: "Traceback...\nKeyError: 'x'\n",
            session: Some(3),
            mcp: false,
            mode: AgentMode::Edit,
            ask: false,
        };
        let text = build_prompt(&p);
        assert!(text.contains("(root@web:22), Ubuntu 24.04"));
        assert!(text.contains("KeyError: 'x'"));
        assert!(text.contains("после деплоя"));
        assert!(!text.contains("read_output"));
        p.mcp = true;
        assert!(build_prompt(&p).contains("read_output для сессии 3"));
        p.ask = true;
        p.comment = "почему nginx не стартует?";
        let q = build_prompt(&p);
        assert!(q.starts_with("Вопрос о терминале"));
        assert!(q.contains("Вопрос пользователя: почему nginx не стартует?"));
        assert!(q.contains("Ничего не меняй"));
        assert!(!q.contains("исправь"));
    }

    #[test]
    fn loads_and_marks_interrupted_tasks() {
        let dir = crate::paths::test_dir::TestDir::new();
        let task = AgentTask {
            id: "t1".into(),
            title: "x".into(),
            project: Some("p".into()),
            ask: false,
            cwd: "p".into(),
            worktree: None,
            branch: None,
            server: None,
            session: None,
            state: AgentState::Running,
            created: 1,
            updated: 1,
            claude_session: Some("s".into()),
            files: vec![],
            result: None,
            error: None,
            cost_usd: None,
            events: vec![],
        };
        std::fs::write(
            dir.path().join("t1.json"),
            serde_json::to_string(&task).unwrap(),
        )
        .unwrap();
        struct Nop;
        impl AgentSink for Nop {
            fn agent_changed(&self, _: &AgentSummary, _: Option<&AgentEvent>) {}
        }
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let m = AgentManager::load(
            dir.path().to_path_buf(),
            2,
            Arc::new(Nop),
            rt.handle().clone(),
        );
        let list = m.list();
        assert_eq!(list[0].state, AgentState::Stopped);
        assert!(list[0].can_continue);
    }
}
