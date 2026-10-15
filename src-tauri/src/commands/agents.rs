//! "Send to Claude" and the Agents panel (SPEC 12.4-12.6).

use std::time::Duration;

use serde::Serialize;
use termnest_core::agents::{
    self, AgentConfig, AgentId, AgentRequest, AgentSummary, AgentTask, PromptInput,
};
use termnest_core::layout::PaneId;
use termnest_core::project::{self, ProjectInfo};
use termnest_core::{CoreError, CoreResult, ErrorKind};

use super::AppStateRef;
use crate::state::AppState;

#[derive(Serialize)]
pub struct Prepared {
    pub project: Option<ProjectInfo>,
    pub server: String,
    pub title: String,
    pub prompt: String,
    /// Lines that look like secrets (shown as a warning before sending).
    pub secrets: Vec<String>,
    pub mcp: bool,
}

/// Settings-derived run configuration; writes the MCP config for the agent when the
/// termnest MCP server is running.
async fn agent_config(state: &AppState) -> CoreResult<AgentConfig> {
    let claude = state.settings().claude;
    let status = state.mcp.status(state).await;
    let mcp_config = if status.running {
        let path = state.dir.join("agents").join("mcp-config.json");
        let config = serde_json::json!({
            "mcpServers": {
                "termnest": {
                    "type": "http",
                    "url": format!("http://127.0.0.1:{}/mcp", status.port),
                    "headers": { "Authorization": format!("Bearer {}", status.token) }
                }
            }
        });
        termnest_core::paths::atomic_write(&path, config.to_string().as_bytes())
            .map_err(|e| CoreError::storage("Не удалось записать MCP-конфиг агента", e))?;
        let _ = termnest_core::keys::restrict_to_owner(&path);
        Some(path)
    } else {
        None
    };
    Ok(AgentConfig {
        cli: claude.cli_path.unwrap_or_else(|| "claude".into()),
        mode: claude.agent_mode,
        mcp_config,
    })
}

/// Builds the request text for an error in a panel (shown to the user before sending).
#[tauri::command]
pub async fn agent_prepare(
    state: AppStateRef<'_>,
    pane: PaneId,
    selection: Option<String>,
    cwd: Option<String>,
    comment: String,
) -> Result<Prepared, CoreError> {
    let (spec, session, pane_project) = {
        let layout = state.layout();
        let p = layout
            .pane(pane)
            .ok_or_else(|| CoreError::new(ErrorKind::NotFound, "Панель не найдена"))?;
        (p.spec.clone(), p.session, p.project.clone())
    };
    let spec =
        spec.ok_or_else(|| CoreError::new(ErrorKind::InvalidInput, "В панели нет подключения"))?;
    let hosts = state.hosts.list();
    let host = spec
        .host_id
        .as_ref()
        .and_then(|id| hosts.hosts.iter().find(|h| &h.id == id));
    let project = project::resolve(pane_project.as_deref(), host, &hosts.groups).map(project::info);

    let settings = state.settings().claude;
    let output = match selection.filter(|s| !s.trim().is_empty()) {
        Some(s) => s,
        None => match session {
            Some(id) => state
                .sessions
                .recent_output(id, settings.context_lines as usize)?,
            None => String::new(),
        },
    };
    // OS name for context; a slow or failing server must not block the dialog.
    let os = match session {
        Some(id) => {
            let cmd =
                "sh -c '(. /etc/os-release 2>/dev/null && echo \"$PRETTY_NAME\") || uname -sr'";
            match tokio::time::timeout(Duration::from_secs(4), state.sessions.exec(id, cmd)).await {
                Ok(Ok(out)) => Some(out.stdout.trim().to_owned()),
                _ => None,
            }
        }
        None => None,
    };
    let mcp = state.mcp.status(&state).await.running;
    let target = format!("{}@{}:{}", spec.user, spec.host, spec.port);
    let prompt = agents::build_prompt(&PromptInput {
        server: &spec.label,
        target: &target,
        os: os.as_deref(),
        cwd: cwd.as_deref(),
        comment: &comment,
        output: &output,
        session,
        mcp,
        mode: settings.agent_mode,
    });
    let title = output
        .lines()
        .rev()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.ends_with('$') && !l.ends_with('#'))
        .map(|l| l.chars().take(90).collect::<String>())
        .unwrap_or_else(|| format!("Ошибка на {}", spec.label));
    Ok(Prepared {
        secrets: agents::secret_lines(&output),
        project,
        server: spec.label.clone(),
        title,
        prompt,
        mcp,
    })
}

#[tauri::command]
pub async fn agent_start(
    state: AppStateRef<'_>,
    pane: PaneId,
    project: String,
    title: String,
    prompt: String,
) -> Result<AgentId, CoreError> {
    let (server, session) = {
        let layout = state.layout();
        let p = layout.pane(pane);
        (
            p.and_then(|p| p.spec.as_ref().map(|s| s.label.clone())),
            p.and_then(|p| p.session),
        )
    };
    let cfg = agent_config(&state).await?;
    state
        .agents
        .start(
            AgentRequest {
                title: title.trim().to_owned(),
                project,
                prompt,
                server,
                session,
            },
            cfg,
        )
        .await
}

#[tauri::command]
pub fn agent_list(state: AppStateRef<'_>) -> Vec<AgentSummary> {
    state.agents.list()
}

#[tauri::command]
pub fn agent_get(state: AppStateRef<'_>, id: String) -> Result<AgentTask, CoreError> {
    state.agents.get(&id)
}

#[tauri::command]
pub async fn agent_follow_up(
    state: AppStateRef<'_>,
    id: String,
    text: String,
) -> Result<(), CoreError> {
    if text.trim().is_empty() {
        return Err(CoreError::invalid("Пустое сообщение"));
    }
    let cfg = agent_config(&state).await?;
    state.agents.follow_up(&id, text, cfg)
}

#[tauri::command]
pub fn agent_stop(state: AppStateRef<'_>, id: String) {
    state.agents.stop(&id);
}

#[tauri::command]
pub fn agent_delete(state: AppStateRef<'_>, id: String) -> Result<(), CoreError> {
    state.agents.delete(&id)
}

#[tauri::command]
pub async fn agent_diff(state: AppStateRef<'_>, id: String) -> Result<String, CoreError> {
    state.agents.diff(&id).await
}

#[tauri::command]
pub async fn agent_remove_worktree(state: AppStateRef<'_>, id: String) -> Result<(), CoreError> {
    state.agents.remove_worktree(&id).await
}

/// Opens the agent's working folder in Explorer.
#[tauri::command]
pub fn agent_open_folder(state: AppStateRef<'_>, id: String) -> Result<(), CoreError> {
    let task = state.agents.get(&id)?;
    std::process::Command::new("explorer")
        .arg(&task.cwd)
        .spawn()
        .map(|_| ())
        .map_err(|e| {
            CoreError::new(
                ErrorKind::NotFound,
                format!("Не удалось открыть папку: {e}"),
            )
        })
}

/// Checks the Claude Code CLI; returns its version.
#[tauri::command]
pub async fn claude_cli_check(state: AppStateRef<'_>) -> Result<String, CoreError> {
    let cli = state
        .settings()
        .claude
        .cli_path
        .unwrap_or_else(|| "claude".into());
    agents::cli_version(&cli).await
}
