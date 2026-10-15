<script lang="ts">
  import { onMount } from 'svelte';
  import { api, errorMessage, type AgentPrepared, type Pane, type PaneId } from '../lib/ipc';
  import { allPanes } from '../lib/layout/geometry';
  import { linkHost, linkPane } from '../lib/projects';
  import { confirmDialog, saveSettings, sessionCwd, store, terminals, toast, ui } from '../stores/app.svelte';
  import Modal from './Modal.svelte';
  import Icon from './Icon.svelte';

  let { paneId }: { paneId: PaneId } = $props();

  type Mode = 'fix' | 'ask';

  let prepared = $state<AgentPrepared | null>(null);
  let mode = $state<Mode | null>(null);
  let prompt = $state('');
  let title = $state('');
  let comment = $state('');
  let edited = $state(false);
  let loading = $state(true);
  let sending = $state(false);
  let error = $state('');
  let cli = $state<string | null>(null);
  let cliError = $state('');

  const pane = $derived<Pane | undefined>(
    store.layout.windows.flatMap((w) => w.tabs).flatMap((t) => allPanes(t.root)).find((p) => p.id === paneId),
  );
  // Captured once: the selection at the moment the dialog was opened.
  // svelte-ignore state_referenced_locally
  const selection = terminals.get(paneId)?.term.getSelection() ?? '';

  const project = $derived(prepared?.project ?? null);
  const ask = $derived(mode === 'ask');

  async function prepare(): Promise<void> {
    if (!pane) return;
    loading = true;
    error = '';
    try {
      const cwd = pane.session !== null ? (sessionCwd.get(pane.session) ?? null) : null;
      prepared = await api.agentPrepare(paneId, selection || null, cwd, comment, mode === 'ask');
      // Without a project only a question is possible.
      if (mode === null || (mode === 'fix' && !prepared.project)) {
        const next: Mode = prepared.project ? 'fix' : 'ask';
        if (next !== mode) {
          mode = next;
          await prepare();
          return;
        }
      }
      if (!edited) prompt = prepared.prompt;
      if (!title) title = prepared.title;
    } catch (e) {
      error = errorMessage(e);
    } finally {
      loading = false;
    }
  }

  onMount(() => {
    void prepare();
    api
      .claudeCliCheck()
      .then((v) => (cli = v))
      .catch((e) => (cliError = errorMessage(e)));
  });

  function setMode(next: Mode): void {
    if (next === mode) return;
    mode = next;
    edited = false;
    void prepare();
  }

  async function link(): Promise<void> {
    if (!pane) return;
    const hostId = pane.spec?.host_id;
    if (hostId) await linkHost(hostId);
    else await linkPane(pane);
    mode = 'fix';
    edited = false;
    await prepare();
  }

  async function send(): Promise<void> {
    if (!prepared || (!ask && !project)) return;
    if (!store.settings?.claude.notice_accepted) {
      const ok = await confirmDialog({
        title: 'Отправка в Claude',
        message:
          'Текст запроса, включая вывод терминала, уйдёт в Claude. В выводе могут быть пароли и токены - проверьте текст перед отправкой. ' +
          'Агент ничего не выполняет на сервере без вашего подтверждения.',
        confirmLabel: 'Понятно',
      });
      if (!ok) return;
      await saveSettings({ claude: { ...store.settings!.claude, notice_accepted: true } });
    }
    sending = true;
    try {
      const taskTitle = (ask && comment.trim() ? comment.trim() : title || prepared.title).slice(0, 120);
      const id = await api.agentStart(paneId, ask ? null : (project?.path ?? null), ask, taskTitle, prompt);
      toast(ask ? 'Вопрос отправлен' : 'Задача отправлена агенту', 'ok', 3000);
      ui.sendToAgent = null;
      ui.sidebarSection = 'agents';
      if (store.settings && !store.settings.sidebar_visible) await saveSettings({ sidebar_visible: true });
      ui.agentOpen = id;
    } catch (e) {
      error = errorMessage(e);
    } finally {
      sending = false;
    }
  }

  const fixLabel = $derived(
    store.settings?.claude.agent_mode === 'propose' ? 'предложит правки в коде проекта' : 'исправит код в проекте',
  );
  const canSend = $derived(
    !!prepared && !sending && !loading && !cliError && !!prompt.trim() && (ask || !!project?.exists),
  );
</script>

