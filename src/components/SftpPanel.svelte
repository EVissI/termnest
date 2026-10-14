<script lang="ts">
  import { onMount } from 'svelte';
  import { getCurrentWebview } from '@tauri-apps/api/webview';
  import { open } from '@tauri-apps/plugin-dialog';
  import { api, errorMessage, type RemoteEntry, type SessionId, type TransferStatus } from '../lib/ipc';
  import { attempt, focusedPane } from '../lib/actions';
  import { formatBytes, formatDate, formatMode, joinRemote, parentRemote } from '../lib/format';
  import { askText, confirmDialog, sessionCwd, store, toast, ui } from '../stores/app.svelte';
  import ContextMenu, { type MenuItem } from './ContextMenu.svelte';
  import ChmodDialog from './ChmodDialog.svelte';
  import Icon from './Icon.svelte';

  const session = $derived<SessionId | null>(focusedPane()?.session ?? null);
  const connected = $derived(session !== null && store.sessions[session]?.state === 'connected');

  let panel: HTMLDivElement;
  let cwd = $state('');
  let home = $state('');
  let entries = $state<RemoteEntry[]>([]);
  let selected = $state<string[]>([]);
  let anchor = $state<string | null>(null);
  let loading = $state(false);
  let error = $state('');
  let dropActive = $state(false);
  let editingPath = $state(false);
  let pathInput = $state('');
  let menu = $state<{ x: number; y: number; items: MenuItem[] } | null>(null);
  let chmod = $state<RemoteEntry | null>(null);
  let loadedFor = $state<SessionId | null>(null);

  const transfers = $derived(
    Object.values(store.transfers)
      .filter((t) => t.session === session)
      .sort((a, b) => b.id - a.id),
  );

  async function load(path: string): Promise<void> {
    if (session === null) return;
    const id = session;
    loading = true;
    error = '';
    try {
      const real = await api.sftpRealpath(id, path);
      const list = await api.sftpList(id, real);
      if (session !== id) return;
      cwd = real;
      entries = list;
      selected = [];
      anchor = null;
    } catch (e) {
      error = errorMessage(e);
    } finally {
      loading = false;
    }
  }

  // Open the home folder whenever the panel switches to another session.
  $effect(() => {
    const id = session;
    if (id === null || !connected || loadedFor === id) return;
    loadedFor = id;
    entries = [];
    cwd = '';
    api
      .sftpHome(id)
      .then((h) => {
        home = h;
        return load(h);
      })
      .catch((e) => (error = errorMessage(e)));
  });

  // Refresh when an upload into the current folder finishes.
  let lastDone = 0;
  $effect(() => {
    const done = transfers.filter((t) => t.direction === 'upload' && t.destination === cwd && t.state === 'done').length;
    if (done > lastDone && cwd) void load(cwd);
    lastDone = done;
  });

  onMount(() => {
    let unlisten: (() => void) | null = null;
    let disposed = false;
    getCurrentWebview()
      .onDragDropEvent((event) => {
        const p = event.payload;
        if (p.type === 'leave') {
          dropActive = false;
          return;
        }
        const dpr = window.devicePixelRatio || 1;
        const r = panel?.getBoundingClientRect();
        const x = p.position.x / dpr;
        const y = p.position.y / dpr;
        const over = !!r && x >= r.left && x <= r.right && y >= r.top && y <= r.bottom;
        dropActive = over && p.type !== 'drop';
        if (p.type === 'drop' && over) void upload(p.paths);
      })
      .then((fn) => {
        if (disposed) fn();
        else unlisten = fn;
      });
    return () => {
      disposed = true;
      unlisten?.();
    };
  });

  async function upload(paths: string[]): Promise<void> {
    if (session === null || !cwd || paths.length === 0) return;
    const id = session;
    await attempt(() => api.sftpUpload(id, paths, cwd));
  }

  async function uploadFiles(directory: boolean): Promise<void> {
    const picked = await open({ title: directory ? 'Загрузить папку' : 'Загрузить файлы', multiple: true, directory });
    if (!picked) return;
    await upload(Array.isArray(picked) ? picked : [picked]);
  }

  async function download(items: RemoteEntry[]): Promise<void> {
    if (session === null || items.length === 0) return;
    const id = session;
    const dir = await open({ title: 'Куда скачать', directory: true, multiple: false });
    if (typeof dir !== 'string') return;
    await attempt(() =>
      api.sftpDownload(
        id,
        items.map((e) => ({ path: e.path, is_dir: e.kind === 'dir' })),
        dir,
      ),
    );
  }

  async function mkdir(): Promise<void> {
    if (session === null) return;
    const id = session;
    const name = await askText('Новая папка', 'Имя папки');
    if (!name?.trim()) return;
    if ((await attempt(() => api.sftpMkdir(id, joinRemote(cwd, name.trim())))) !== undefined) await load(cwd);
  }

  async function rename(entry: RemoteEntry): Promise<void> {
    if (session === null) return;
    const id = session;
    const name = await askText('Переименовать', 'Новое имя', entry.name);
    if (!name?.trim() || name.trim() === entry.name) return;
    if ((await attempt(() => api.sftpRename(id, entry.path, joinRemote(cwd, name.trim())))) !== undefined) await load(cwd);
  }

  async function remove(items: RemoteEntry[]): Promise<void> {
    if (session === null || items.length === 0) return;
    const id = session;
    const dirs = items.filter((e) => e.kind === 'dir').length;
    const ok = await confirmDialog({
      title: 'Удаление',
      message:
        items.length === 1
          ? `Удалить «${items[0].name}»${dirs ? ' вместе со всем содержимым' : ''}?`
          : `Удалить ${items.length} объектов${dirs ? ' (папки - вместе с содержимым)' : ''}?`,
      detail: items.map((e) => e.path).join('\n'),
      confirmLabel: 'Удалить',
      danger: true,
    });
    if (!ok) return;
    if ((await attempt(() => api.sftpRemove(id, items.map((e) => e.path)))) !== undefined) await load(cwd);
  }

  async function applyChmod(entry: RemoteEntry, mode: number): Promise<void> {
    chmod = null;
    if (session === null) return;
    const id = session;
    if ((await attempt(() => api.sftpChmod(id, entry.path, mode))) !== undefined) await load(cwd);
  }

  async function goToTerminalDir(): Promise<void> {
    if (session === null) return;
    let path = sessionCwd.get(session) ?? null;
    if (!path) path = (await api.sessionCwd(session).catch(() => null)) ?? null;
    if (!path) {
      toast('Терминал не сообщает текущую папку (нужен OSC 7 или заголовок вида user@host: путь)', 'info');
      return;
    }
    if (path.startsWith('~')) path = home + path.slice(1);
    await load(path);
  }

  function click(e: MouseEvent, entry: RemoteEntry): void {
    if (e.shiftKey && anchor) {
      const a = entries.findIndex((x) => x.path === anchor);
      const b = entries.findIndex((x) => x.path === entry.path);
      const [from, to] = a < b ? [a, b] : [b, a];
      selected = entries.slice(from, to + 1).map((x) => x.path);
    } else if (e.ctrlKey) {
      selected = selected.includes(entry.path) ? selected.filter((p) => p !== entry.path) : [...selected, entry.path];
      anchor = entry.path;
    } else {
      selected = [entry.path];
      anchor = entry.path;
    }
  }

  function selectedEntries(): RemoteEntry[] {
    return entries.filter((e) => selected.includes(e.path));
  }

  function openMenu(e: MouseEvent, entry: RemoteEntry): void {
    e.preventDefault();
    if (!selected.includes(entry.path)) {
      selected = [entry.path];
      anchor = entry.path;
    }
    const items = selectedEntries();
    menu = {
      x: e.clientX,
      y: e.clientY,
      items: [
        { label: 'Открыть', disabled: entry.kind !== 'dir' || items.length > 1, action: () => load(entry.path) },
        { label: 'Скачать...', action: () => download(items) },
        { label: 'Переименовать', disabled: items.length > 1, action: () => rename(entry) },
        { label: 'Права...', disabled: items.length > 1, action: () => (chmod = entry) },
        { separator: true, label: '' },
        { label: 'Удалить', danger: true, action: () => remove(items) },
      ],
    };
  }

  function onListKey(e: KeyboardEvent): void {
    if (e.key === 'Delete') void remove(selectedEntries());
    else if (e.key === 'F2' && selected.length === 1) void rename(selectedEntries()[0]);
    else if (e.key === 'Backspace') void load(parentRemote(cwd));
    else if (e.key === 'Enter' && selected.length === 1) {
      const entry = selectedEntries()[0];
      if (entry.kind === 'dir') void load(entry.path);
    }
  }

  const crumbs = $derived.by(() => {
    const parts = cwd.split('/').filter(Boolean);
    const out = [{ name: '/', path: '/' }];
    let acc = '';
    for (const p of parts) {
      acc += `/${p}`;
      out.push({ name: p, path: acc });
    }
    return out;
  });

  function submitPath(e: SubmitEvent): void {
    e.preventDefault();
    editingPath = false;
    if (pathInput.trim()) void load(pathInput.trim());
  }

  function progress(t: TransferStatus): number {
    return t.total_bytes > 0 ? Math.min(100, (t.done_bytes / t.total_bytes) * 100) : t.state === 'done' ? 100 : 0;
  }

  const stateLabels: Record<string, string> = {
    queued: 'в очереди',
    running: '',
    done: 'готово',
    failed: 'ошибка',
    cancelled: 'отменено',
  };
