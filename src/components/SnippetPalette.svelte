<script lang="ts">
  import { onMount, tick } from 'svelte';
  import type { Snippet } from '../lib/ipc';
  import { render, toTerminalInput, variables } from '../lib/snippets';
  import { focusedPane, sendText, targetSessions, type SendTarget } from '../lib/actions';
  import { store, terminals, ui } from '../stores/app.svelte';

  let query = $state('');
  let index = $state(0);
  let chosen = $state<Snippet | null>(null);
  let values = $state<Record<string, string>>({});
  let searchInput = $state<HTMLInputElement | null>(null);
  let varsForm = $state<HTMLFormElement | null>(null);

  const targets: { id: SendTarget; label: string }[] = [
    { id: 'current', label: 'Текущая панель' },
    { id: 'selected', label: 'Выбранные' },
    { id: 'tab', label: 'Все панели вкладки' },
    { id: 'all', label: 'Все сессии' },
  ];

  const results = $derived(
    store.snippets.filter((s) => {
      const q = query.trim().toLowerCase();
      if (!q) return true;
      return s.name.toLowerCase().includes(q) || s.tags.some((t) => t.toLowerCase().includes(q));
    }),
  );
  const vars = $derived(chosen ? variables(chosen.command) : []);
  const count = $derived(targetSessions(ui.sendTarget).length);

  onMount(() => {
    const pre = store.snippets.find((s) => s.id === ui.paletteSnippet);
    ui.paletteSnippet = null;
    if (pre) void choose(pre);
    else searchInput?.focus();
  });

  function close(): void {
    ui.palette = false;
    const pane = focusedPane();
    if (pane) terminals.get(pane.id)?.focus();
  }

  async function choose(s: Snippet): Promise<void> {
    chosen = s;
    values = Object.fromEntries(variables(s.command).map((v) => [v, '']));
    await tick();
    varsForm?.querySelector('input')?.focus();
    if (variables(s.command).length === 0) varsForm?.querySelector<HTMLButtonElement>('button[type=submit]')?.focus();
  }

  async function send(e?: SubmitEvent): Promise<void> {
    e?.preventDefault();
    if (!chosen) return;
    const text = toTerminalInput(render(chosen.command, values), chosen.press_enter);
    const sessions = targetSessions(ui.sendTarget);
    close();
    await sendText(sessions, text);
  }

  function onSearchKey(e: KeyboardEvent): void {
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      index = Math.min(results.length - 1, index + 1);
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      index = Math.max(0, index - 1);
    } else if (e.key === 'Enter') {
      e.preventDefault();
      if (results[index]) void choose(results[index]);
    }
  }

  function onKey(e: KeyboardEvent): void {
    if (e.key === 'Escape') {
      e.stopPropagation();
      if (chosen && !ui.paletteSnippet) {
        chosen = null;
        void tick().then(() => searchInput?.focus());
      } else close();
    }
  }

  $effect(() => {
    // Keep the highlighted item valid while filtering.
    if (index >= results.length) index = Math.max(0, results.length - 1);
  });
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="backdrop" onmousedown={(e) => e.target === e.currentTarget && close()} onkeydown={onKey}>
  <div class="palette" role="dialog" aria-label="Палитра сниппетов">
    {#if !chosen}
      <input
        bind:this={searchInput}
        bind:value={query}
        oninput={() => (index = 0)}
        onkeydown={onSearchKey}
        placeholder="Сниппет: поиск по названию и тегам"
        aria-label="Поиск сниппета"
      />
      <div class="results">
        {#each results as s, i (s.id)}
          <button class="item" class:active={i === index} type="button" onmouseenter={() => (index = i)} onclick={() => choose(s)}>
            <span class="name">{s.name}</span>
            {#each s.tags as t (t)}<span class="tag">{t}</span>{/each}
            <span class="cmd mono">{s.command}</span>
          </button>
        {:else}
          <div class="hint none">{store.snippets.length ? 'Ничего не найдено' : 'Сниппетов пока нет: создайте их в разделе «Сниппеты»'}</div>
        {/each}
      </div>
    {:else}
      <form class="vars" bind:this={varsForm} onsubmit={send}>
        <div class="title">{chosen.name}</div>
        <pre class="mono preview">{render(chosen.command, values)}</pre>
        {#each vars as v (v)}
          <label class="field">
            <span>{v}</span>
            <input bind:value={values[v]} spellcheck="false" />
          </label>
        {/each}
        <div class="row end">
          <span class="hint grow">Получателей: {count}</span>
          <button class="btn primary" type="submit" disabled={count === 0}>Отправить</button>
        </div>
      </form>
    {/if}
    <div class="targets" role="radiogroup" aria-label="Куда отправить">
      {#each targets as t (t.id)}
        <label class="target">
          <input type="radio" name="send-target" value={t.id} bind:group={ui.sendTarget} />
          {t.label}
        </label>
      {/each}
    </div>
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 150;
    display: flex;
    justify-content: center;
    align-items: flex-start;
    padding-top: 10vh;
    background: rgb(0 0 0 / 25%);
  }
  .palette {
    width: min(620px, calc(100vw - 32px));
    max-height: 70vh;
    display: flex;
    flex-direction: column;
    background: var(--bg-panel);
    border: 1px solid var(--border);
    border-radius: 8px;
    box-shadow: 0 16px 48px rgb(0 0 0 / 45%);
    overflow: hidden;
  }
  .palette > input {
    margin: 10px;
    font-size: 14px;
    padding: 8px 10px;
  }
  .results {
    overflow: auto;
    padding: 0 6px 6px;
  }
  .item {
    display: flex;
    align-items: baseline;
    gap: 6px;
    width: 100%;
    padding: 6px 8px;
    border: none;
    border-radius: 4px;
    background: none;
    text-align: left;
    cursor: pointer;
    min-width: 0;
  }
  .item.active {
    background: var(--bg-active);
  }
  .name {
    font-weight: 600;
    white-space: nowrap;
  }
  .tag {
    padding: 0 6px;
    border-radius: 8px;
    background: var(--bg-hover);
    font-size: 11px;
    white-space: nowrap;
  }
  .cmd {
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
    flex: 1;
    text-align: right;
  }
  .none {
    padding: 10px;
  }
  .vars {
    padding: 12px 14px 4px;
    overflow: auto;
  }
  .title {
    font-weight: 600;
    margin-bottom: 6px;
  }
  .preview {
    margin: 0 0 10px;
    padding: 8px;
    max-height: 140px;
    overflow: auto;
    white-space: pre-wrap;
    background: var(--bg-input);
    border: 1px solid var(--border);
    border-radius: 4px;
  }
  .vars input {
    width: 100%;
  }
  .grow {
    flex: 1;
  }
  .end {
    margin-bottom: 8px;
  }
  .targets {
    display: flex;
    flex-wrap: wrap;
    gap: 12px;
    padding: 8px 12px;
    border-top: 1px solid var(--border);
  }
  .target {
    display: flex;
    align-items: center;
    gap: 5px;
    cursor: pointer;
  }
</style>