<Modal title="Спросить Claude" onclose={() => (ui.sendToAgent = null)} width={720}>
  {#if loading && !prepared}
    <p class="hint">Собираю контекст...</p>
  {:else if prepared}
    <div class="modes" role="radiogroup" aria-label="Режим">
      <label class="mode" class:active={mode === 'ask'}>
        <input type="radio" name="agent-mode" checked={mode === 'ask'} onchange={() => setMode('ask')} />
        <span><b>Просто спросить</b><br /><span class="hint">ответит по выводу терминала, ничего не меняя</span></span>
      </label>
      <label class="mode" class:active={mode === 'fix'} class:disabled={!project}>
        <input type="radio" name="agent-mode" checked={mode === 'fix'} disabled={!project} onchange={() => setMode('fix')} />
        <span><b>Исправить в проекте</b><br /><span class="hint">{project ? `агент ${fixLabel} «${project.name}»` : 'нужен привязанный проект'}</span></span>
      </label>
    </div>
    {#if !project}
      <p class="hint">
        Чтобы агент мог править код, привяжите проект:
        <button class="link" type="button" onclick={link}>
          {pane?.spec?.host_id ? 'привязать сервер к папке проекта' : 'привязать панель к папке проекта'}
        </button>
      </p>
    {/if}

    <div class="meta">
      <div><span class="hint">Сервер</span> {prepared.server}</div>
      {#if !ask && project}
        <div><span class="hint">Проект</span> <b>{project.name}</b> <span class="mono hint">{project.path}</span></div>
        <div>
          <span class="hint">Где</span>
          {project.is_git ? 'в отдельной ветке (worktree)' : 'прямо в папке (не git)'}
        </div>
      {/if}
      {#if prepared.mcp}<div><span class="hint">MCP</span> может дочитать контекст с сервера через termnest</div>{/if}
    </div>
    {#if !ask && project && !project.exists}<div class="err-box">Папка проекта не найдена.</div>{/if}
    {#if cliError}<div class="err-box">{cliError}</div>{:else if cli}<p class="hint">Claude Code {cli}</p>{/if}

    {#if ask}
      <label class="field">
        <span>Вопрос</span>
        <!-- svelte-ignore a11y_autofocus -->
        <input
          bind:value={comment}
          autofocus
          onchange={() => !edited && prepare()}
          placeholder="Например: почему упал сервис и как это починить?"
        />
      </label>
    {:else}
      <label class="field">
        <span>Заголовок задачи</span>
        <input bind:value={title} />
      </label>
      <label class="field">
        <span>Комментарий (добавится в запрос)</span>
        <input bind:value={comment} onchange={() => !edited && prepare()} placeholder="Например: ошибка появилась после деплоя" />
      </label>
    {/if}
    {#if prepared.secrets.length}
      <div class="warn-box">
        <b>Похоже на секреты</b> - проверьте и при необходимости уберите из текста:
        <ul class="mono">
          {#each prepared.secrets as s, i (i)}<li>{s}</li>{/each}
        </ul>
      </div>
    {/if}
    <label class="field">
      <span>Текст запроса {edited ? '(изменён вручную)' : ''}{selection ? ' - взято выделение' : ' - последние строки вывода'}</span>
      <textarea class="mono" rows="12" bind:value={prompt} oninput={() => (edited = true)} spellcheck="false"></textarea>
    </label>
  {/if}
  {#if error}<div class="err-box">{error}</div>{/if}

  {#snippet footer()}
    <button class="btn" type="button" onclick={() => (ui.sendToAgent = null)}>Отмена</button>
    <button class="btn primary" type="button" onclick={send} disabled={!canSend}>
      <Icon name="agent" size={14} />
      {sending ? 'Отправка...' : ask ? 'Спросить' : 'Отправить агенту'}
    </button>
  {/snippet}
</Modal>

<style>
  .modes {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 8px;
    margin-bottom: 8px;
  }
  .mode {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    padding: 8px 10px;
    border: 1px solid var(--border);
    border-radius: 6px;
    cursor: pointer;
  }
  .mode.active {
    border-color: var(--accent);
    background: var(--bg-active);
  }
  .mode.disabled {
    opacity: 0.55;
    cursor: default;
  }
  .mode input {
    margin-top: 3px;
  }
  .link {
    border: none;
    background: none;
    padding: 0;
    color: var(--accent);
    cursor: pointer;
  }
  .meta {
    display: grid;
    gap: 3px;
    margin-bottom: 10px;
    padding: 8px 10px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg-input);
  }
  .meta .hint {
    display: inline-block;
    min-width: 60px;
  }
  input:not([type='radio']),
  textarea {
    width: 100%;
  }
  textarea {
    resize: vertical;
  }
  ul {
    margin: 4px 0 0;
    padding-left: 18px;
  }
</style>
