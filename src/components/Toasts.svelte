<script lang="ts">
  import { ui } from '../stores/app.svelte';
  import Icon from './Icon.svelte';

  function dismiss(id: number): void {
    const i = ui.toasts.findIndex((t) => t.id === id);
    if (i >= 0) ui.toasts.splice(i, 1);
  }
</script>

<div class="toasts" aria-live="polite">
  {#each ui.toasts as t (t.id)}
    <div class="toast {t.kind}">
      <Icon name={t.kind === 'error' ? 'warning' : t.kind === 'ok' ? 'check' : 'more'} size={14} />
      <span>{t.text}</span>
      <button class="icon" type="button" onclick={() => dismiss(t.id)} title="Скрыть"><Icon name="x" size={12} /></button>
    </div>
  {/each}
</div>

<style>
  .toasts {
    position: fixed;
    right: 12px;
    bottom: 12px;
    display: flex;
    flex-direction: column;
    gap: 6px;
    z-index: 300;
    max-width: 420px;
  }
  .toast {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    padding: 8px 6px 8px 10px;
    border-radius: 6px;
    background: var(--bg-panel);
    border: 1px solid var(--border);
    box-shadow: 0 6px 20px rgb(0 0 0 / 30%);
  }
  .toast span {
    flex: 1;
    padding-top: 1px;
    white-space: pre-wrap;
  }
  .error {
    border-color: color-mix(in srgb, var(--err) 60%, var(--border));
  }
  .error :global(svg:first-child) {
    color: var(--err);
  }
  .ok :global(svg:first-child) {
    color: var(--ok);
  }
</style>
