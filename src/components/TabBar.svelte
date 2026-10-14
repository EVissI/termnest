<script lang="ts">
  import { tick } from 'svelte';
  import { api, type SessionState, type Tab, type TabId, type WindowInfo } from '../lib/ipc';
  import { allPanes } from '../lib/layout/geometry';
  import { attempt, focusedPane, newTab } from '../lib/actions';
  import { myWindow, store, ui, windowLabel } from '../stores/app.svelte';
  import ContextMenu, { type MenuItem } from './ContextMenu.svelte';
  import StatusIndicator from './StatusIndicator.svelte';
  import Icon from './Icon.svelte';

  const colors = [
    { name: 'Без цвета', value: '' },
    { name: 'Красный', value: '#ef4444' },
    { name: 'Оранжевый', value: '#f97316' },
    { name: 'Жёлтый', value: '#eab308' },
    { name: 'Зелёный', value: '#22c55e' },
    { name: 'Голубой', value: '#06b6d4' },
    { name: 'Синий', value: '#3b82f6' },
    { name: 'Фиолетовый', value: '#a855f7' },
  ];

  const win = $derived(myWindow());
  const tabs = $derived(win?.tabs ?? []);

  let bar: HTMLDivElement;
  let renaming = $state<TabId | null>(null);
  let renameValue = $state('');
  let menu = $state<{ x: number; y: number; items: MenuItem[] } | null>(null);

  interface Drag {
    tab: TabId;
    title: string;
    startX: number;
    startY: number;
    x: number;
    y: number;
    active: boolean;
    index: number | null;
    markerX: number;
  }
  let drag = $state<Drag | null>(null);

  function title(tab: Tab): string {
    if (tab.title) return tab.title;
    const panes = allPanes(tab.root);
    const focused = panes.find((p) => p.id === tab.focused) ?? panes[0];
    const label = focused?.spec?.label ?? 'Новая вкладка';
    return panes.length > 1 ? `${label} (+${panes.length - 1})` : label;
  }

  /** The "worst" state among the tab's sessions. */
  function tabState(tab: Tab): SessionState | 'held' | null {
    const states = allPanes(tab.root).map((p) =>
      p.held ? 'held' : p.session !== null ? (store.sessions[p.session]?.state ?? 'connecting') : null,
    );
    for (const s of ['error', 'reconnecting', 'connecting', 'held', 'disconnected', 'connected'] as const) {
      if (states.includes(s)) return s;
    }
    return null;
  }

  async function startRename(tab: Tab): Promise<void> {
    renaming = tab.id;
    renameValue = tab.title ?? title(tab);
    await tick();
    const el = bar.querySelector<HTMLInputElement>('input.rename');
    el?.focus();
    el?.select();
  }

  function finishRename(save: boolean): void {
    const id = renaming;
    renaming = null;
    if (save && id !== null) void attempt(() => api.tabRename(id, renameValue.trim() || null));
  }

  async function openMenu(e: MouseEvent, tab: Tab): Promise<void> {
    e.preventDefault();
    const windows: WindowInfo[] = (await api.windowList().catch(() => [])).filter((w) => w.label !== windowLabel);
    const pane = allPanes(tab.root).find((p) => p.id === tab.focused);
    menu = {
      x: e.clientX,
      y: e.clientY,
      items: [
        { label: 'Переименовать', action: () => startRename(tab) },
        {
          label: 'Цвет',
          children: colors.map((c) => ({
            label: c.name,
            swatch: c.value,
            action: () => attempt(() => api.tabSetColor(tab.id, c.value || null)),
          })),
        },
        {
          label: 'Дублировать сессию',
          disabled: !pane?.spec,
          action: () => pane && attempt(() => api.tabDuplicate(pane.id)),
        },
        { separator: true, label: '' },
        {
          label: 'Переместить в окно',
          disabled: windows.length === 0,
          children: windows.map((w) => ({
            label: `Окно ${w.number} (вкладок: ${w.tabs})`,
            action: () => attempt(() => api.tabMove(tab.id, w.label, null)),
          })),
        },
        { label: 'В новое окно', action: () => attempt(() => api.tabToNewWindow(tab.id)) },
        { separator: true, label: '' },
        { label: 'Закрыть вкладку', danger: true, action: () => attempt(() => api.tabClose(tab.id)) },
      ],
    };
  }

  function onPointerDown(e: PointerEvent, tab: Tab): void {
    if (e.button !== 0 || renaming === tab.id) return;
    if ((e.target as HTMLElement).closest('button.close')) return;
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    drag = {
      tab: tab.id,
      title: title(tab),
      startX: e.clientX,
      startY: e.clientY,
      x: e.clientX,
      y: e.clientY,
      active: false,
      index: null,
      markerX: 0,
    };
  }

  function onPointerMove(e: PointerEvent): void {
    if (!drag) return;
    drag.x = e.clientX;
    drag.y = e.clientY;
    if (!drag.active && Math.hypot(e.clientX - drag.startX, e.clientY - drag.startY) > 6) {
      drag.active = true;
    }
    if (!drag.active) return;
    const r = bar.getBoundingClientRect();
    const overBar = e.clientY >= r.top - 20 && e.clientY <= r.bottom + 20 && e.clientX >= r.left && e.clientX <= r.right;
    if (!overBar) {
      drag.index = null;
      return;
    }
    const els = [...bar.querySelectorAll<HTMLElement>('.tab')];
    let index = els.length;
    let markerX = els.length ? els[els.length - 1].getBoundingClientRect().right : r.left;
    for (let i = 0; i < els.length; i++) {
      const tr = els[i].getBoundingClientRect();
      if (e.clientX < tr.left + tr.width / 2) {
        index = i;
        markerX = tr.left;
        break;
      }
    }
    drag.index = index;
    drag.markerX = markerX - r.left;
  }

  function onPointerUp(e: PointerEvent, tab: Tab): void {
    const d = drag;
    drag = null;
    if (!d) return;
    if (!d.active) {
      void attempt(() => api.tabActivate(tab.id));
      return;
    }
    const inside = e.clientX >= 0 && e.clientY >= 0 && e.clientX <= window.innerWidth && e.clientY <= window.innerHeight;
    if (d.index !== null) {
      const current = tabs.findIndex((t) => t.id === d.tab);
      if (d.index !== current && d.index !== current + 1) {
        void attempt(() => api.tabMove(d.tab, windowLabel, d.index));
      }
    } else if (!inside) {
      const dpr = window.devicePixelRatio || 1;
      void attempt(() => api.tabDrop(d.tab, e.screenX * dpr, e.screenY * dpr));
    }
  }

  function onAuxClick(e: MouseEvent, tab: Tab): void {
    if (e.button === 1) {
      e.preventDefault();
      void attempt(() => api.tabClose(tab.id));
    }
  }

  const broadcastCount = $derived.by(() => {
    if (!ui.broadcast) return 0;
    const active = tabs.find((t) => t.id === win?.active);
    if (!active) return 0;
    const panes = allPanes(active.root);
    return ui.selected.filter((id) => panes.some((p) => p.id === id)).length || panes.length;
  });
