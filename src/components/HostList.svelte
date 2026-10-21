<script lang="ts">
  import { api, type Host } from '../lib/ipc';
  import { attempt, openSource } from '../lib/actions';
  import { linkGroup, linkHost } from '../lib/projects';
  import { askText, confirmDialog, store, toast, ui } from '../stores/app.svelte';
  import ContextMenu, { type MenuItem } from './ContextMenu.svelte';
  import Icon from './Icon.svelte';

  type Row =
    | { kind: 'group'; key: string; name: string; parent: string | null; depth: number; collapsed: boolean; count: number }
    | { kind: 'host'; key: string; host: Host; group: string | null; depth: number };

  type Place = 'before' | 'after' | 'into';

  interface Drag {
    row: Row;
    label: string;
    startX: number;
    startY: number;
    x: number;
    y: number;
    active: boolean;
    /** Row under the cursor (`null` with `root` - the empty area below the list). */
    target: Row | null;
    root: boolean;
    place: Place;
    valid: boolean;
  }

  let filter = $state('');
  let quick = $state('');
  let menu = $state<{ x: number; y: number; items: MenuItem[] } | null>(null);
  let newGroup = $state<string | null>(null);
  let drag = $state<Drag | null>(null);
  let list = $state<HTMLDivElement>();
  /** A finished drag must not also count as a click on the row. */
  let swallowClick = false;

  function matches(h: Host): boolean {
    const q = filter.trim().toLowerCase();
    if (!q) return true;
    return [h.name, h.host, h.user, h.group ?? '', h.note].some((v) => v.toLowerCase().includes(q));
  }

  const rows = $derived.by(() => {
    const searching = filter.trim() !== '';
    const groups = store.hosts.groups;
    const known = new Set(groups.map((g) => g.name));
    const groupOf = (h: Host) => (h.group && known.has(h.group) ? h.group : null);
    const parentOf = (name: string | null | undefined) => (name && known.has(name) ? name : null);
    const total = (name: string): number =>
      store.hosts.hosts.filter((h) => groupOf(h) === name && matches(h)).length +
      groups.filter((g) => parentOf(g.parent) === name).reduce((n, g) => n + total(g.name), 0);

    const out: Row[] = [];
    const walk = (parent: string | null, depth: number) => {
      // The store keeps groups a tree; the depth guard only protects the UI.
      if (depth > 32) return;
      for (const g of groups.filter((x) => parentOf(x.parent) === parent)) {
        const count = total(g.name);
        if (searching && count === 0) continue;
        const collapsed = g.collapsed && !searching;
        out.push({ kind: 'group', key: `g:${g.name}`, name: g.name, parent, depth, collapsed, count });
        if (!collapsed) walk(g.name, depth + 1);
      }
      for (const h of store.hosts.hosts.filter((x) => groupOf(x) === parent && matches(x))) {
        out.push({ kind: 'host', key: `h:${h.id}`, host: h, group: parent, depth });
      }
    };
    walk(null, 0);
    return out;
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
        { label: 'Информация о сервере', action: () => (ui.serverInfo = { host: h.id, session: null, title: h.name }) },
        { separator: true, label: '' },
        { label: 'Редактировать', action: () => (ui.hostDialog = { ...h }) },
        { label: 'Дублировать', action: () => attempt(() => api.hostDuplicate(h.id)) },
        { label: 'Установить ключ на сервер...', action: () => (ui.deploy = { hostId: h.id }) },
        { label: h.project ? 'Сменить проект...' : 'Привязать к проекту...', action: () => linkHost(h.id) },
        ...(h.project ? [{ label: 'Отвязать от проекта', action: () => attempt(() => api.hostSetProject(h.id, null)) }] : []),
        ...(h.group ? [{ label: 'Убрать из группы', action: () => attempt(() => api.hostMove(h.id, null, null)) }] : []),
        { separator: true, label: '' },
        { label: 'Удалить', danger: true, action: () => remove(h) },
      ],
    };
  }

  async function renameGroup(name: string): Promise<void> {
    const next = await askText('Переименовать группу', 'Новое имя группы', name);
    if (next && next.trim() && next.trim() !== name) await attempt(() => api.groupRename(name, next.trim()));
  }

  async function addSubgroup(parent: string): Promise<void> {
    const name = await askText('Новая подгруппа', `Имя подгруппы в «${parent}»`);
    if (name && name.trim()) await attempt(() => api.groupAdd(name.trim(), parent));
  }

  async function deleteGroup(name: string, parent: string | null): Promise<void> {
    const ok = await confirmDialog({
      title: 'Удалить группу',
      message: parent
        ? `Удалить группу «${name}»? Её серверы и подгруппы перейдут в «${parent}».`
        : `Удалить группу «${name}»? Её подгруппы станут группами верхнего уровня, серверы останутся без группы.`,
      confirmLabel: 'Удалить',
      danger: true,
    });
    if (ok) await attempt(() => api.groupDelete(name));
  }

  function groupMenu(e: MouseEvent, name: string, parent: string | null): void {
    e.preventDefault();
    menu = {
      x: e.clientX,
      y: e.clientY,
      items: [
        { label: 'Добавить сервер в группу', action: () => (ui.hostDialog = blankHost(name)) },
        { label: 'Новая подгруппа...', action: () => addSubgroup(name) },
        { label: 'Переименовать', action: () => renameGroup(name) },
        { label: 'Проект группы...', action: () => linkGroup(name) },
        ...(store.hosts.groups.find((g) => g.name === name)?.project
          ? [{ label: 'Отвязать проект группы', action: () => attempt(() => api.groupSetProject(name, null)) }]
          : []),
        ...(parent ? [{ label: 'Вынести на верхний уровень', action: () => attempt(() => api.groupMove(name, null, null)) }] : []),
        { separator: true, label: '' },
        { label: 'Удалить группу', danger: true, action: () => deleteGroup(name, parent) },
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

  function toggleGroup(row: Row & { kind: 'group' }): void {
    if (swallowClick) return;
    void attempt(() => api.groupSetCollapsed(row.name, !row.collapsed));
  }

  // ---- Drag and drop ----

  /** `name` is `ancestor` or lies inside it. */
  function within(name: string | null, ancestor: string): boolean {
    // Bounded by the group count, so a broken tree cannot loop forever.
    for (let i = 0; name && i <= store.hosts.groups.length; i++) {
      if (name === ancestor) return true;
      const current: string = name;
      name = store.hosts.groups.find((g) => g.name === current)?.parent ?? null;
    }
    return false;
  }

  function onPointerDown(e: PointerEvent, row: Row): void {
    if (e.button !== 0) return;
    swallowClick = false;
    // Moves and the release then reach the window even outside the row.
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    drag = {
      row,
      label: row.kind === 'group' ? row.name : row.host.name,
      startX: e.clientX,
      startY: e.clientY,
      x: e.clientX,
      y: e.clientY,
      active: false,
      target: null,
      root: false,
      place: 'into',
      valid: false,
    };
  }

  function onPointerMove(e: PointerEvent): void {
    if (!drag) return;
    // The button was released outside the list before the drag started.
    if (e.buttons === 0) {
      drag = null;
      return;
    }
    drag.x = e.clientX;
    drag.y = e.clientY;
    if (!drag.active) {
      if (Math.hypot(e.clientX - drag.startX, e.clientY - drag.startY) < 6) return;
      drag.active = true;
    }
    const el = document.elementFromPoint(e.clientX, e.clientY) as HTMLElement | null;
    const rowEl = el?.closest<HTMLElement>('[data-key]');
    const target = rowEl ? (rows.find((r) => r.key === rowEl.dataset.key) ?? null) : null;
    drag.root = !target && !!el && !!list?.contains(el);
    drag.target = target;
    if (!target || !rowEl) {
      drag.valid = drag.root;
      return;
    }
    const r = rowEl.getBoundingClientRect();
    const y = (e.clientY - r.top) / r.height;
    const dragged = drag.row;
    if (target.kind === 'group') {
      // A server always goes into a group; a group can also land next to it.
      drag.place = dragged.kind === 'host' ? 'into' : y < 0.25 ? 'before' : y > 0.75 ? 'after' : 'into';
    } else {
      drag.place = y < 0.5 ? 'before' : 'after';
    }
    if (dragged.kind === 'group') {
      const destination = target.kind === 'group' && drag.place === 'into' ? target.name : target.kind === 'group' ? target.parent : target.group;
      drag.valid = !(target.kind === 'group' && target.name === dragged.name) && !within(destination, dragged.name);
    } else {
      drag.valid = !(target.kind === 'host' && target.host.id === dragged.host.id);
    }
  }

  /** The server after `h` in the same group, skipping the dragged one. */
  function nextHost(h: Host, skip: string): string | null {
    const all = store.hosts.hosts;
    const i = all.findIndex((x) => x.id === h.id);
    return all.slice(i + 1).find((x) => x.id !== skip && (x.group ?? null) === (h.group ?? null))?.id ?? null;
  }

  /** The group after `name` with the same parent, skipping the dragged one. */
  function nextGroup(name: string, parent: string | null, skip: string): string | null {
    const all = store.hosts.groups;
    const i = all.findIndex((g) => g.name === name);
    return all.slice(i + 1).find((g) => g.name !== skip && (g.parent ?? null) === parent)?.name ?? null;
  }

  function onPointerUp(): void {
    const d = drag;
    drag = null;
    if (!d?.active) return;
    swallowClick = true;
    setTimeout(() => (swallowClick = false), 0);
    if (!d.valid) return;
    const t = d.target;
    if (d.row.kind === 'host') {
      const id = d.row.host.id;
      if (!t) void attempt(() => api.hostMove(id, null, null));
      else if (t.kind === 'group') void attempt(() => api.hostMove(id, t.name, null));
      else {
        const before = d.place === 'before' ? t.host.id : nextHost(t.host, id);
        void attempt(() => api.hostMove(id, t.group, before));
      }
    } else {
      const name = d.row.name;
      if (!t) void attempt(() => api.groupMove(name, null, null));
      else if (t.kind === 'host') void attempt(() => api.groupMove(name, t.group, null));
      else if (d.place === 'into') void attempt(() => api.groupMove(name, t.name, null));
      else {
        const before = d.place === 'before' ? t.name : nextGroup(t.name, t.parent, name);
        void attempt(() => api.groupMove(name, t.parent, before));
      }
    }
  }

  function dropClass(row: Row): string {
    if (!drag?.active || !drag.valid || drag.target?.key !== row.key) return '';
    return `drop-${drag.place}`;
  }
</script>

<svelte:window
  onkeydown={(e) => e.key === 'Escape' && drag?.active && (drag = null)}
  onpointermove={onPointerMove}
  onpointerup={onPointerUp}
/>

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

<div class="list" class:root-drop={drag?.active && drag.root && drag.valid} bind:this={list}>
  {#each rows as row (row.key)}
    {#if row.kind === 'group'}
      <button
        class="group {dropClass(row)}"
        class:dragging={drag?.active && drag.row.key === row.key}
        type="button"
        data-key={row.key}
        style:padding-left="{10 + row.depth * 14}px"
        onclick={() => toggleGroup(row)}
        oncontextmenu={(e) => groupMenu(e, row.name, row.parent)}
        onpointerdown={(e) => onPointerDown(e, row)}
        onpointercancel={() => (drag = null)}
      >
        <Icon name={row.collapsed ? 'chevron-right' : 'chevron-down'} size={12} />
        <span class="gname">{row.name}</span>
        <span class="count">{row.count}</span>
      </button>
    {:else}
      {@const h = row.host}
      <button
        class="host {dropClass(row)}"
        class:dragging={drag?.active && drag.row.key === row.key}
        type="button"
        data-key={row.key}
        style:padding-left="{10 + row.depth * 14}px"
        title={h.note || `${h.user}@${h.host}:${h.port}`}
        ondblclick={() => connect(h)}
        oncontextmenu={(e) => hostMenu(e, h)}
        onkeydown={(e) => e.key === 'Enter' && connect(h)}
        onpointerdown={(e) => onPointerDown(e, row)}
        onpointercancel={() => (drag = null)}
      >
        <Icon name="terminal" size={13} />
        <span class="name">{h.name}</span>
        <span class="addr">{h.user}{h.switch_user ? ` > ${h.switch_user}` : ''}@{h.host}</span>
      </button>
    {/if}
  {/each}
  {#if store.hosts.hosts.length === 0}
    <p class="hint empty">
      Серверов пока нет. Добавьте сервер кнопкой «+» или импортируйте из ~/.ssh/config. Двойной клик по серверу подключает его.
    </p>
  {:else if drag?.active}
    <p class="hint drop-hint">Отпустите на пустом месте, чтобы вынести на верхний уровень</p>
  {/if}
</div>

{#if drag?.active}
  <div class="ghost" class:invalid={!drag.valid} style:left="{drag.x + 12}px" style:top="{drag.y + 8}px">{drag.label}</div>
{/if}

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
  .list.root-drop {
    box-shadow: inset 0 0 0 1px var(--accent);
  }
  .group,
  .host {
    position: relative;
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
    user-select: none;
    touch-action: none;
  }
  .group {
    color: var(--text-dim);
    font-weight: 600;
    font-size: 12px;
    margin-top: 2px;
  }
  .group:hover,
  .host:hover {
    background: var(--bg-hover);
  }
  .dragging {
    opacity: 0.45;
  }
  .drop-into {
    background: var(--bg-hover);
    box-shadow: inset 0 0 0 1px var(--accent);
  }
  .drop-before::before,
  .drop-after::after {
    content: '';
    position: absolute;
    left: 8px;
    right: 8px;
    height: 2px;
    background: var(--accent);
    pointer-events: none;
  }
  .drop-before::before {
    top: -1px;
  }
  .drop-after::after {
    bottom: -1px;
  }
  .gname {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .count {
    margin-left: auto;
    font-weight: 400;
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
  .empty,
  .drop-hint {
    padding: 8px 12px;
  }
  .ghost {
    position: fixed;
    z-index: 400;
    padding: 4px 10px;
    border-radius: 4px;
    background: var(--bg-panel);
    border: 1px solid var(--accent);
    pointer-events: none;
    white-space: nowrap;
  }
  .ghost.invalid {
    border-color: var(--border);
    opacity: 0.7;
  }
</style>
