<script lang="ts">
  import { onMount } from 'svelte';
  import { api, errorMessage, type AgentPrepared, type Pane, type PaneId } from '../lib/ipc';
  import { allPanes } from '../lib/layout/geometry';
  import { linkHost, linkPane } from '../lib/projects';
  import { confirmDialog, saveSettings, sessionCwd, store, terminals, toast, ui } from '../stores/app.svelte';
  import Modal from './Modal.svelte';
  import Icon from './Icon.svelte';

  let { paneId }: { paneId: PaneId } = $props();

  let prepared = $state<AgentPrepared | null>(null);
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

  async function prepare(): Promise<void> {
    if (!pane) return;
    loading = true;
    error = '';
    try {
      const cwd = pane.session !== null ? (sessionCwd.get(pane.session) ?? null) : null;
      prepared = await api.agentPrepare(paneId, selection || null, cwd, comment);
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

  async function link(): Promise<void> {
    if (!pane) return;
    const hostId = pane.spec?.host_id;
    if (hostId) await linkHost(hostId);
    else await linkPane(pane);
    await prepare();
  }

  async function send(): Promise<void> {
    if (!prepared?.project) return;
    if (!store.settings?.claude.notice_accepted) {
      const ok = await confirmDialog({
        title: 'Отправка в Claude',
        message:
          'Текст запроса, включая вывод терминала, уйдёт в Claude. В выводе могут быть пароли и токены - проверьте текст перед отправкой. ' +
          'Агент работает с файлами проекта на этом компьютере и ничего не выполняет на сервере.',
        confirmLabel: 'Понятно',
      });
      if (!ok) return;
      await saveSettings({ claude: { ...store.settings!.claude, notice_accepted: true } });
    }
    sending = true;
    try {
      const id = await api.agentStart(paneId, prepared.project.path, title || prepared.title, prompt);
      toast('Задача отправлена агенту', 'ok', 3000);
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

  const modeLabel = $derived(
    store.settings?.claude.agent_mode === 'propose' ? 'только предложит правки' : 'может править файлы проекта',
  );
</script>

<Modal title="Отправить на фикс в Claude" onclose={() => (ui.sendToAgent = null)} width={720}>
  {#if loading && !prepared}
    <p class="hint">Собираю контекст...</p>
  {:else if prepared && !prepared.project}
    <div class="warn-box">
      У панели «{prepared.server}» нет привязанного проекта. Агенту нужна папка с кодом, в которой он будет искать и исправлять
      ошибку.
    </div>
    <button class="btn primary" type="button" onclick={link}>
      <Icon name="folder" size={14} /> {pane?.spec?.host_id ? 'Привязать сервер к проекту...' : 'Привязать панель к проекту...'}
    </button>
  {:else if prepared?.project}
    {@const p = prepared.project}
    <div class="meta">
      <div><span class="hint">Проект</span> <b>{p.name}</b> <span class="mono hint">{p.path}</span></div>
      <div><span class="hint">Сервер</span> {prepared.server}</div>
      <div>
        <span class="hint">Агент</span>
        {modeLabel}{p.is_git ? ', работает в отдельной ветке (worktree)' : ', работает прямо в папке (не git)'}{prepared.mcp
          ? ', может дочитать контекст через MCP termnest'
          : ''}
      </div>
    </div>
    {#if !p.exists}<div class="err-box">Папка проекта не найдена.</div>{/if}
    {#if cliError}<div class="err-box">{cliError}</div>{:else if cli}<p class="hint">Claude Code {cli}</p>{/if}

    <label class="field">
      <span>Заголовок задачи</span>
      <input bind:value={title} />
    </label>
    <label class="field">
      <span>Комментарий (добавится в запрос)</span>
      <input bind:value={comment} onchange={() => !edited && prepare()} placeholder="Например: ошибка появилась после деплоя" />
    </label>
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
      <textarea class="mono" rows="14" bind:value={prompt} oninput={() => (edited = true)} spellcheck="false"></textarea>
    </label>
  {/if}
  {#if error}<div class="err-box">{error}</div>{/if}

  {#snippet footer()}
    <button class="btn" type="button" onclick={() => (ui.sendToAgent = null)}>Отмена</button>
    <button
      class="btn primary"
      type="button"
      onclick={send}
      disabled={sending || loading || !prepared?.project?.exists || !!cliError || !prompt.trim()}
    >
      <Icon name="agent" size={14} />
      {sending ? 'Отправка...' : 'Отправить агенту'}
    </button>
  {/snippet}
</Modal>

<style>
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
  input,
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
