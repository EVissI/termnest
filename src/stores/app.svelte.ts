import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow';
import {
  api,
  on,
  type HostKeyInfo,
  type HostsFile,
  type KeyInfo,
  type Layout,
  type PaneId,
  type PendingPrompt,
  type SessionId,
  type SessionStatus,
  type Settings,
  type Snippet,
  type TransferStatus,
  type AgentEvent,
  type AgentSummary,
  type WindowState,
  type Host,
} from '../lib/ipc';
import { applyUiTheme, themeById } from '../lib/themes';
import type { TerminalView } from '../lib/terminal/TerminalView';

/** Label of the window this webview belongs to. */
export const windowLabel = getCurrentWebviewWindow().label;

export const store = $state({
  layout: { windows: [] } as Layout,
  settings: null as Settings | null,
  hosts: { groups: [], hosts: [] } as HostsFile,
  snippets: [] as Snippet[],
  sessions: {} as Record<SessionId, SessionStatus>,
  transfers: {} as Record<number, TransferStatus>,
  keys: [] as KeyInfo[],
  prompts: [] as PendingPrompt[],
  agents: {} as Record<string, AgentSummary>,
});

export interface ConfirmRequest {
  title: string;
  message: string;
  detail?: string;
  confirmLabel?: string;
  danger?: boolean;
  resolve(ok: boolean): void;
}

export interface TextPromptRequest {
  title: string;
  label: string;
  value: string;
  resolve(value: string | null): void;
}

export interface Toast {
  id: number;
  kind: 'info' | 'error' | 'ok';
  text: string;
}

export interface DeployRequest {
  hostId?: string;
  keyPath?: string;
}

export const ui = $state({
  sidebarSection: 'hosts' as 'hosts' | 'snippets' | 'keys' | 'agents',
  sftpOpen: false,
  broadcast: false,
  /** Panels selected for broadcast and snippet sending. */
  selected: [] as PaneId[],
  restorePending: false,
  hostDialog: null as Host | null,
  /** Server information window: a saved server or the server of a session. */
  serverInfo: null as { host: string | null; session: number | null; title: string } | null,
  settingsOpen: false,
  keyGenOpen: false,
  deploy: null as DeployRequest | null,
  snippetEdit: null as Snippet | null,
  palette: false,
  /** Snippet to start sending right away when the palette opens. */
  paletteSnippet: null as string | null,
  sendTarget: 'current' as 'current' | 'selected' | 'tab' | 'all',
  confirm: null as ConfirmRequest | null,
  textPrompt: null as TextPromptRequest | null,
  closeRequest: null as { sessions: number; other_windows: boolean } | null,
  hostKeyChanged: null as { info: HostKeyInfo; session: SessionId } | null,
  /** Panel whose error is being sent to Claude. */
  sendToAgent: null as PaneId | null,
  /** Agent task shown in the details dialog. */
  agentOpen: null as string | null,
  searchPane: null as PaneId | null,
  toasts: [] as Toast[],
});

/** Live terminals of this window by panel id (not reactive on purpose: nothing renders from it). */
// eslint-disable-next-line svelte/prefer-svelte-reactivity
export const terminals = new Map<PaneId, TerminalView>();
/** Working directory reported by each session's shell (not reactive on purpose). */
// eslint-disable-next-line svelte/prefer-svelte-reactivity
export const sessionCwd = new Map<SessionId, string>();

/** Live agent events for an open details dialog. */
type AgentListener = (id: string, event: AgentEvent) => void;
// eslint-disable-next-line svelte/prefer-svelte-reactivity
const agentListeners = new Set<AgentListener>();
export function onAgentEvent(fn: AgentListener): () => void {
  agentListeners.add(fn);
  return () => agentListeners.delete(fn);
}

export function myWindow(): WindowState | undefined {
  return store.layout.windows.find((w) => w.label === windowLabel);
}

let toastId = 0;
export function toast(text: string, kind: Toast['kind'] = 'info', timeout = 5000): void {
  const id = ++toastId;
  ui.toasts.push({ id, kind, text });
  setTimeout(() => {
    const i = ui.toasts.findIndex((t) => t.id === id);
    if (i >= 0) ui.toasts.splice(i, 1);
  }, timeout);
}

export function confirmDialog(req: Omit<ConfirmRequest, 'resolve'>): Promise<boolean> {
  return new Promise((resolve) => {
    ui.confirm = {
      ...req,
      resolve: (ok) => {
        ui.confirm = null;
        resolve(ok);
      },
    };
  });
}

export function askText(title: string, label: string, value = ''): Promise<string | null> {
  return new Promise((resolve) => {
    ui.textPrompt = {
      title,
      label,
      value,
      resolve: (v) => {
        ui.textPrompt = null;
        resolve(v);
      },
    };
  });
}

function applySettings(s: Settings): void {
  store.settings = s;
  applyUiTheme(themeById(s.theme));
  for (const t of terminals.values()) t.applySettings(s);
}

export async function refreshKeys(): Promise<void> {
  store.keys = await api.keys();
}

/** Loads everything this window needs and subscribes to backend events. */
export async function initStore(): Promise<void> {
  await Promise.all([
    on<Layout>('layout://changed', (l) => (store.layout = l)),
    on<Settings>('settings://changed', applySettings),
    on<HostsFile>('hosts://changed', (h) => (store.hosts = h)),
    on<Snippet[]>('snippets://changed', (s) => (store.snippets = s)),
    on<SessionStatus>('session://state', (s) => (store.sessions[s.id] = s)),
    on<TransferStatus>('sftp://transfer', (t) => (store.transfers[t.id] = t)),
    on<PendingPrompt>('prompt://request', (p) => store.prompts.push(p)),
    on<number>('prompt://resolved', (id) => {
      store.prompts = store.prompts.filter((p) => p.id !== id);
    }),
    on<{ sessions: number; other_windows: boolean }>('window://close-requested', (r) => (ui.closeRequest = r)),
    on<{ task: AgentSummary; event: AgentEvent | null }>('agents://update', (u) => {
      store.agents[u.task.id] = u.task;
      if (u.event) for (const fn of agentListeners) fn(u.task.id, u.event);
    }),
    on<number>('mcp://selection-request', (id) => {
      // Answer with the selection of this window's focused panel (empty answers are ignored).
      const w = myWindow();
      const tab = w?.tabs.find((t) => t.id === w.active);
      const text = tab ? (terminals.get(tab.focused)?.term.getSelection() ?? '') : '';
      void api.mcpSelectionReply(id, text).catch(() => {});
    }),
  ]);

  const [layout, settings, hosts, snippets, statuses, transfers, prompts, keys] = await Promise.all([
    api.layout(),
    api.settings(),
    api.hosts(),
    api.snippets(),
    api.statuses(),
    api.transfers(),
    api.promptPending(),
    api.keys(),
  ]);
  for (const a of await api.agentList()) store.agents[a.id] = a;
  store.layout = layout;
  applySettings(settings);
  store.hosts = hosts;
  store.snippets = snippets;
  for (const s of statuses) store.sessions[s.id] = s;
  for (const t of transfers) store.transfers[t.id] = t;
  store.prompts = prompts;
  store.keys = keys;
}

export async function saveSettings(patch: Partial<Settings>): Promise<void> {
  if (!store.settings) return;
  applySettings(await api.settingsSet({ ...store.settings, ...patch }));
}
