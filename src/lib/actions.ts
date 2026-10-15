import { api, errorMessage, type Direction, type LayoutNode, type OpenSource, type Pane, type PaneId, type SessionId, type Tab } from './ipc';
import { allPanes, neighbor, type Side } from './layout/geometry';
import { myWindow, saveSettings, store, terminals, toast, ui, windowLabel } from '../stores/app.svelte';
import type { HotkeyAction } from './hotkeys';

export function activeTab(): Tab | undefined {
  const w = myWindow();
  if (!w) return undefined;
  return w.tabs.find((t) => t.id === w.active) ?? w.tabs[0];
}

export function focusedPane(): Pane | undefined {
  const tab = activeTab();
  if (!tab) return undefined;
  const panes = allPanes(tab.root);
  return panes.find((p) => p.id === tab.focused) ?? panes[0];
}

export function findPane(root: LayoutNode, id: PaneId): Pane | undefined {
  return allPanes(root).find((p) => p.id === id);
}

/** Runs a backend call and shows its error as a toast. */
export async function attempt<T>(fn: () => Promise<T>): Promise<T | undefined> {
  try {
    return await fn();
  } catch (e) {
    toast(errorMessage(e), 'error');
    return undefined;
  }
}

export function focusPane(id: PaneId): void {
  void attempt(() => api.paneFocus(id));
  terminals.get(id)?.focus();
}

/** Opens a saved host: in the focused panel if it is empty, otherwise in a new tab. */
export async function openSource(source: OpenSource, forceNewTab = false): Promise<void> {
  const pane = focusedPane();
  if (!forceNewTab && pane && !pane.spec) {
    await attempt(() => api.paneOpen(pane.id, source));
    return;
  }
  await attempt(() => api.tabNew(windowLabel, source));
}

export async function newTab(): Promise<void> {
  await attempt(() => api.tabNew(windowLabel, { kind: 'empty' }));
}

export async function split(direction: Direction): Promise<void> {
  const pane = focusedPane();
  if (!pane) return;
  const source: OpenSource = pane.spec ? { kind: 'pane', pane: pane.id } : { kind: 'empty' };
  await attempt(() => api.paneSplit(pane.id, direction, source));
}

export function moveFocus(side: Side): void {
  const tab = activeTab();
  if (!tab) return;
  const next = neighbor(tab.root, tab.focused, side);
  if (next !== null) focusPane(next);
}

export function cycleTab(delta: number): void {
  const w = myWindow();
  if (!w || w.tabs.length === 0) return;
  const i = Math.max(0, w.tabs.findIndex((t) => t.id === w.active));
  const next = w.tabs[(i + delta + w.tabs.length) % w.tabs.length];
  void attempt(() => api.tabActivate(next.id));
}

/** Sessions of the panels a snippet or broadcast input goes to. */
export type SendTarget = 'current' | 'selected' | 'tab' | 'all';

export function targetSessions(target: SendTarget): SessionId[] {
  const tab = activeTab();
  const panes = tab ? allPanes(tab.root) : [];
  const live = (ps: Pane[]) => ps.map((p) => p.session).filter((s): s is SessionId => s !== null);
  switch (target) {
    case 'current': {
      const p = focusedPane();
      return p?.session != null ? [p.session] : [];
    }
    case 'selected':
      return live(panes.filter((p) => ui.selected.includes(p.id)));
    case 'tab':
      return live(panes);
    case 'all':
      return Object.values(store.sessions)
        .filter((s) => s.state === 'connected')
        .map((s) => s.id);
  }
}

/** Broadcast targets: the selected panels of the active tab (all of them if none selected). */
export function broadcastSessions(): SessionId[] {
  const selected = targetSessions('selected');
  return selected.length > 0 ? selected : targetSessions('tab');
}

export function togglePaneSelection(id: PaneId): void {
  const i = ui.selected.indexOf(id);
  if (i >= 0) ui.selected.splice(i, 1);
  else ui.selected.push(id);
}

const encoder = new TextEncoder();

export async function sendText(sessions: SessionId[], text: string): Promise<void> {
  if (sessions.length === 0) {
    toast('Нет подключённых панелей для отправки', 'error');
    return;
  }
  await attempt(() => api.writeMany(sessions, encoder.encode(text)));
}

export async function runAction(action: HotkeyAction): Promise<void> {
  const pane = focusedPane();
  switch (action) {
    case 'tab.new':
      return newTab();
    case 'pane.close':
      if (pane) await attempt(() => api.paneClose(pane.id));
      return;
    case 'window.new':
      await attempt(() => api.windowNew());
      return;
    case 'split.right':
      return split('horizontal');
    case 'split.down':
      return split('vertical');
    case 'focus.left':
      return moveFocus('left');
    case 'focus.right':
      return moveFocus('right');
    case 'focus.up':
      return moveFocus('up');
    case 'focus.down':
      return moveFocus('down');
    case 'tab.next':
      return cycleTab(1);
    case 'tab.prev':
      return cycleTab(-1);
    case 'session.duplicate':
      if (pane?.spec) await attempt(() => api.tabDuplicate(pane.id));
      return;
    case 'snippets.palette':
      ui.palette = !ui.palette;
      return;
    case 'terminal.search':
      if (pane) ui.searchPane = ui.searchPane === pane.id ? null : pane.id;
      return;
    case 'terminal.copy':
      if (pane) await terminals.get(pane.id)?.copySelection();
      return;
    case 'terminal.paste':
      if (pane) await terminals.get(pane.id)?.paste();
      return;
    case 'broadcast.toggle':
      ui.broadcast = !ui.broadcast;
      return;
    case 'sftp.toggle':
      ui.sftpOpen = !ui.sftpOpen;
      return;
    case 'agent.send':
      if (pane?.spec) ui.sendToAgent = pane.id;
      return;
    case 'sidebar.toggle':
      if (store.settings) await saveSettings({ sidebar_visible: !store.settings.sidebar_visible });
      return;
  }
}
