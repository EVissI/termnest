<script lang="ts">
  import { ui } from '../stores/app.svelte';
  import Modal from './Modal.svelte';

  const req = $derived(ui.confirm);
</script>

{#if req}
  <Modal title={req.title} onclose={() => req.resolve(false)}>
    <p class="msg">{req.message}</p>
    {#if req.detail}
      <pre class="detail mono">{req.detail}</pre>
    {/if}
    {#snippet footer()}
      <button class="btn" type="button" onclick={() => req.resolve(false)}>Отмена</button>
      <!-- svelte-ignore a11y_autofocus -->
      <button class="btn {req.danger ? 'danger' : 'primary'}" type="button" autofocus onclick={() => req.resolve(true)}>
        {req.confirmLabel ?? 'OK'}
      </button>
    {/snippet}
  </Modal>
{/if}

<style>
  .msg {
    margin: 4px 0 8px;
    white-space: pre-wrap;
  }
  .detail {
    max-height: 200px;
    overflow: auto;
    margin: 0;
    padding: 8px;
    background: var(--bg-input);
    border: 1px solid var(--border);
    border-radius: 4px;
    white-space: pre-wrap;
  }
</style>
