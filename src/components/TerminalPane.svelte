<script lang="ts">
  import { onMount } from 'svelte';
  import { TerminalView } from '../lib/terminal/TerminalView';
  import { api, type Pane, type SessionStatus } from '../lib/ipc';
  import { attempt, broadcastSessions, focusPane, togglePaneSelection } from '../lib/actions';
  import { confirmDialog, sessionCwd, store, terminals, ui } from '../stores/app.svelte';
  import StatusIndicator from './StatusIndicator.svelte';
  import PanePicker from './PanePicker.svelte';
  import Icon from './Icon.svelte';
  import ContextMenu, { type MenuItem } from './ContextMenu.svelte';
  import { effectiveProject, linkHost, linkPane } from '../lib/projects';

  let { pane, visible, focused }: { pane: Pane; visible: boolean; focused: boolean } = $props();

  let container: HTMLDivElement;
  let view = $state.raw<TerminalView | null>(null);
  let attaching = false;
  let searchInput = $state<HTMLInputElement | null>(null);
  let query = $state('');
  let now = $state(Date.now());
  let reconnectSince = $state(Date.now());

  const status = $derived<SessionStatus | undefined>(pane.session !== null ? store.sessions[pane.session] : undefined);
  const selected = $derived(ui.selected.includes(pane.id));
  const broadcasting = $derived(ui.broadcast && (selected || ui.selected.length === 0));
  const label = $derived(pane.spec?.label ?? 'Новая панель');
  const indicator = $derived(pane.held ? 'held' : (status?.state ?? null));

  onMount(() => {
    const settings = store.settings!;
    const v = new TerminalView(container, settings, {
      onInput: (data) => {
        if (ui.broadcast && focused) {
          const targets = broadcastSessions();
          if (pane.session !== null && !targets.includes(pane.session)) targets.push(pane.session);
          if (targets.length > 0) api.writeMany(targets, data).catch(() => {});
        } else if (pane.session !== null) {
          api.write(pane.session, data).catch(() => {});
        }
      },
      onResize: (cols, rows) => {
        if (pane.session !== null) api.resize(pane.session, cols, rows).catch(() => {});
      },
      onCwd: (path) => {
        if (pane.session !== null) sessionCwd.set(pane.session, path);
      },
      confirmPaste: (text) => {
        const lines = text.split(/\r?\n/);
        return confirmDialog({
          title: 'Вставка нескольких строк',
          message: `Вставить ${lines.length} строк(и)? Каждая строка может сразу выполниться как команда.`,
          detail: lines.slice(0, 12).join('\n') + (lines.length > 12 ? '\n...' : ''),
          confirmLabel: 'Вставить',
        });
      },
      onFocus: () => {
        if (!focused) void attempt(() => api.paneFocus(pane.id));
      },
    });
    view = v;
    terminals.set(pane.id, v);
    return () => {
      terminals.delete(pane.id);
      view = null;
      v.dispose();
    };
  });

  // Start the connection once the panel knows its size.
  $effect(() => {
    const v = view;
    if (!v || pane.session !== null || !pane.spec || pane.held || attaching) return;
    attaching = true;
    api
      .paneAttach(pane.id, v.cols, v.rows)
      .catch(() => {})
      .finally(() => (attaching = false));
  });

  // Subscribe to the session output; buffered output is replayed first.
  $effect(() => {
    const id = pane.session;
    const v = view;
    if (id === null || !v) return;

    v.reset();
    let subscription: number | null = null;
    let cancelled = false;
    api
      .subscribe(id, (data) => v.write(data))
      .then((sub) => {
        if (cancelled) api.unsubscribe(id, sub).catch(() => {});
        else subscription = sub;
      })
      .catch(() => {});
    api.resize(id, v.cols, v.rows).catch(() => {});

    return () => {
      cancelled = true;
      if (subscription !== null) api.unsubscribe(id, subscription).catch(() => {});
    };
  });

  $effect(() => {
    view?.setVisible(visible);
  });

  $effect(() => {
    if (focused && visible && view && pane.spec) view.focus();
  });

  $effect(() => {
    if (ui.searchPane === pane.id) searchInput?.focus();
  });

  // Countdown for the reconnect banner; the timer only runs while it is shown.
  $effect(() => {
    if (status?.state !== 'reconnecting' || status.retry_in_ms === null) return;
    reconnectSince = Date.now();
    now = Date.now();
    const t = setInterval(() => (now = Date.now()), 500);
    return () => clearInterval(t);
  });

  const retryIn = $derived(
    status?.retry_in_ms != null ? Math.max(0, Math.ceil((status.retry_in_ms - (now - reconnectSince)) / 1000)) : null,
  );

  const project = $derived(effectiveProject(pane));
  const projectSourceLabel = { pane: 'только эта панель', host: 'от сервера', group: 'от группы' } as const;
  let projectMenu = $state<{ x: number; y: number; items: MenuItem[] } | null>(null);

  function openProjectMenu(e: MouseEvent): void {
    e.stopPropagation();
    const hostId = pane.spec?.host_id;
    const host = hostId ? store.hosts.hosts.find((h) => h.id === hostId) : undefined;
    const items: MenuItem[] = [];
    if (project) items.push({ label: project.path, disabled: true }, { separator: true, label: '' });
    items.push({ label: 'Привязать эту панель к проекту...', action: () => linkPane(pane) });
    if (pane.project) {
      items.push({ label: 'Отвязать панель', action: () => attempt(() => api.paneSetProject(pane.id, null)) });
    }
    if (host) {
      items.push({ separator: true, label: '' });
      items.push({ label: `Привязать сервер «${host.name}» к проекту...`, action: () => linkHost(host.id) });
      if (host.project) {
        items.push({ label: 'Отвязать сервер', action: () => attempt(() => api.hostSetProject(host.id, null)) });
      }
    }
    projectMenu = { x: e.clientX, y: e.clientY, items };
  }

  function openServerInfo(): void {
    const title = pane.spec ? pane.spec.label || `${pane.spec.user}@${pane.spec.host}` : 'Сервер';
    ui.serverInfo = { host: pane.spec?.host_id ?? null, session: pane.session, title };
  }

  function splitThis(direction: 'horizontal' | 'vertical'): void {
    const source = pane.spec ? ({ kind: 'pane', pane: pane.id } as const) : ({ kind: 'empty' } as const);
    void attempt(() => api.paneSplit(pane.id, direction, source));
  }

  function sessionCall(fn: (id: number) => Promise<void>): void {
    const id = pane.session;
    if (id !== null) void attempt(() => fn(id));
  }

  function connect(): void {
    if (!view) return;
    void attempt(() => api.paneConnect(pane.id, view!.cols, view!.rows));
  }

  function search(backwards = false): void {
    if (!view || !query) return;
    if (backwards) view.findPrevious(query);
    else view.findNext(query);
  }

  function closeSearch(): void {
    ui.searchPane = null;
    view?.clearSearch();
    view?.focus();
  }

  function onSearchKey(e: KeyboardEvent): void {
    if (e.key === 'Enter') {
      e.preventDefault();
      search(e.shiftKey);
    } else if (e.key === 'Escape') {
      e.preventDefault();
      closeSearch();
    }
  }

  function showHostKey(): void {
    if (status?.error?.host_key && pane.session !== null) {
      ui.hostKeyChanged = { info: status.error.host_key, session: pane.session };
    }
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="pane"
  class:focused
  class:broadcast={ui.broadcast && broadcasting}
  onmousedown={() => !focused && focusPane(pane.id)}
>
  <div class="header">
    <input
      type="checkbox"
      class="select"
      checked={selected}
      onchange={() => togglePaneSelection(pane.id)}
      title="Выбрать панель для broadcast и сниппетов"
      aria-label="Выбрать панель"
    />
    <StatusIndicator state={indicator} label={false} />
    <span class="label" title={pane.spec ? `${pane.spec.user}@${pane.spec.host}:${pane.spec.port}` : ''}>{label}</span>
    {#if project}
      <button
        class="project"
        type="button"
        title="Проект: {project.path} ({projectSourceLabel[project.source]})"
        onclick={(e) => openProjectMenu(e)}
      >
        <Icon name="folder" size={11} />
        <span>{project.name}</span>
      </button>
    {/if}
    <span class="spacer"></span>
    {#if !project && pane.spec}
      <button class="icon" type="button" title="Привязать к проекту" onclick={(e) => openProjectMenu(e)}>
        <Icon name="link" size={13} />
      </button>
    {/if}
    {#if pane.spec?.host_id || pane.session !== null}
      <button class="icon" type="button" title="Информация о сервере" onclick={openServerInfo}>
        <Icon name="activity" size={13} />
      </button>
    {/if}
    {#if pane.spec}
      <button class="icon" type="button" title="Отправить на фикс в Claude (Ctrl+Shift+A)" onclick={() => (ui.sendToAgent = pane.id)}>
        <Icon name="agent" size={13} />
      </button>
    {/if}
    <button class="icon" type="button" title="Сплит вправо" onclick={() => splitThis('horizontal')}>
      <Icon name="split-h" size={13} />
    </button>
    <button class="icon" type="button" title="Сплит вниз" onclick={() => splitThis('vertical')}>
      <Icon name="split-v" size={13} />
    </button>
    <button class="icon" type="button" title="Закрыть панель" onclick={() => attempt(() => api.paneClose(pane.id))}>
      <Icon name="x" size={13} />
    </button>
  </div>

  <div class="body">
    <div class="term" bind:this={container} class:hidden={!pane.spec}></div>

    {#if !pane.spec}
      <PanePicker paneId={pane.id} />
    {:else if pane.held}
      <div class="banner">
        <span>Подключение не восстановлено после запуска</span>
        <button class="btn primary" type="button" onclick={connect}>Подключить</button>
      </div>
    {:else if status?.state === 'reconnecting'}
      <div class="banner warn">
        <Icon name="refresh" size={14} />
        <span>
          Переподключение{status.attempt ? `, попытка ${status.attempt}` : ''}{retryIn !== null ? ` через ${retryIn} с` : '...'}
          {#if status.error}<span class="dim"> ({status.error.message})</span>{/if}
        </span>
        <button class="btn" type="button" onclick={() => sessionCall(api.reconnectNow)}>Сейчас</button>
        <button class="btn" type="button" onclick={() => sessionCall(api.cancelReconnect)}>Отмена</button>
      </div>
    {:else if status?.state === 'error'}
      <div class="banner err">
        <Icon name="warning" size={14} />
        <span>{status.error?.message ?? 'Ошибка подключения'}</span>
        {#if status.error?.kind === 'host_key_changed'}
          <button class="btn danger" type="button" onclick={showHostKey}>Подробнее</button>
        {:else}
          <button class="btn" type="button" onclick={() => sessionCall(api.retry)}>Повторить</button>
        {/if}
        <button class="btn" type="button" onclick={() => attempt(() => api.paneClose(pane.id))}>Закрыть</button>
      </div>
    {:else if status?.state === 'disconnected'}
      <div class="banner">
        <span>Сеанс завершён</span>
        <button class="btn" type="button" onclick={() => sessionCall(api.retry)}>Переподключить</button>
        <button class="btn" type="button" onclick={() => attempt(() => api.paneClose(pane.id))}>Закрыть</button>
      </div>
    {/if}

    {#if ui.searchPane === pane.id}
      <div class="search">
        <input
          bind:this={searchInput}
          bind:value={query}
          placeholder="Поиск"
          onkeydown={onSearchKey}
          oninput={() => search()}
          aria-label="Поиск в терминале"
        />
        <button class="icon" type="button" title="Назад (Shift+Enter)" onclick={() => search(true)}><Icon name="up" size={13} /></button>
        <button class="icon" type="button" title="Вперёд (Enter)" onclick={() => search()}><Icon name="chevron-down" size={13} /></button>
        <button class="icon" type="button" title="Закрыть (Esc)" onclick={closeSearch}><Icon name="x" size={13} /></button>
      </div>
    {/if}
  </div>
</div>

{#if projectMenu}
  <ContextMenu x={projectMenu.x} y={projectMenu.y} items={projectMenu.items} onclose={() => (projectMenu = null)} />
{/if}

<style>
  .pane {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    border: 1px solid transparent;
    background: var(--bg);
  }
  .pane.focused {
    border-color: color-mix(in srgb, var(--accent) 55%, transparent);
  }
  .pane.broadcast {
    border-color: var(--broadcast);
  }
  .header {
    display: flex;
    align-items: center;
    gap: 6px;
    height: 24px;
    padding: 0 2px 0 6px;
    background: var(--bg-panel);
    border-bottom: 1px solid var(--border);
    font-size: 12px;
    min-width: 0;
  }
  .header .icon {
    width: 20px;
    height: 20px;
  }
  .select {
    margin: 0;
  }
  .project {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    max-width: 40%;
    padding: 0 6px;
    height: 18px;
    border: 1px solid var(--border);
    border-radius: 9px;
    background: var(--bg-input);
    color: var(--text-dim);
    font-size: 11px;
    cursor: pointer;
    min-width: 0;
  }
  .project span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .project:hover {
    color: var(--text);
    border-color: var(--accent);
  }
  .label {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }
  .focused .label {
    color: var(--text);
  }
  .spacer {
    flex: 1;
  }
  .body {
    position: relative;
    flex: 1;
    min-height: 0;
  }
  .term {
    position: absolute;
    inset: 2px 0 0 6px;
  }
  .term.hidden {
    visibility: hidden;
  }
  .banner {
    position: absolute;
    left: 8px;
    right: 8px;
    top: 8px;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 10px;
    border-radius: 6px;
    background: var(--bg-panel);
    border: 1px solid var(--border);
    box-shadow: 0 4px 16px rgb(0 0 0 / 30%);
    z-index: 5;
  }
  .banner > span {
    flex: 1;
    min-width: 0;
  }
  .banner.warn {
    border-color: color-mix(in srgb, var(--warn) 60%, var(--border));
  }
  .banner.err {
    border-color: color-mix(in srgb, var(--err) 70%, var(--border));
  }
  .banner.err :global(svg) {
    color: var(--err);
    flex-shrink: 0;
  }
  .dim {
    color: var(--text-dim);
  }
  .search {
    position: absolute;
    top: 6px;
    right: 14px;
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 4px;
    background: var(--bg-panel);
    border: 1px solid var(--border);
    border-radius: 6px;
    z-index: 6;
  }
  .search input {
    width: 200px;
  }
</style>
