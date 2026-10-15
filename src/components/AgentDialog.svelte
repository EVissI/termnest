<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { api, errorMessage, type AgentEvent, type AgentTask, type SessionId } from '../lib/ipc';
  import { openUrl } from '@tauri-apps/plugin-opener';
  import { renderMarkdown } from '../lib/markdown';
  import { allPanes } from '../lib/layout/geometry';
  import { attempt, focusedPane, sendText } from '../lib/actions';
  import { render, toTerminalInput, variables } from '../lib/snippets';
  import { confirmDialog, onAgentEvent, store, ui } from '../stores/app.svelte';
  import Modal from './Modal.svelte';
  import Icon from './Icon.svelte';

  let { id }: { id: string } = $props();

  let task = $state<AgentTask | null>(null);
  let events = $state<AgentEvent[]>([]);
  let followUp = $state('');
  let diff = $state<string | null>(null);
  let error = $state('');
  let deploying = $state(false);
  let deploySnippet = $state('');
  let log: HTMLDivElement;

  const summary = $derived(store.agents[id]);
  const running = $derived(summary?.state === 'running' || summary?.state === 'queued');

  // Claude Code sends its last message and then the same text again as the result:
  // show it once, as the result.
  const shown = $derived(
    events.filter((ev, i) => {
      const next = events[i + 1];
      return !(ev.kind === 'text' && next && (next.kind === 'result' || next.kind === 'error') && next.text.trim() === ev.text.trim());
    }),
  );

  /** Links in agent messages open in the default browser, not in the app window. */
  function openLink(e: MouseEvent): void {
    const a = (e.target as HTMLElement).closest('a');
    if (!a) return;
    e.preventDefault();
    const href = a.getAttribute('href') ?? '';
    if (/^https?:\/\//i.test(href)) void openUrl(href).catch(() => {});
  }

  async function scrollDown(): Promise<void> {
    await tick();
    if (log) log.scrollTop = log.scrollHeight;
  }

  onMount(() => {
    const off = onAgentEvent((taskId, ev) => {
      if (taskId === id) {
        events.push(ev);
        void scrollDown();
      }
    });
    api
      .agentGet(id)
      .then((t) => {
        task = t;
        events = t.events;
        void scrollDown();
      })
      .catch((e) => (error = errorMessage(e)));
    return off;
  });

  async function sendFollowUp(e: SubmitEvent): Promise<void> {
    e.preventDefault();
    const text = followUp.trim();
    if (!text) return;
    followUp = '';
    await attempt(() => api.agentFollowUp(id, text));
  }

  async function showDiff(): Promise<void> {
    diff = diff === null ? ((await attempt(() => api.agentDiff(id))) ?? null) : null;
  }

  async function removeWorktree(): Promise<void> {
    const ok = await confirmDialog({
      title: 'Удалить worktree',
      message: `Папка ${summary?.worktree} будет удалена. Ветка ${summary?.branch} останется в репозитории.`,
      confirmLabel: 'Удалить',
      danger: true,
    });
    if (ok) await attempt(() => api.agentRemoveWorktree(id));
  }

  async function deleteTask(): Promise<void> {
    const ok = await confirmDialog({
      title: 'Удалить задачу',
      message: 'Удалить задачу и её журнал из termnest? Изменения в проекте и ветка не трогаются.',
      confirmLabel: 'Удалить',
      danger: true,
    });
    if (!ok) return;
    if ((await attempt(() => api.agentDelete(id))) !== undefined) {
      delete store.agents[id];
      ui.agentOpen = null;
    }
  }

  /** Session to deploy to: the panel the error came from if it is still open, else the focused one. */
  function deployTarget(): { session: SessionId; label: string } | null {
    const panes = store.layout.windows.flatMap((w) => w.tabs).flatMap((t) => allPanes(t.root));
    const origin = panes.find((p) => p.session !== null && p.session === summary?.session);
    const p = origin ?? focusedPane();
    if (!p || p.session === null) return null;
    return { session: p.session, label: p.spec?.label ?? 'текущая панель' };
  }

  async function deploy(): Promise<void> {
    const snippet = store.snippets.find((s) => s.id === deploySnippet);
    const target = deployTarget();
    if (!snippet || !target) return;
    if (variables(snippet.command).length) {
      error = 'Сниппет с переменными нельзя запустить отсюда: используйте палитру (Ctrl+Shift+P).';
      return;
    }
    const ok = await confirmDialog({
      title: 'Выложить',
      message: `Выполнить сниппет «${snippet.name}» в панели «${target.label}»?`,
      detail: snippet.command,
      confirmLabel: 'Выполнить',
      danger: true,
    });
    if (!ok) return;
    deploying = false;
    await sendText([target.session], toTerminalInput(render(snippet.command, {}), snippet.press_enter));
  }

  const labels: Record<string, string> = {
    prompt: 'Запрос',
    text: 'Агент',
    tool: 'Действие',
    tool_error: 'Ошибка действия',
    result: 'Итог',
    error: 'Ошибка',
    info: 'termnest',
  };
</script>

<Modal title={summary?.title ?? 'Задача агента'} onclose={() => (ui.agentOpen = null)} width={820}>
  {#if summary}
    <div class="meta">
      <span class="state {summary.state}">{summary.state === 'running' ? 'работает' : summary.state === 'queued' ? 'в очереди' : summary.state === 'done' ? 'готово' : summary.state === 'failed' ? 'ошибка' : 'остановлен'}</span>
      {#if summary.ask}<span class="hint">вопрос, без проекта</span>{:else}<span class="mono hint" title={summary.cwd}>{summary.cwd}</span>{/if}
      {#if summary.branch}<span class="hint">ветка <span class="mono">{summary.branch}</span></span>{/if}
      {#if summary.cost_usd}<span class="hint">${summary.cost_usd.toFixed(3)}</span>{/if}
    </div>
  {/if}

  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div class="log" bind:this={log} onclick={openLink}>
    {#each shown as ev, i (i)}
      <div class="ev {ev.kind}">
        <div class="kind">{labels[ev.kind] ?? ev.kind}</div>
        {#if ev.kind === 'text' || ev.kind === 'result'}
          <!-- renderMarkdown() sanitizes the HTML with DOMPurify. -->
          <!-- eslint-disable-next-line svelte/no-at-html-tags -->
          <div class="text md">{@html renderMarkdown(ev.text)}</div>
        {:else}
          <div class="text">{ev.text}</div>
        {/if}
      </div>
    {:else}
      <p class="hint">{task ? 'Событий пока нет.' : 'Загрузка...'}</p>
    {/each}
    {#if running}<p class="hint working">Агент работает...</p>{/if}
  </div>

  {#if summary?.files.length}
    <div class="files">
      <span class="hint">Изменённые файлы:</span>
      {#each summary.files as f (f)}<span class="mono file">{f}</span>{/each}
    </div>
  {/if}
  {#if summary?.error && !running}<div class="err-box">{summary.error}</div>{/if}
  {#if error}<div class="err-box">{error}</div>{/if}

  {#if diff !== null}
    <pre class="diff mono">{diff}</pre>
  {/if}

  {#if deploying}
    <div class="deploy row">
      <select bind:value={deploySnippet} class="grow" aria-label="Сниппет для выкладки">
        <option value="">Выберите сниппет выкладки</option>
        {#each store.snippets as s (s.id)}<option value={s.id}>{s.name}</option>{/each}
      </select>
      <button class="btn danger" type="button" disabled={!deploySnippet} onclick={deploy}>Выполнить...</button>
      <button class="btn" type="button" onclick={() => (deploying = false)}>Отмена</button>
    </div>
  {/if}

  {#if summary?.can_continue && !running}
    <form class="follow row" onsubmit={sendFollowUp}>
      <input class="grow" bind:value={followUp} placeholder="Уточнение для агента (продолжит тот же разговор)" />
      <button class="btn primary" type="submit" disabled={!followUp.trim()}>Отправить</button>
    </form>
  {/if}

  {#snippet footer()}
    {#if running}
      <button class="btn danger" type="button" onclick={() => api.agentStop(id)}><Icon name="stop" size={13} /> Остановить</button>
    {/if}
    {#if !summary?.ask}
      <button class="btn" type="button" onclick={() => attempt(() => api.agentOpenFolder(id))}>Открыть папку</button>
      <button class="btn" type="button" onclick={showDiff}>{diff === null ? 'Показать diff' : 'Скрыть diff'}</button>
    {/if}
    {#if !running && summary?.state === 'done' && !summary?.ask}
      <button class="btn" type="button" onclick={() => (deploying = !deploying)}>Выложить...</button>
    {/if}
    {#if summary?.worktree && !running}
      <button class="btn" type="button" onclick={removeWorktree}>Удалить worktree</button>
    {/if}
    {#if !running}
      <button class="btn" type="button" onclick={deleteTask}>Удалить задачу</button>
    {/if}
    <button class="btn primary" type="button" onclick={() => (ui.agentOpen = null)}>Закрыть</button>
  {/snippet}
</Modal>

<style>
  .meta {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: 6px 14px;
    margin-bottom: 8px;
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
  .log {
    height: min(380px, 45vh);
    overflow: auto;
    padding: 6px 8px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg-input);
  }
  .ev {
    display: grid;
    grid-template-columns: 110px 1fr;
    gap: 8px;
    padding: 4px 0;
    border-bottom: 1px solid color-mix(in srgb, var(--border) 60%, transparent);
  }
  .kind {
    color: var(--text-dim);
    font-size: 11px;
    padding-top: 1px;
  }
  .text {
    white-space: pre-wrap;
    word-break: break-word;
    user-select: text;
  }
  .md :global(p) {
    margin: 0 0 6px;
  }
  .md :global(p:last-child) {
    margin-bottom: 0;
  }
  .md :global(h1),
  .md :global(h2),
  .md :global(h3),
  .md :global(h4) {
    margin: 8px 0 4px;
    font-size: 14px;
  }
  .md :global(h1) {
    font-size: 16px;
  }
  .md :global(ul),
  .md :global(ol) {
    margin: 2px 0 6px;
    padding-left: 22px;
  }
  .md :global(li) {
    margin: 1px 0;
  }
  .md :global(code) {
    font-family: 'Cascadia Mono', Consolas, monospace;
    font-size: 12px;
    padding: 1px 4px;
    border-radius: 3px;
    background: var(--bg-hover);
  }
  .md :global(pre) {
    margin: 4px 0 8px;
    padding: 8px 10px;
    overflow: auto;
    border: 1px solid var(--border);
    border-radius: 4px;
    background: var(--bg);
  }
  .md :global(pre code) {
    padding: 0;
    background: none;
    white-space: pre;
  }
  .md :global(blockquote) {
    margin: 4px 0;
    padding-left: 10px;
    border-left: 3px solid var(--border);
    color: var(--text-dim);
  }
  .md :global(table) {
    border-collapse: collapse;
    margin: 4px 0 8px;
  }
  .md :global(th),
  .md :global(td) {
    padding: 3px 8px;
    border: 1px solid var(--border);
    text-align: left;
  }
  .md :global(a) {
    color: var(--accent);
  }
  .text.md {
    white-space: normal;
  }
  .ev.result .text.md {
    color: var(--text);
  }
  .ev.tool .text {
    font-family: 'Cascadia Mono', Consolas, monospace;
    font-size: 12px;
    color: var(--text-dim);
  }
  .ev.prompt .text {
    max-height: 120px;
    overflow: auto;
    font-family: 'Cascadia Mono', Consolas, monospace;
    font-size: 12px;
  }
  .ev.result .kind,
  .ev.result .text {
    color: var(--ok);
  }
  .ev.error .text,
  .ev.tool_error .text {
    color: var(--err);
  }
  .working {
    margin: 6px 0 0;
  }
  .files {
    display: flex;
    flex-wrap: wrap;
    gap: 4px 8px;
    margin-top: 8px;
  }
  .file {
    padding: 0 6px;
    border-radius: 8px;
    background: var(--bg-active);
  }
  .diff {
    max-height: 260px;
    overflow: auto;
    margin: 8px 0 0;
    padding: 8px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg);
    white-space: pre;
    user-select: text;
  }
  .follow,
  .deploy {
    margin-top: 8px;
  }
  .grow {
    flex: 1;
    min-width: 0;
  }
</style>
