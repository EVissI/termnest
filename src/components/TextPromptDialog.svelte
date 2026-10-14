<script lang="ts">
  import { ui } from '../stores/app.svelte';
  import Modal from './Modal.svelte';

  const req = $derived(ui.textPrompt);
  let value = $state('');

  $effect(() => {
    if (req) value = req.value;
  });

  function submit(e: SubmitEvent): void {
    e.preventDefault();
    req?.resolve(value);
  }
</script>

{#if req}
  <Modal title={req.title} onclose={() => req.resolve(null)}>
    <form id="text-prompt" onsubmit={submit}>
      <label class="field">
        <span>{req.label}</span>
        <!-- svelte-ignore a11y_autofocus -->
        <input bind:value autofocus />
      </label>
    </form>
    {#snippet footer()}
      <button class="btn" type="button" onclick={() => req.resolve(null)}>Отмена</button>
      <button class="btn primary" type="submit" form="text-prompt">OK</button>
    {/snippet}
  </Modal>
{/if}
