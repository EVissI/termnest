<script lang="ts">
  import { api, type Snippet } from '../lib/ipc';
  import { attempt } from '../lib/actions';
  import { confirmDialog, store, ui } from '../stores/app.svelte';
  import ContextMenu, { type MenuItem } from './ContextMenu.svelte';
  import Icon from './Icon.svelte';

  let filter = $state('');
  let menu = $state<{ x: number; y: number; items: MenuItem[] } | null>(null);

  const list = $derived(
    store.snippets.filter((s) => {
      const q = filter.trim().toLowerCase();
      if (!q) return true;
      return s.name.toLowerCase().includes(q) || s.tags.some((t) => t.toLowerCase().includes(q)) || s.command.toLowerCase().includes(q);
    }),
  );

  function blank(): Snippet {
    return { id: '', name: '', command: '', tags: [], press_enter: true };
  }

  function send(s: Snippet): void {
    ui.paletteSnippet = s.id;
    ui.palette = true;
  }

  async function remove(s: Snippet): Promise<void> {
    if (await confirmDialog({ title: 'Удалить сниппет', message: `Удалить «${s.name}»?`, confirmLabel: 'Удалить', danger: true })) {
      await attempt(() => api.snippetDelete(s.id));
    }
  }

  function openMenu(e: MouseEvent, s: Snippet): void {
    e.preventDefault();
    menu = {
      x: e.clientX,
      y: e.clientY,
      items: [
        { label: 'Отправить...', action: () => send(s) },
        { label: 'Редактировать', action: () => (ui.snippetEdit = { ...s, tags: [...s.tags] }) },
        { label: 'Дублировать', action: () => attempt(() => api.snippetUpsert({ ...s, id: '', name: `${s.name} (копия)` })) },
        { separator: true, label: '' },
        { label: 'Удалить', danger: true, action: () => remove(s) },
      ],
    };
  }
</script>

<div class="head">
  <h3>Сниппеты</h3>
  <button class="icon" type="button" title="Палитра (Ctrl+Shift+P)" onclick={() => (ui.palette = true)}><Icon name="search" /></button>
  <button class="icon" type="button" title="Новый сниппет" onclick={() => (ui.snippetEdit = blank())}><Icon name="plus" /></button>
</div>

<div class="search">
  <Icon name="search" size={13} />
  <input bind:value={filter} placeholder="Поиск по названию и тегам" aria-label="Поиск сниппетов" />
</div>

<div class="list">
  {#each list as s (s.id)}
    <div class="snippet" role="listitem" oncontextmenu={(e) => openMenu(e, s)}>
      <div class="top">
        <span class="name">{s.name}</span>
        <button class="icon" type="button" title="Отправить" onclick={() => send(s)}><Icon name="play" size={13} /></button>
        <button class="icon" type="button" title="Редактировать" onclick={() => (ui.snippetEdit = { ...s, tags: [...s.tags] })}>
          <Icon name="edit" size={13} />
        </button>
      </div>
      <div class="cmd mono">{s.command}</div>
      {#if s.tags.length}
        <div class="tags">
          {#each s.tags as t (t)}<span class="tag">{t}</span>{/each}
        </div>
      {/if}
    </div>
  {:else}
    <p class="hint empty">
      {store.snippets.length ? 'Ничего не найдено' : 'Сниппетов нет. В команде можно использовать переменные вида {{имя}} - их спросят при отправке.'}
    </p>
  {/each}
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
  .search {
    display: flex;
    align-items: center;
    gap: 4px;
    margin: 4px 8px;
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
    padding: 4px 8px 8px;
  }
  .snippet {
    padding: 6px 8px;
    margin-bottom: 6px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg);
  }
  .top {
    display: flex;
    align-items: center;
    gap: 2px;
  }
  .top .icon {
    width: 22px;
    height: 22px;
  }
  .name {
    flex: 1;
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .cmd {
    color: var(--text-dim);
    max-height: 3.6em;
    overflow: hidden;
    white-space: pre-wrap;
  }
  .tags {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    margin-top: 4px;
  }
  .tag {
    padding: 0 6px;
    border-radius: 8px;
    background: var(--bg-active);
    font-size: 11px;
  }
  .empty {
    padding: 4px;
  }
</style>
