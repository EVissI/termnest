<script lang="ts">
  import type { AgentState } from '../lib/ipc';
  import { focusedPane } from '../lib/actions';
  import { store, ui } from '../stores/app.svelte';
  import Icon from './Icon.svelte';

  const tasks = $derived(Object.values(store.agents).sort((a, b) => b.created - a.created));

  const stateLabels: Record<AgentState, string> = {
    queued: 'в очереди',
    running: 'работает',
    done: 'готово',
    failed: 'ошибка',
    stopped: 'остановлен',
  };

  function folder(path: string): string {
    const parts = path.split(/[\\/]/).filter(Boolean);
    return parts[parts.length - 1] ?? path;
  }

  function when(t: number): string {
    return new Date(t * 1000).toLocaleString([], { day: '2-digit', month: '2-digit', hour: '2-digit', minute: '2-digit' });
  }
</script>

<div class="head">
  <h3>Агенты</h3>
  <button
    class="icon"
    type="button"
    title="Отправить ошибку из текущей панели (Ctrl+Shift+A)"
    disabled={!focusedPane()?.spec}
    onclick={() => {
      const p = focusedPane();
      if (p?.spec) ui.sendToAgent = p.id;
    }}
  >
    <Icon name="plus" />
  </button>
</div>

<div class="list">
  {#each tasks as t (t.id)}
    <button class="task" type="button" onclick={() => (ui.agentOpen = t.id)}>
      <div class="top">
        <span class="state {t.state}">{stateLabels[t.state]}</span>
        <span class="when">{when(t.created)}</span>
      </div>
      <div class="title">{t.title}</div>
      <div class="hint">{folder(t.project)}{t.server ? ` - ${t.server}` : ''}{t.files.length ? ` - файлов: ${t.files.length}` : ''}</div>
    </button>
  {:else}
    <p class="hint empty">
      Задач пока нет. Выделите ошибку в терминале и нажмите Ctrl+Shift+A (или кнопку со звёздочкой в шапке панели), чтобы отправить её
      агенту Claude Code в папке привязанного проекта.
    </p>
  {/each}
</div>

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
  .list {
    flex: 1;
    overflow: auto;
    padding: 4px 8px 8px;
  }
  .task {
    display: block;
    width: 100%;
    padding: 7px 9px;
    margin-bottom: 6px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg);
    text-align: left;
    cursor: pointer;
  }
  .task:hover {
    border-color: var(--accent);
  }
  .top {
    display: flex;
    justify-content: space-between;
    font-size: 11px;
  }
  .when {
    color: var(--text-dim);
  }
  .state {
    font-weight: 600;
  }
  .state.running,
  .state.queued {
    color: var(--warn);
  }
  .state.done {
    color: var(--ok);
  }
  .state.failed {
    color: var(--err);
  }
  .state.stopped {
    color: var(--text-dim);
  }
  .title {
    margin: 2px 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .empty {
    padding: 4px;
  }
</style>
