<script lang="ts">
  import { api, type Host } from '../lib/ipc';
  import { attempt, openSource } from '../lib/actions';
  import { linkGroup, linkHost } from '../lib/projects';
  import { askText, confirmDialog, store, toast, ui } from '../stores/app.svelte';
  import ContextMenu, { type MenuItem } from './ContextMenu.svelte';
  import Icon from './Icon.svelte';

  let filter = $state('');
  let quick = $state('');
  let menu = $state<{ x: number; y: number; items: MenuItem[] } | null>(null);
  let newGroup = $state<string | null>(null);

  function matches(h: Host): boolean {
    const q = filter.trim().toLowerCase();
    if (!q) return true;
    return [h.name, h.host, h.user, h.group ?? '', h.note].some((v) => v.toLowerCase().includes(q));
  }

  const sections = $derived.by(() => {
    const visible = store.hosts.hosts.filter(matches);
    const groups = store.hosts.groups.map((g) => ({
      name: g.name as string | null,
      collapsed: g.collapsed && !filter.trim(),
      hosts: visible.filter((h) => h.group === g.name),
    }));
    const ungrouped = visible.filter((h) => !h.group || !store.hosts.groups.some((g) => g.name === h.group));
    const all = [{ name: null as string | null, collapsed: false, hosts: ungrouped }, ...groups];
    // Hide empty groups while searching.
    return filter.trim() ? all.filter((s) => s.hosts.length > 0) : all;
  });

  function blankHost(group: string | null = null): Host {
    return { id: '', name: '', host: '', port: 22, user: '', key_path: null, group, note: '' };
  }

  function connect(h: Host, newTab = false): void {
    void openSource({ kind: 'host', id: h.id }, newTab);
  }

  function submitQuick(e: SubmitEvent): void {
    e.preventDefault();
    const target = quick.trim();
    if (!target) return;
    void openSource({ kind: 'target', target }, true).then(() => (quick = ''));
  }

  async function remove(h: Host): Promise<void> {
    const ok = await confirmDialog({
      title: 'Удалить сервер',
      message: `Удалить «${h.name}» (${h.user}@${h.host})?`,
      confirmLabel: 'Удалить',
      danger: true,
    });
    if (ok) await attempt(() => api.hostDelete(h.id));
  }

  function hostMenu(e: MouseEvent, h: Host): void {
    e.preventDefault();
    menu = {
      x: e.clientX,
      y: e.clientY,
      items: [
        { label: 'Подключиться', action: () => connect(h) },
        { label: 'Подключиться в новой вкладке', action: () => connect(h, true) },
        { separator: true, label: '' },
        { label: 'Редактировать', action: () => (ui.hostDialog = { ...h }) },
        { label: 'Дублировать', action: () => attempt(() => api.hostDuplicate(h.id)) },
        { label: 'Установить ключ на сервер...', action: () => (ui.deploy = { hostId: h.id }) },
        { label: h.project ? 'Сменить проект...' : 'Привязать к проекту...', action: () => linkHost(h.id) },
        ...(h.project ? [{ label: 'Отвязать от проекта', action: () => attempt(() => api.hostSetProject(h.id, null)) }] : []),
        { separator: true, label: '' },
        { label: 'Удалить', danger: true, action: () => remove(h) },
      ],
    };
  }

  async function renameGroup(name: string): Promise<void> {
    const next = await askText('Переименовать группу', 'Новое имя группы', name);
    if (next && next.trim() && next.trim() !== name) await attempt(() => api.groupRename(name, next.trim()));
  }

  async function deleteGroup(name: string): Promise<void> {
    const ok = await confirmDialog({
      title: 'Удалить группу',
      message: `Удалить группу «${name}»? Серверы из неё останутся без группы.`,
      confirmLabel: 'Удалить',
      danger: true,
    });
    if (ok) await attempt(() => api.groupDelete(name));
  }

  function groupMenu(e: MouseEvent, name: string): void {
    e.preventDefault();
    menu = {
      x: e.clientX,
      y: e.clientY,
      items: [
        { label: 'Добавить сервер в группу', action: () => (ui.hostDialog = blankHost(name)) },
        { label: 'Переименовать', action: () => renameGroup(name) },
        { label: 'Проект группы...', action: () => linkGroup(name) },
        ...(store.hosts.groups.find((g) => g.name === name)?.project
          ? [{ label: 'Отвязать проект группы', action: () => attempt(() => api.groupSetProject(name, null)) }]
          : []),
        { label: 'Удалить группу', danger: true, action: () => deleteGroup(name) },
      ],
    };
  }

  async function importConfig(): Promise<void> {
    const added = await attempt(() => api.importSshConfig());
    if (added !== undefined) toast(added > 0 ? `Импортировано серверов: ${added}` : 'Новых серверов в ~/.ssh/config нет', 'ok');
  }

  function submitGroup(e: SubmitEvent): void {
    e.preventDefault();
    const name = newGroup?.trim();
    newGroup = null;
    if (name) void attempt(() => api.groupAdd(name));
  }
