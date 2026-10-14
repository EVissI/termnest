<script lang="ts">
  import type { SessionState } from '../lib/ipc';

  let { state, label = true }: { state: SessionState | null | 'held'; label?: boolean } = $props();

  const labels: Record<string, string> = {
    connecting: 'Подключение',
    connected: 'Подключено',
    reconnecting: 'Переподключение',
    disconnected: 'Отключено',
    error: 'Ошибка',
    held: 'Ожидает подключения',
  };

  const text = $derived(state ? labels[state] : 'Не подключено');
</script>

<span class="status" data-state={state ?? 'idle'} title={text}>
  <svg width="9" height="9" viewBox="0 0 10 10" aria-hidden="true">
    <circle cx="5" cy="5" r="4" />
  </svg>
  {#if label}{text}{/if}
</span>

<style>
  .status {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    color: var(--text-dim);
    white-space: nowrap;
  }
  circle {
    fill: var(--text-dim);
  }
  [data-state='connected'] circle {
    fill: var(--ok);
  }
  [data-state='connecting'] circle,
  [data-state='reconnecting'] circle {
    fill: var(--warn);
  }
  [data-state='error'] circle {
    fill: var(--err);
  }
</style>
