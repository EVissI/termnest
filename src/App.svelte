<script lang="ts">
  import { onMount } from 'svelte';
  import TerminalPane from './components/TerminalPane.svelte';
  import StatusIndicator from './components/StatusIndicator.svelte';
  import {
    errorMessage,
    sessionApi,
    type SessionId,
    type SessionState,
    type SessionStateEvent,
  } from './lib/ipc';

  let target = $state('');
  let keyPath = $state('');
  let sessionId = $state<SessionId | null>(null);
  let sessionState = $state<SessionState | null>(null);
  let errorText = $state('');
  let busy = $state(false);
  let pane: ReturnType<typeof TerminalPane>;

  // State events can arrive before `session_open` returns the id, so keep the latest per id.
  // Plain Map on purpose: it is a lookup table, nothing renders from it.
  // eslint-disable-next-line svelte/prefer-svelte-reactivity
  const latestEvents = new Map<SessionId, SessionStateEvent>();

  const active = $derived(sessionState === 'connecting' || sessionState === 'connected');

  onMount(() => {
    let unlisten: (() => void) | null = null;
    let disposed = false;
    sessionApi
      .onState((event) => {
        latestEvents.set(event.id, event);
        applyState(event);
      })
      .then((fn) => {
        if (disposed) fn();
        else unlisten = fn;
      });
    sessionApi.defaultKeyPath().then((path) => {
      if (path && !keyPath) keyPath = path;
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  });

  function applyState(event: SessionStateEvent): void {
    if (event.id !== sessionId) return;
    sessionState = event.state;
    errorText = event.error?.message ?? '';

    if (event.state === 'connected') {
      pane.focus();
    } else if (event.state === 'error' || event.state === 'disconnected') {
      const color = event.state === 'error' ? '31' : '90';
      const text = errorText || 'Сессия завершена';
      pane.print(`\r\n\x1b[${color}m[${text}]\x1b[0m\r\n`);
      sessionApi.close(event.id).catch(() => {});
      latestEvents.delete(event.id);
      sessionId = null;
    }
  }

  async function connect(e: SubmitEvent): Promise<void> {
    e.preventDefault();
    if (busy) return;
    busy = true;
    try {
      if (sessionId !== null) {
        const old = sessionId;
        sessionId = null;
        await sessionApi.close(old);
      }
      pane.reset();
      errorText = '';
      sessionState = 'connecting';
      pane.print(`\x1b[90mПодключение к ${target.trim()}...\x1b[0m\r\n`);

      const { cols, rows } = pane.size();
      const id = await sessionApi.open(target, keyPath, cols, rows);
      sessionId = id;
      const early = latestEvents.get(id);
      if (early) applyState(early);
    } catch (err) {
      sessionState = 'error';
      errorText = errorMessage(err);
      pane.print(`\x1b[31m[${errorText}]\x1b[0m\r\n`);
    } finally {
      busy = false;
    }
  }

  function disconnect(): void {
    if (sessionId !== null) sessionApi.close(sessionId).catch(() => {});
  }
</script>

<div class="app">
  <form class="toolbar" onsubmit={connect}>
    <input
      class="target"
      bind:value={target}
      placeholder="user@host:port"
      spellcheck="false"
      autocomplete="off"
      aria-label="Адрес сервера"
    />
    <input
      class="key"
      bind:value={keyPath}
      placeholder="Путь к приватному ключу"
      spellcheck="false"
      autocomplete="off"
      aria-label="Путь к приватному ключу"
    />
    {#if active}
      <button type="button" onclick={disconnect}>Отключить</button>
    {:else}
      <button type="submit" class="primary" disabled={busy || !target.trim()}>Подключить</button>
    {/if}
    <StatusIndicator state={sessionState} />
    {#if errorText}
      <span class="error" title={errorText}>{errorText}</span>
    {/if}
  </form>

  <main class="terminal-area">
    <TerminalPane bind:this={pane} {sessionId} />
  </main>
</div>

<style>
  .app {
    display: flex;
    flex-direction: column;
    height: 100%;
  }

  .toolbar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 10px;
    background: var(--bg-panel);
    border-bottom: 1px solid var(--border);
    min-width: 0;
  }

  input {
    background: var(--bg-input);
    border: 1px solid var(--border);
    border-radius: 4px;
    padding: 5px 8px;
    outline: none;
    min-width: 0;
  }
  input:focus {
    border-color: var(--accent);
  }
  .target {
    flex: 0 1 240px;
  }
  .key {
    flex: 1 1 320px;
  }

  button {
    background: var(--bg-input);
    border: 1px solid var(--border);
    border-radius: 4px;
    padding: 5px 14px;
    cursor: pointer;
  }
  button.primary {
    background: var(--accent);
    border-color: var(--accent);
    color: #fff;
  }
  button:disabled {
    opacity: 0.5;
    cursor: default;
  }

  .error {
    color: var(--err);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }

  .terminal-area {
    flex: 1;
    min-height: 0;
    padding: 4px 0 0 6px;
    background: #14161b;
  }
</style>
