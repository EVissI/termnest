<script lang="ts">
  import { onMount } from 'svelte';
  import { api, type OpenSource, type PaneId } from '../lib/ipc';
  import { attempt } from '../lib/actions';
  import { store, ui, windowLabel } from '../stores/app.svelte';
  import Icon from './Icon.svelte';

  /** Shown in an empty panel (or an empty window when `paneId` is null). */
  let { paneId = null }: { paneId?: PaneId | null } = $props();

  let filter = $state('');
  let quick = $state('');
  let input: HTMLInputElement;

  const hosts = $derived(
    store.hosts.hosts.filter((h) => {
      const q = filter.trim().toLowerCase();
      if (!q) return true;
      return [h.name, h.host, h.user, h.group ?? ''].some((v) => v.toLowerCase().includes(q));
    }),
  );

  onMount(() => input.focus());

  async function open(source: OpenSource): Promise<void> {
    if (paneId !== null) await attempt(() => api.paneOpen(paneId, source));
    else await attempt(() => api.tabNew(windowLabel, source));
  }

  function submitQuick(e: SubmitEvent): void {
    e.preventDefault();
    if (quick.trim()) void open({ kind: 'target', target: quick.trim() });
  }

  function onFilterKey(e: KeyboardEvent): void {
    if (e.key === 'Enter' && hosts.length > 0) {
      e.preventDefault();
      void open({ kind: 'host', id: hosts[0].id });
    }
  }
</script>

<div class="picker">
  <div class="box">
    <h3><Icon name="server" size={15} /> Куда подключиться?</h3>
    <input
      bind:this={input}
      bind:value={filter}
      placeholder="Поиск сервера (Enter - первый в списке)"
      onkeydown={onFilterKey}
      aria-label="Поиск сервера"
    />
    <div class="list">
      {#each hosts as h (h.id)}
        <button class="host" type="button" onclick={() => open({ kind: 'host', id: h.id })}>
          <span class="name">{h.name}</span>
          <span class="addr">{h.user}@{h.host}{h.port !== 22 ? `:${h.port}` : ''}</span>
          {#if h.group}<span class="group">{h.group}</span>{/if}
        </button>
      {:else}
        <div class="empty">
          {#if store.hosts.hosts.length === 0}
            Сохранённых серверов нет.
            <button class="link" type="button" onclick={() => (ui.hostDialog = { id: '', name: '', host: '', port: 22, user: '', note: '' })}>
              Добавить сервер
            </button>
          {:else}
            Ничего не найдено
          {/if}
        </div>
      {/each}
    </div>
    <form class="quick" onsubmit={submitQuick}>
      <input bind:value={quick} placeholder="Быстрое подключение: user@host:port" aria-label="Быстрое подключение" />
      <button class="btn primary" type="submit" disabled={!quick.trim()}>Подключить</button>
    </form>
  </div>
</div>

<style>
  .picker {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: flex-start;
    justify-content: center;
    padding: 6% 12px 12px;
    overflow: auto;
    background: var(--bg);
    z-index: 4;
  }
  .box {
    width: min(460px, 100%);
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  h3 {
    display: flex;
    align-items: center;
    gap: 8px;
    margin: 0 0 4px;
    font-size: 14px;
    font-weight: 600;
  }
  .list {
    max-height: 280px;
    overflow: auto;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg-panel);
  }
  .host {
    display: flex;
    align-items: baseline;
    gap: 8px;
    width: 100%;
    padding: 6px 10px;
    border: none;
    background: none;
    text-align: left;
    cursor: pointer;
  }
  .host:hover {
    background: var(--bg-hover);
  }
  .name {
    font-weight: 500;
  }
  .addr,
  .group {
    color: var(--text-dim);
    font-size: 12px;
  }
  .group {
    margin-left: auto;
  }
  .empty {
    padding: 12px;
    color: var(--text-dim);
  }
  .link {
    background: none;
    border: none;
    padding: 0;
    color: var(--accent);
    cursor: pointer;
  }
  .quick {
    display: flex;
    gap: 8px;
  }
  .quick input {
    flex: 1;
  }
</style>