</script>

<div class="sftp" class:drop={dropActive} bind:this={panel}>
  <div class="head">
    <h3>SFTP</h3>
    <button class="icon" type="button" title="Закрыть панель" onclick={() => (ui.sftpOpen = false)}><Icon name="x" /></button>
  </div>

  {#if session === null}
    <p class="hint pad">Выберите панель с подключённым сервером.</p>
  {:else if !connected}
    <p class="hint pad">Сессия не подключена.</p>
  {:else}
    <div class="toolbar">
      <button class="icon" type="button" title="Вверх (Backspace)" onclick={() => load(parentRemote(cwd))} disabled={cwd === '/'}><Icon name="up" size={15} /></button>
      <button class="icon" type="button" title="Домашняя папка" onclick={() => load(home)}><Icon name="home" size={15} /></button>
      <button class="icon" type="button" title="Папка терминала" onclick={goToTerminalDir}><Icon name="terminal" size={15} /></button>
      <button class="icon" type="button" title="Обновить" onclick={() => load(cwd)}><Icon name="refresh" size={15} /></button>
      <span class="sep"></span>
      <button class="icon" type="button" title="Новая папка" onclick={mkdir}><Icon name="folder-plus" size={15} /></button>
      <button class="icon" type="button" title="Загрузить файлы" onclick={() => uploadFiles(false)}><Icon name="upload" size={15} /></button>
      <button class="icon" type="button" title="Загрузить папку" onclick={() => uploadFiles(true)}><Icon name="folder" size={15} /></button>
      <button class="icon" type="button" title="Скачать выбранное" disabled={selected.length === 0} onclick={() => download(selectedEntries())}>
        <Icon name="download" size={15} />
      </button>
      <button class="icon" type="button" title="Удалить выбранное (Delete)" disabled={selected.length === 0} onclick={() => remove(selectedEntries())}>
        <Icon name="trash" size={15} />
      </button>
    </div>

    {#if editingPath}
      <form class="path-edit" onsubmit={submitPath}>
        <!-- svelte-ignore a11y_autofocus -->
        <input bind:value={pathInput} autofocus onblur={() => (editingPath = false)} class="mono" aria-label="Путь" />
      </form>
    {:else}
      <div class="crumbs" role="navigation" aria-label="Путь" ondblclick={() => ((pathInput = cwd), (editingPath = true))}>
        {#each crumbs as c, i (c.path)}
          {#if i > 1}<span class="slash">/</span>{/if}
          <button class="crumb" type="button" onclick={() => load(c.path)}>{c.name}</button>
        {/each}
      </div>
    {/if}

    {#if error}<div class="err-box small">{error}</div>{/if}

    <div class="list" role="listbox" tabindex="0" aria-multiselectable="true" onkeydown={onListKey} class:loading>
      {#each entries as entry (entry.path)}
        <div
          class="entry"
          class:selected={selected.includes(entry.path)}
          role="option"
          tabindex="-1"
          aria-selected={selected.includes(entry.path)}
          onclick={(e) => click(e, entry)}
          ondblclick={() => entry.kind === 'dir' && load(entry.path)}
          oncontextmenu={(e) => openMenu(e, entry)}
          onkeydown={() => {}}
          title="{entry.name}{entry.owner ? `\n${entry.owner}:${entry.group}` : ''}"
        >
          <Icon name={entry.kind === 'dir' ? 'folder' : entry.kind === 'symlink' ? 'link' : 'file'} size={14} />
          <span class="name">{entry.name}</span>
          <span class="size">{entry.kind === 'dir' ? '' : formatBytes(entry.size)}</span>
          <span class="mode mono">{formatMode(entry.mode)}</span>
          <span class="date">{formatDate(entry.modified)}</span>
        </div>
      {:else}
        {#if !loading && !error}<p class="hint pad">Папка пуста</p>{/if}
      {/each}
    </div>
    <div class="drop-hint hint">Перетащите файлы или папки из Проводника сюда</div>
  {/if}

  {#if transfers.length}
    <div class="transfers">
      <div class="thead">
        <span>Передачи</span>
        <button class="link" type="button" onclick={() => api.transfersClear()}>Очистить завершённые</button>
      </div>
      {#each transfers as t (t.id)}
        <div class="transfer" class:failed={t.state === 'failed'}>
          <div class="trow">
            <Icon name={t.direction === 'upload' ? 'upload' : 'download'} size={12} />
            <span class="tname" title={t.current_file ?? t.name}>{t.name}</span>
            <span class="tinfo">
              {#if t.state === 'running'}
                {formatBytes(t.done_bytes)} / {formatBytes(t.total_bytes)}{t.speed_bps ? `, ${formatBytes(t.speed_bps)}/с` : ''}
              {:else}
                {stateLabels[t.state]}{t.skipped ? `, пропущено: ${t.skipped}` : ''}
              {/if}
            </span>
            {#if t.state === 'running' || t.state === 'queued'}
              <button class="icon" type="button" title="Отменить" onclick={() => api.transferCancel(t.id)}><Icon name="x" size={12} /></button>
            {/if}
          </div>
          <div class="bar"><div class="fill {t.state}" style:width="{progress(t)}%"></div></div>
          {#if t.error}<div class="terr">{t.error}</div>{/if}
          {#if t.files_total > 1}<div class="hint">Файлов: {t.files_done} / {t.files_total}</div>{/if}
        </div>
      {/each}
    </div>
  {/if}
</div>

{#if menu}
  <ContextMenu x={menu.x} y={menu.y} items={menu.items} onclose={() => (menu = null)} />
{/if}
{#if chmod}
  {@const entry = chmod}
  <ChmodDialog name={entry.path} mode={entry.mode ?? 0o644} onapply={(m) => applyChmod(entry, m)} onclose={() => (chmod = null)} />
{/if}

<style>
  .sftp {
    display: flex;
    flex-direction: column;
    width: 380px;
    min-width: 280px;
    background: var(--bg-panel);
    border-left: 1px solid var(--border);
    overflow: hidden;
  }
  .sftp.drop {
    outline: 2px dashed var(--accent);
    outline-offset: -4px;
  }
  .head {
    display: flex;
    align-items: center;
    padding: 8px 6px 4px 12px;
  }
  h3 {
    flex: 1;
    margin: 0;
    font-size: 12px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-dim);
  }
  .pad {
    padding: 8px 12px;
  }
  .toolbar {
    display: flex;
    align-items: center;
    gap: 1px;
    padding: 0 6px;
  }
  .sep {
    width: 1px;
    height: 16px;
    margin: 0 4px;
    background: var(--border);
  }
  .crumbs {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    padding: 4px 10px;
    font-size: 12px;
    border-bottom: 1px solid var(--border);
  }
  .crumb {
    border: none;
    background: none;
    padding: 1px 2px;
    border-radius: 3px;
    cursor: pointer;
  }
  .crumb:hover {
    background: var(--bg-hover);
  }
  .slash {
    color: var(--text-dim);
  }
  .path-edit {
    padding: 4px 8px;
  }
  .path-edit input {
    width: 100%;
  }
  .small {
    margin: 6px 8px;
    font-size: 12px;
  }
  .list {
    flex: 1;
    overflow: auto;
    outline: none;
    padding: 2px 0;
  }
  .list.loading {
    opacity: 0.6;
  }
  .entry {
    display: grid;
    grid-template-columns: 16px minmax(0, 1fr) 60px 72px;
    grid-template-rows: auto auto;
    column-gap: 6px;
    align-items: center;
    padding: 3px 10px;
    cursor: default;
    user-select: none;
  }
  .entry:hover {
    background: var(--bg-hover);
  }
  .entry.selected {
    background: var(--bg-active);
  }
  .entry :global(svg) {
    color: var(--text-dim);
  }
  .name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .size,
  .mode {
    color: var(--text-dim);
    font-size: 11px;
    text-align: right;
  }
  .date {
    grid-column: 2 / 5;
    color: var(--text-dim);
    font-size: 11px;
  }
  .drop-hint {
    padding: 4px 10px;
    border-top: 1px solid var(--border);
    font-size: 11px;
  }
  .transfers {
    max-height: 40%;
    overflow: auto;
    border-top: 1px solid var(--border);
    padding: 4px 8px 8px;
  }
  .thead {
    display: flex;
    justify-content: space-between;
    align-items: center;
    font-size: 12px;
    color: var(--text-dim);
    padding: 2px 0 4px;
  }
  .link {
    border: none;
    background: none;
    padding: 0;
    color: var(--accent);
    cursor: pointer;
    font-size: 12px;
  }
  .transfer {
    padding: 4px 0;
  }
  .trow {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
  }
  .trow .icon {
    width: 18px;
    height: 18px;
  }
  .tname {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .tinfo {
    color: var(--text-dim);
    font-size: 11px;
    white-space: nowrap;
  }
  .bar {
    height: 4px;
    margin-top: 3px;
    border-radius: 2px;
    background: var(--bg-input);
    overflow: hidden;
  }
  .fill {
    height: 100%;
    background: var(--accent);
  }
  .fill.done {
    background: var(--ok);
  }
  .fill.failed,
  .fill.cancelled {
    background: var(--err);
  }
  .terr {
    color: var(--err);
    font-size: 11px;
  }
</style>