</script>

<div class="head">
  <h3>Серверы</h3>
  <button class="icon" type="button" title="Новая группа" onclick={() => (newGroup = '')}><Icon name="folder-plus" /></button>
  <button class="icon" type="button" title="Импорт из ~/.ssh/config" onclick={importConfig}><Icon name="import" /></button>
  <button class="icon" type="button" title="Добавить сервер" onclick={() => (ui.hostDialog = blankHost())}><Icon name="plus" /></button>
</div>

<form class="quick" onsubmit={submitQuick}>
  <input bind:value={quick} placeholder="user@host:port" aria-label="Быстрое подключение" spellcheck="false" />
  <button class="icon" type="submit" title="Подключиться без сохранения" disabled={!quick.trim()}><Icon name="play" size={14} /></button>
</form>

<div class="search">
  <Icon name="search" size={13} />
  <input bind:value={filter} placeholder="Поиск" aria-label="Поиск серверов" spellcheck="false" />
</div>

{#if newGroup !== null}
  <form class="new-group" onsubmit={submitGroup}>
    <!-- svelte-ignore a11y_autofocus -->
    <input bind:value={newGroup} placeholder="Имя группы" autofocus onblur={() => (newGroup = null)} aria-label="Имя группы" />
  </form>
{/if}

<div class="list">
  {#each sections as section (section.name ?? '')}
    {#if section.name !== null}
      <button
        class="group"
        type="button"
        onclick={() => attempt(() => api.groupSetCollapsed(section.name!, !section.collapsed))}
        oncontextmenu={(e) => groupMenu(e, section.name!)}
      >
        <Icon name={section.collapsed ? 'chevron-right' : 'chevron-down'} size={12} />
        <span>{section.name}</span>
        <span class="count">{section.hosts.length}</span>
      </button>
    {/if}
    {#if !section.collapsed}
      {#each section.hosts as h (h.id)}
        <button
          class="host"
          class:indent={section.name !== null}
          type="button"
          title={h.note || `${h.user}@${h.host}:${h.port}`}
          ondblclick={() => connect(h)}
          oncontextmenu={(e) => hostMenu(e, h)}
          onkeydown={(e) => e.key === 'Enter' && connect(h)}
        >
          <Icon name="terminal" size={13} />
          <span class="name">{h.name}</span>
          <span class="addr">{h.user}@{h.host}</span>
        </button>
      {/each}
    {/if}
  {/each}
  {#if store.hosts.hosts.length === 0}
    <p class="hint empty">
      Серверов пока нет. Добавьте сервер кнопкой «+» или импортируйте из ~/.ssh/config. Двойной клик по серверу подключает его.
    </p>
  {/if}
</div>

{#if menu}
  <ContextMenu x={menu.x} y={menu.y} items={menu.items} onclose={() => (menu = null)} />
{/if}

<style>
  .head {
    display: flex;
    align-items: center;
    gap: 2px;
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
  .quick,
  .search,
  .new-group {
    display: flex;
    align-items: center;
    gap: 4px;
    margin: 4px 8px;
  }
  .quick input,
  .new-group input {
    flex: 1;
  }
  .search {
    padding-left: 8px;
    border: 1px solid var(--border);
    border-radius: 4px;
    background: var(--bg-input);
    color: var(--text-dim);
  }
  .search input {
    flex: 1;
    border: none;
    background: none;
  }
  .list {
    flex: 1;
    overflow: auto;
    padding: 4px 0 8px;
  }
  .group,
  .host {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    padding: 4px 10px;
    border: none;
    background: none;
    text-align: left;
    cursor: pointer;
    min-width: 0;
  }
  .group {
    color: var(--text-dim);
    font-weight: 600;
    font-size: 12px;
    margin-top: 4px;
  }
  .group:hover,
  .host:hover {
    background: var(--bg-hover);
  }
  .count {
    margin-left: auto;
    font-weight: 400;
  }
  .host.indent {
    padding-left: 24px;
  }
  .host :global(svg) {
    flex-shrink: 0;
    color: var(--text-dim);
  }
  .name {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .addr {
    margin-left: auto;
    color: var(--text-dim);
    font-size: 11px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 50%;
  }
  .empty {
    padding: 8px 12px;
  }
</style>
