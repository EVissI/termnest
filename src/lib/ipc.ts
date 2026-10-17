import { Channel, invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

// ---- Types mirrored from the Rust core -------------------------------------------------

export type SessionId = number;
export type PaneId = number;
export type TabId = number;
export type SplitId = number;

export type SessionState = 'connecting' | 'connected' | 'reconnecting' | 'disconnected' | 'error';

export interface HostKeyInfo {
  host: string;
  port: number;
  key_type: string;
  fingerprint: string;
  known_fingerprint: string | null;
}

/** Error object returned by the Rust core (`CoreError`). */
export interface CoreError {
  kind: string;
  message: string;
  host_key?: HostKeyInfo;
}

export interface SessionStatus {
  id: SessionId;
  label: string;
  state: SessionState;
  error: CoreError | null;
  attempt: number;
  retry_in_ms: number | null;
  exit_code: number | null;
}

export interface ConnectSpec {
  label: string;
  host_id?: string;
  user: string;
  host: string;
  port: number;
  key_path: string;
}

export interface Pane {
  id: PaneId;
  spec: ConnectSpec | null;
  session: SessionId | null;
  held: boolean;
  project?: string | null;
}

export type Direction = 'horizontal' | 'vertical';

export type LayoutNode =
  | ({ type: 'pane' } & Pane)
  | {
      type: 'split';
      id: SplitId;
      direction: Direction;
      ratio: number;
      first: LayoutNode;
      second: LayoutNode;
    };

export interface Tab {
  id: TabId;
  title?: string;
  color?: string;
  root: LayoutNode;
  focused: PaneId;
}

export interface WindowState {
  label: string;
  tabs: Tab[];
  active: TabId | null;
}

export interface Layout {
  windows: WindowState[];
}

export interface Host {
  id: string;
  name: string;
  host: string;
  port: number;
  user: string;
  key_path?: string | null;
  group?: string | null;
  note: string;
  project?: string | null;
}

export interface Group {
  name: string;
  parent?: string | null;
  collapsed: boolean;
  project?: string | null;
}

export interface PasswordLoginProbe {
  methods: string[];
  password_login: boolean;
}

export interface PasswordLoginReport extends PasswordLoginProbe {
  steps: string[];
}

export interface ProjectInfo {
  path: string;
  name: string;
  exists: boolean;
  is_git: boolean;
}

export interface HostsFile {
  groups: Group[];
  hosts: Host[];
}

export type CursorStyle = 'block' | 'underline' | 'bar';

export interface Settings {
  theme: string;
  font_family: string;
  font_size: number;
  line_height: number;
  cursor_style: CursorStyle;
  cursor_blink: boolean;
  scrollback: number;
  copy_on_select: boolean;
  warn_multiline_paste: boolean;
  keepalive_secs: number;
  on_window_close: 'ask' | 'close_sessions';
  restore: 'auto' | 'ask' | 'off';
  default_key_path: string | null;
  extra_keys: string[];
  hotkeys: Record<string, string>;
  sidebar_visible: boolean;
  claude: ClaudeSettings;
}

export interface ClaudeSettings {
  mcp_enabled: boolean;
  mcp_port: number;
  agent_mode: 'propose' | 'edit';
  context_lines: number;
  max_agents: number;
  cli_path: string | null;
  notice_accepted: boolean;
}

export interface McpStatus {
  enabled: boolean;
  running: boolean;
  port: number;
  token: string;
  add_command: string;
  error: string | null;
}

export interface McpLogEntry {
  time: number;
  tool: string;
  session: number | null;
  ok: boolean;
}

export interface KeyInfo {
  path: string;
  name: string;
  algorithm: string;
  fingerprint: string;
  comment: string;
  encrypted: boolean;
  manual: boolean;
}

export interface InstallReport {
  deploy_id: number;
  added: boolean;
  verified: boolean;
  verify_error: string | null;
  can_disable_password: boolean;
  privilege_note: string;
}

export interface DisableReport {
  steps: string[];
  effective: string;
}

export interface Snippet {
  id: string;
  name: string;
  command: string;
  tags: string[];
  press_enter: boolean;
}

export type EntryKind = 'dir' | 'file' | 'symlink' | 'other';

export interface RemoteEntry {
  name: string;
  path: string;
  kind: EntryKind;
  size: number;
  modified: number | null;
  mode: number | null;
  owner: string | null;
  group: string | null;
}

export type TransferState = 'queued' | 'running' | 'done' | 'failed' | 'cancelled';

export interface TransferStatus {
  id: number;
  session: SessionId;
  direction: 'upload' | 'download';
  name: string;
  destination: string;
  state: TransferState;
  total_bytes: number;
  done_bytes: number;
  files_total: number;
  files_done: number;
  skipped: number;
  current_file: string | null;
  speed_bps: number;
  error: string | null;
}

export type PromptRequest =
  | { kind: 'host_key_unknown'; host: string; port: number; key_type: string; fingerprint: string }
  | { kind: 'passphrase'; key_path: string; retry: boolean }
  | { kind: 'file_conflict'; name: string; remote: boolean }
  | { kind: 'agent_action'; server: string; session: number; action: 'exec' | 'input'; command: string };

export interface PendingPrompt {
  id: number;
  request: PromptRequest;
}

export type ConflictAction = 'replace' | 'skip' | 'rename';

export type PromptReply =
  | { kind: 'cancel' }
  | { kind: 'confirm' }
  | { kind: 'confirm_session' }
  | { kind: 'passphrase'; value: string; remember: boolean }
  | { kind: 'conflict'; action: ConflictAction; apply_to_all: boolean };

export type OpenSource =
  | { kind: 'host'; id: string }
  | { kind: 'target'; target: string }
  | { kind: 'pane'; pane: PaneId }
  | { kind: 'empty' };

export interface WindowInfo {
  label: string;
  number: number;
  tabs: number;
}

export interface KnownHost {
  host: string;
  port: number;
  key_type: string;
  fingerprint: string;
}

export type AgentState = 'queued' | 'running' | 'done' | 'failed' | 'stopped';
export type AgentEventKind = 'prompt' | 'text' | 'tool' | 'tool_error' | 'result' | 'error' | 'info';

export interface AgentEvent {
  time: number;
  kind: AgentEventKind;
  text: string;
}

export interface AgentSummary {
  id: string;
  title: string;
  project: string | null;
  ask: boolean;
  cwd: string;
  worktree: string | null;
  branch: string | null;
  server: string | null;
  session: SessionId | null;
  state: AgentState;
  created: number;
  updated: number;
  files: string[];
  result: string | null;
  error: string | null;
  cost_usd: number | null;
  can_continue: boolean;
}

export interface AgentTask extends Omit<AgentSummary, 'can_continue'> {
  claude_session: string | null;
  events: AgentEvent[];
}

export interface AgentPrepared {
  project: ProjectInfo | null;
  server: string;
  title: string;
  prompt: string;
  secrets: string[];
  mcp: boolean;
}

// ---- Helpers ----------------------------------------------------------------------------

export function errorMessage(e: unknown): string {
  if (typeof e === 'string') return e;
  if (e && typeof e === 'object' && 'message' in e && typeof e.message === 'string') {
    return e.message;
  }
  return String(e);
}

export function on<T>(event: string, handler: (payload: T) => void): Promise<UnlistenFn> {
  return listen<T>(event, (e) => handler(e.payload));
}

// ---- Commands ---------------------------------------------------------------------------

export const api = {
  // sessions
  subscribe(id: SessionId, onData: (data: Uint8Array) => void): Promise<number> {
    const channel = new Channel<ArrayBuffer>();
    channel.onmessage = (buffer) => onData(new Uint8Array(buffer));
    return invoke('session_subscribe', { id, onOutput: channel });
  },
  unsubscribe: (id: SessionId, subscription: number) => invoke<void>('session_unsubscribe', { id, subscription }),
  write: (id: SessionId, data: Uint8Array) => invoke<void>('session_write', { id, data: Array.from(data) }),
  writeMany: (ids: SessionId[], data: Uint8Array) => invoke<void>('session_write_many', { ids, data: Array.from(data) }),
  resize: (id: SessionId, cols: number, rows: number) => invoke<void>('session_resize', { id, cols, rows }),
  retry: (id: SessionId) => invoke<void>('session_retry', { id }),
  reconnectNow: (id: SessionId) => invoke<void>('session_reconnect_now', { id }),
  cancelReconnect: (id: SessionId) => invoke<void>('session_cancel_reconnect', { id }),
  statuses: () => invoke<SessionStatus[]>('session_statuses'),
  promptRespond: (id: number, reply: PromptReply) => invoke<void>('prompt_respond', { id, reply }),
  promptPending: () => invoke<PendingPrompt[]>('prompt_pending'),
  knownHosts: () => invoke<KnownHost[]>('known_hosts_list'),
  knownHostRemove: (host: string, port: number) => invoke<number>('known_hosts_remove', { host, port }),

  // layout
  layout: () => invoke<Layout>('layout_get'),
  tabNew: (window: string, source: OpenSource, index?: number) =>
    invoke<[TabId, PaneId]>('tab_new', { window, source, index: index ?? null }),
  tabDuplicate: (pane: PaneId) => invoke<[TabId, PaneId]>('tab_duplicate', { pane }),
  tabClose: (tab: TabId) => invoke<void>('tab_close', { tab }),
  tabActivate: (tab: TabId) => invoke<void>('tab_activate', { tab }),
  tabRename: (tab: TabId, title: string | null) => invoke<void>('tab_rename', { tab, title }),
  tabSetColor: (tab: TabId, color: string | null) => invoke<void>('tab_set_color', { tab, color }),
  tabMove: (tab: TabId, window: string, index: number | null) => invoke<void>('tab_move', { tab, window, index }),
  tabToNewWindow: (tab: TabId) => invoke<string>('tab_to_new_window', { tab, screenX: null, screenY: null }),
  tabDrop: (tab: TabId, screenX: number, screenY: number) => invoke<void>('tab_drop', { tab, screenX, screenY }),
  paneOpen: (pane: PaneId, source: OpenSource) => invoke<void>('pane_open', { pane, source }),
  paneAttach: (pane: PaneId, cols: number, rows: number) =>
    invoke<SessionId | null>('pane_attach', { pane, cols, rows }),
  paneConnect: (pane: PaneId, cols: number, rows: number) =>
    invoke<SessionId | null>('pane_connect', { pane, cols, rows }),
  paneSplit: (pane: PaneId, direction: Direction, source: OpenSource) =>
    invoke<PaneId>('pane_split', { pane, direction, source }),
  paneClose: (pane: PaneId) => invoke<void>('pane_close', { pane }),
  paneFocus: (pane: PaneId) => invoke<void>('pane_focus', { pane }),
  splitSetRatio: (tab: TabId, split: SplitId, ratio: number) => invoke<void>('split_set_ratio', { tab, split, ratio }),
  windowList: () => invoke<WindowInfo[]>('window_list'),
  windowNew: () => invoke<string>('window_new'),
  windowClose: (label: string, action: 'close_sessions' | 'move_tabs') =>
    invoke<void>('window_close', { label, action }),
  startupInfo: () => invoke<{ warnings: string[]; restore_pending: boolean }>('startup_info'),
  restoreConnectAll: () => invoke<void>('restore_connect_all'),

  // hosts
  hosts: () => invoke<HostsFile>('hosts_get'),
  hostUpsert: (host: Host) => invoke<Host>('hosts_upsert', { host }),
  hostDuplicate: (id: string) => invoke<Host>('hosts_duplicate', { id }),
  hostDelete: (id: string) => invoke<void>('hosts_delete', { id }),
  groupAdd: (name: string, parent: string | null = null) => invoke<void>('group_add', { name, parent }),
  groupMove: (name: string, parent: string | null, before: string | null) =>
    invoke<void>('group_move', { name, parent, before }),
  hostMove: (id: string, group: string | null, before: string | null) => invoke<void>('host_move', { id, group, before }),
  hostPasswordLogin: (id: string) => invoke<PasswordLoginProbe>('host_password_login', { id }),
  hostSetPasswordLogin: (id: string, allow: boolean) =>
    invoke<PasswordLoginReport>('host_set_password_login', { id, allow }),
  groupRename: (old: string, name: string) => invoke<void>('group_rename', { old, new: name }),
  groupDelete: (name: string) => invoke<void>('group_delete', { name }),
  groupSetCollapsed: (name: string, collapsed: boolean) => invoke<void>('group_set_collapsed', { name, collapsed }),
  importSshConfig: () => invoke<number>('hosts_import_ssh_config'),
  hostSetProject: (id: string, project: string | null) => invoke<void>('host_set_project', { id, project }),
  groupSetProject: (name: string, project: string | null) => invoke<void>('group_set_project', { name, project }),
  projectInfo: (path: string) => invoke<ProjectInfo>('project_info', { path }),
  paneSetProject: (pane: PaneId, project: string | null) => invoke<void>('pane_set_project', { pane, project }),

  // keys
  keys: () => invoke<KeyInfo[]>('keys_list'),
  sshDir: () => invoke<string | null>('keys_ssh_dir'),
  keyAdd: (path: string) => invoke<KeyInfo>('keys_add', { path }),
  keyForget: (path: string) => invoke<void>('keys_forget', { path }),
  keyPublic: (path: string) => invoke<string>('keys_public', { path }),
  keyGenerate: (name: string, kind: 'ed25519' | 'rsa4096', comment: string, passphrase: string | null, overwrite: boolean) =>
    invoke<KeyInfo>('keys_generate', { name, kind, comment, passphrase, overwrite }),
  deployInstall: (target: string, password: string, keyPath: string) =>
    invoke<InstallReport>('deploy_install', { target, password, keyPath }),
  deployDisablePassword: (deployId: number) => invoke<DisableReport>('deploy_disable_password', { deployId }),
  deployFinish: (deployId: number) => invoke<void>('deploy_finish', { deployId }),

  // settings
  settings: () => invoke<Settings>('settings_get'),
  settingsSet: (settings: Settings) => invoke<Settings>('settings_set', { settings }),
  appInfo: () => invoke<{ version: string; data_dir: string }>('app_info'),
  passphrasesClear: () => invoke<void>('passphrases_clear'),
  mcpStatus: () => invoke<McpStatus>('mcp_status'),
  mcpRegenerateToken: () => invoke<McpStatus>('mcp_regenerate_token'),
  mcpJournal: () => invoke<McpLogEntry[]>('mcp_journal'),
  mcpSelectionReply: (id: number, text: string) => invoke<void>('mcp_selection_reply', { id, text }),

  // agents
  agentPrepare: (pane: PaneId, selection: string | null, cwd: string | null, comment: string, ask: boolean) =>
    invoke<AgentPrepared>('agent_prepare', { pane, selection, cwd, comment, ask }),
  agentStart: (pane: PaneId, project: string | null, ask: boolean, title: string, prompt: string) =>
    invoke<string>('agent_start', { pane, project, ask, title, prompt }),
  agentList: () => invoke<AgentSummary[]>('agent_list'),
  agentGet: (id: string) => invoke<AgentTask>('agent_get', { id }),
  agentFollowUp: (id: string, text: string) => invoke<void>('agent_follow_up', { id, text }),
  agentStop: (id: string) => invoke<void>('agent_stop', { id }),
  agentDelete: (id: string) => invoke<void>('agent_delete', { id }),
  agentDiff: (id: string) => invoke<string>('agent_diff', { id }),
  agentRemoveWorktree: (id: string) => invoke<void>('agent_remove_worktree', { id }),
  agentOpenFolder: (id: string) => invoke<void>('agent_open_folder', { id }),
  claudeCliCheck: () => invoke<string>('claude_cli_check'),

  // snippets
  snippets: () => invoke<Snippet[]>('snippets_list'),
  snippetUpsert: (snippet: Snippet) => invoke<Snippet>('snippets_upsert', { snippet }),
  snippetDelete: (id: string) => invoke<void>('snippets_delete', { id }),

  // sftp
  sftpHome: (session: SessionId) => invoke<string>('sftp_home', { session }),
  sftpRealpath: (session: SessionId, path: string) => invoke<string>('sftp_realpath', { session, path }),
  sftpList: (session: SessionId, path: string) => invoke<RemoteEntry[]>('sftp_list', { session, path }),
  sftpMkdir: (session: SessionId, path: string) => invoke<void>('sftp_mkdir', { session, path }),
  sftpRename: (session: SessionId, from: string, to: string) => invoke<void>('sftp_rename', { session, from, to }),
  sftpRemove: (session: SessionId, paths: string[]) => invoke<void>('sftp_remove', { session, paths }),
  sftpChmod: (session: SessionId, path: string, mode: number) => invoke<void>('sftp_chmod', { session, path, mode }),
  sftpUpload: (session: SessionId, localPaths: string[], remoteDir: string) =>
    invoke<number[]>('sftp_upload', { session, localPaths, remoteDir }),
  sftpDownload: (session: SessionId, items: { path: string; is_dir: boolean }[], localDir: string) =>
    invoke<number[]>('sftp_download', { session, items, localDir }),
  transfers: () => invoke<TransferStatus[]>('transfers_list'),
  transferCancel: (id: number) => invoke<void>('transfer_cancel', { id }),
  transfersClear: () => invoke<void>('transfers_clear'),
  sessionCwd: (session: SessionId) => invoke<string | null>('session_cwd', { session }),
};
