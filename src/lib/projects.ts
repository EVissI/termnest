import { open } from '@tauri-apps/plugin-dialog';
import { api, type Pane } from './ipc';
import { attempt } from './actions';
import { store, toast } from '../stores/app.svelte';

export type ProjectSource = 'pane' | 'host' | 'group';

export interface EffectiveProject {
  path: string;
  name: string;
  source: ProjectSource;
}

function folderName(path: string): string {
  const parts = path.split(/[\\/]/).filter(Boolean);
  return parts[parts.length - 1] ?? path;
}

/** Project of a panel: its own override, then the server's, then the group's (SPEC 12.2). */
export function effectiveProject(pane: Pane): EffectiveProject | null {
  if (pane.project) return { path: pane.project, name: folderName(pane.project), source: 'pane' };
  const hostId = pane.spec?.host_id;
  if (!hostId) return null;
  const host = store.hosts.hosts.find((h) => h.id === hostId);
  if (!host) return null;
  if (host.project) return { path: host.project, name: folderName(host.project), source: 'host' };
  const group = store.hosts.groups.find((g) => g.name === host.group);
  if (group?.project) return { path: group.project, name: folderName(group.project), source: 'group' };
  return null;
}

/** Asks for a project folder; warns if it is missing or not a git repository. */
export async function pickProject(title: string, current?: string | null): Promise<string | null> {
  const picked = await open({ title, directory: true, multiple: false, defaultPath: current ?? undefined });
  if (typeof picked !== 'string') return null;
  const info = await api.projectInfo(picked).catch(() => null);
  if (info && !info.exists) toast(`Папка не найдена: ${picked}`, 'error');
  else if (info && !info.is_git) toast(`«${info.name}» не git-репозиторий: агенты смогут работать только прямо в папке`, 'info', 7000);
  return picked;
}

export async function linkPane(pane: Pane): Promise<void> {
  const path = await pickProject('Проект для этой панели', effectiveProject(pane)?.path);
  if (path) await attempt(() => api.paneSetProject(pane.id, path));
}

export async function linkHost(hostId: string): Promise<void> {
  const host = store.hosts.hosts.find((h) => h.id === hostId);
  const path = await pickProject(`Проект для сервера ${host?.name ?? ''}`, host?.project);
  if (path) await attempt(() => api.hostSetProject(hostId, path));
}

export async function linkGroup(name: string): Promise<void> {
  const group = store.hosts.groups.find((g) => g.name === name);
  const path = await pickProject(`Проект по умолчанию для группы ${name}`, group?.project);
  if (path) await attempt(() => api.groupSetProject(name, path));
}