</script>

<div class="tabbar">
  <div class="tabs" bind:this={bar}>
    {#each tabs as tab (tab.id)}
      <div
        class="tab"
        class:active={tab.id === win?.active}
        class:dragging={drag?.active && drag.tab === tab.id}
        style:--tab-color={tab.color ?? 'transparent'}
        role="tab"
        tabindex="0"
        aria-selected={tab.id === win?.active}
        onpointerdown={(e) => onPointerDown(e, tab)}
        onpointermove={onPointerMove}
        onpointerup={(e) => onPointerUp(e, tab)}
        onpointercancel={() => (drag = null)}
        ondblclick={() => startRename(tab)}
        oncontextmenu={(e) => openMenu(e, tab)}
        onauxclick={(e) => onAuxClick(e, tab)}
        onkeydown={(e) => e.key === 'Enter' && attempt(() => api.tabActivate(tab.id))}
      >
        <StatusIndicator state={tabState(tab)} label={false} />
        {#if renaming === tab.id}
          <input
            class="rename"
            bind:value={renameValue}
            onkeydown={(e) => {
              e.stopPropagation();
              if (e.key === 'Enter') finishRename(true);
              if (e.key === 'Escape') finishRename(false);
            }}
            onblur={() => finishRename(true)}
            aria-label="Название вкладки"
          />
        {:else}
          <span class="title" title={title(tab)}>{title(tab)}</span>
        {/if}
        <button class="close icon" type="button" title="Закрыть вкладку" onclick={() => attempt(() => api.tabClose(tab.id))}>
          <Icon name="x" size={12} />
        </button>
      </div>
    {/each}
    {#if drag?.active && drag.index !== null}
      <div class="marker" style:left="{drag.markerX}px"></div>
    {/if}
    <button class="icon new" type="button" title="Новая вкладка (Ctrl+Shift+T)" onclick={newTab}><Icon name="plus" /></button>
  </div>

  <div class="tools">
    {#if ui.broadcast}
      <span class="broadcast-pill" title="Ввод с клавиатуры дублируется в выбранные панели">
        <Icon name="broadcast" size={13} /> BROADCAST: {broadcastCount}
      </span>
    {/if}
    <button
      class="icon"
      class:active={ui.broadcast}
      type="button"
      title="Режим broadcast (Ctrl+Shift+B)"
      onclick={() => (ui.broadcast = !ui.broadcast)}
    >
      <Icon name="broadcast" />
    </button>
    <button
      class="icon"
      class:active={ui.sftpOpen}
      type="button"
      title="SFTP (Ctrl+Shift+S)"
      disabled={!focusedPane()?.session}
      onclick={() => (ui.sftpOpen = !ui.sftpOpen)}
    >
      <Icon name="folder" />
    </button>
  </div>
</div>

{#if drag?.active}
  <div class="ghost" style:left="{drag.x + 12}px" style:top="{drag.y + 8}px">{drag.title}</div>
{/if}

{#if menu}
  <ContextMenu x={menu.x} y={menu.y} items={menu.items} onclose={() => (menu = null)} />
{/if}

<style>
  .tabbar {
    display: flex;
    align-items: stretch;
    height: 34px;
    background: var(--bg-panel);
    border-bottom: 1px solid var(--border);
    min-width: 0;
  }
  .tabs {
    position: relative;
    display: flex;
    align-items: stretch;
    flex: 1;
    min-width: 0;
    overflow-x: auto;
    overflow-y: hidden;
    scrollbar-width: thin;
  }
  .tab {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 90px;
    max-width: 220px;
    padding: 0 4px 0 10px;
    border-right: 1px solid var(--border);
    border-top: 2px solid var(--tab-color);
    color: var(--text-dim);
    cursor: default;
    user-select: none;
    touch-action: none;
  }
  .tab:hover {
    background: var(--bg-hover);
  }
  .tab.active {
    background: var(--bg);
    color: var(--text);
  }
  .tab.dragging {
    opacity: 0.5;
  }
  .title {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .rename {
    flex: 1;
    width: 100%;
    padding: 2px 4px;
  }
  .close {
    width: 20px;
    height: 20px;
    opacity: 0;
  }
  .tab:hover .close,
  .tab.active .close {
    opacity: 1;
  }
  .new {
    align-self: center;
    margin-left: 4px;
    flex-shrink: 0;
  }
  .marker {
    position: absolute;
    top: 4px;
    bottom: 4px;
    width: 2px;
    margin-left: -1px;
    background: var(--accent);
    pointer-events: none;
  }
  .tools {
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 0 6px;
  }
  .broadcast-pill {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 2px 8px;
    margin-right: 4px;
    border-radius: 10px;
    background: var(--broadcast);
    color: #fff;
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.04em;
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
</style>
