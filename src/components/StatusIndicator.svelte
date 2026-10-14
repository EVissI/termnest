<script lang="ts">
  import type { SessionState } from '../lib/ipc';

  let { state }: { state: SessionState | null } = $props();

  const labels: Record<SessionState, string> = {
    connecting: 'Подключение',
    connected: 'Подключено',
    reconnecting: 'Переподключение',
    disconnected: 'Отключено',
    error: 'Ошибка',
  };

  const label = $derived(state ? labels[state] : 'Не подключено');
</script>

<span class="status" data-state={state ?? 'idle'} title={label}>
  <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true">
    <circle cx="5" cy="5" r="4" />
  </svg>
  {label}
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
