<script lang="ts">
  import { onMount } from 'svelte';
  import { writeText } from '@tauri-apps/plugin-clipboard-manager';
  import { api, type ClaudeSettings, type McpLogEntry, type McpStatus } from '../lib/ipc';
  import { attempt } from '../lib/actions';
  import { confirmDialog, toast } from '../stores/app.svelte';

  /** "Claude integration" section of the settings dialog (SPEC section 12). */
  let { claude = $bindable() }: { claude: ClaudeSettings } = $props();

  let status = $state<McpStatus | null>(null);
  let journal = $state<McpLogEntry[]>([]);
  let showToken = $state(false);

  async function refresh(): Promise<void> {
    status = (await attempt(() => api.mcpStatus())) ?? null;
    journal = (await attempt(() => api.mcpJournal())) ?? [];
  }

  onMount(() => {
    void refresh();
  });

  // The server restarts in the background after the setting is saved.
  $effect(() => {
    void claude.mcp_enabled;
    void claude.mcp_port;
    const t = setTimeout(() => void refresh(), 900);
    return () => clearTimeout(t);
  });

  async function toggleMcp(e: Event): Promise<void> {
    const input = e.target as HTMLInputElement;
    if (input.checked && !claude.notice_accepted) {
      const ok = await confirmDialog({
        title: 'Интеграция с Claude Code',
        message:
          'Claude Code сможет читать список серверов, вывод терминалов и файлы на серверах через SFTP. ' +
          'В выводе терминала могут оказаться пароли и токены - они тоже попадут в Claude. ' +
          'Сервер слушает только 127.0.0.1 и требует токен; ключи и пароли termnest через него недоступны.',
        confirmLabel: 'Понятно, включить',
      });
      if (!ok) {
        input.checked = false;
        return;
      }
      claude.notice_accepted = true;
    }
    claude.mcp_enabled = input.checked;
  }

  async function copyCommand(): Promise<void> {
    if (!status) return;
    await writeText(status.add_command);
    toast('Команда скопирована: выполните её в терминале Windows', 'ok', 4000);
  }

  const removeCommand = 'claude mcp remove termnest -s user';

  async function copyRemove(): Promise<void> {
    await writeText(removeCommand);
    toast('Команда удаления скопирована: выполните её, затем команду подключения', 'ok', 4000);
  }

  async function regenerate(): Promise<void> {
    const ok = await confirmDialog({
      title: 'Новый токен',
      message: 'Старый токен перестанет работать: Claude Code нужно будет подключить заново новой командой.',
      confirmLabel: 'Выпустить новый',
    });
    if (ok) status = (await attempt(() => api.mcpRegenerateToken())) ?? status;
  }

  function time(t: number): string {
    return new Date(t * 1000).toLocaleTimeString();
  }

  const masked = $derived(status ? `${status.token.slice(0, 6)}...${status.token.slice(-4)}` : '');
</script>

<h4>MCP-сервер</h4>
<label class="check">
  <input type="checkbox" checked={claude.mcp_enabled} onchange={toggleMcp} />
  <span>Разрешить Claude Code читать контекст termnest (MCP)</span>
</label>
<div class="grid2">
  <label class="field">
    <span>Порт (только 127.0.0.1)</span>
    <input type="number" min="1024" max="65535" bind:value={claude.mcp_port} />
  </label>
  <div class="field">
    <span>Состояние</span>
    <div class="state">
      {#if status?.error}
        <span class="error-text">{status.error}</span>
      {:else if status?.running}
        <span class="ok">Работает</span>
      {:else}
        <span class="hint">Выключен</span>
      {/if}
      <button class="btn small-btn" type="button" onclick={refresh}>Обновить</button>
    </div>
  </div>
</div>

{#if status}
  <div class="field">
    <span>Подключение Claude Code (выполнить один раз в терминале Windows)</span>
    <div class="cmd mono">{status.add_command.replace(status.token, showToken ? status.token : masked)}</div>
    <div class="row">
      <button class="btn" type="button" onclick={copyCommand}>Скопировать команду</button>
      <button class="btn" type="button" onclick={() => (showToken = !showToken)}>{showToken ? 'Скрыть' : 'Показать'} токен</button>
      <button class="btn" type="button" onclick={regenerate}>Новый токен</button>
    </div>
  </div>
  <div class="field">
    <span>Сброс подключения (если Claude Code пишет «already exists» или ошибку авторизации после смены токена)</span>
    <div class="cmd mono">{removeCommand}</div>
    <div class="row">
      <button class="btn" type="button" onclick={copyRemove}>Скопировать команду удаления</button>
      <span class="hint">После неё выполните команду подключения выше.</span>
    </div>
  </div>
{/if}

<h4>Агенты</h4>
<div class="grid2">
  <label class="field">
    <span>Права агента в проекте</span>
    <select bind:value={claude.agent_mode}>
      <option value="edit">Править файлы проекта</option>
      <option value="propose">Только читать и предлагать правки</option>
    </select>
  </label>
  <label class="field">
    <span>Одновременно агентов</span>
    <input type="number" min="1" max="8" bind:value={claude.max_agents} />
  </label>
  <label class="field">
    <span>Строк вывода в запросе (без выделения)</span>
    <input type="number" min="20" max="2000" step="10" bind:value={claude.context_lines} />
  </label>
  <label class="field">
    <span>Путь к Claude Code CLI (пусто - из PATH)</span>
    <input
      value={claude.cli_path ?? ''}
      oninput={(e) => (claude.cli_path = (e.target as HTMLInputElement).value || null)}
      placeholder="claude"
      spellcheck="false"
    />
  </label>
</div>

<h4>Журнал вызовов MCP</h4>
{#if journal.length}
  <div class="journal mono">
    {#each journal as j, i (i)}
      <div class:fail={!j.ok}>
        {time(j.time)}&nbsp;&nbsp;{j.tool}{j.session !== null ? ` (сессия ${j.session})` : ''}{j.ok ? '' : ' - ошибка'}
      </div>
    {/each}
  </div>
{:else}
  <p class="hint">Вызовов пока не было.</p>
{/if}

<style>
  h4 {
    margin: 4px 0 8px;
    font-size: 13px;
  }
  h4:not(:first-child) {
    margin-top: 16px;
  }
  .grid2 {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 0 12px;
    align-items: end;
  }
  .state {
    display: flex;
    align-items: center;
    gap: 10px;
    min-height: 30px;
  }
  .ok {
    color: var(--ok);
  }
  .cmd {
    padding: 8px;
    margin-bottom: 6px;
    border: 1px solid var(--border);
    border-radius: 4px;
    background: var(--bg-input);
    user-select: text;
  }
  .small-btn {
    padding: 2px 8px;
    font-size: 12px;
  }
  .journal {
    max-height: 140px;
    overflow: auto;
    padding: 6px 8px;
    border: 1px solid var(--border);
    border-radius: 4px;
    background: var(--bg-input);
  }
  .fail {
    color: var(--err);
  }
</style>
