<script lang="ts">
  import { api, errorMessage, type Snippet } from '../lib/ipc';
  import { ui } from '../stores/app.svelte';
  import { variables } from '../lib/snippets';
  import Modal from './Modal.svelte';

  let { snippet }: { snippet: Snippet } = $props();

  // svelte-ignore state_referenced_locally
  let form = $state({ ...snippet, tagsText: snippet.tags.join(', ') });
  let error = $state('');

  const vars = $derived(variables(form.command));

  async function save(e: SubmitEvent): Promise<void> {
    e.preventDefault();
    try {
      await api.snippetUpsert({
        id: form.id,
        name: form.name,
        command: form.command,
        tags: form.tagsText.split(',').map((t) => t.trim()).filter(Boolean),
        press_enter: form.press_enter,
      });
      ui.snippetEdit = null;
    } catch (err) {
      error = errorMessage(err);
    }
  }
</script>

<Modal title={snippet.id ? 'Редактировать сниппет' : 'Новый сниппет'} onclose={() => (ui.snippetEdit = null)} width={540}>
  <form id="snippet-form" onsubmit={save}>
    <label class="field">
      <span>Название</span>
      <!-- svelte-ignore a11y_autofocus -->
      <input bind:value={form.name} required autofocus />
    </label>
    <label class="field">
      <span>Команда</span>
      <textarea class="mono" bind:value={form.command} rows="5" required spellcheck="false"></textarea>
    </label>
    {#if vars.length}
      <p class="hint">Переменные: {vars.join(', ')} - значения спросят при отправке.</p>
    {:else}
      <p class="hint">Используйте {'{{имя}}'} для переменных.</p>
    {/if}
    <label class="field">
      <span>Теги (через запятую)</span>
      <input bind:value={form.tagsText} placeholder="docker, logs" />
    </label>
    <label class="check"><input type="checkbox" bind:checked={form.press_enter} /> Нажимать Enter после команды</label>
    {#if error}<p class="error-text">{error}</p>{/if}
  </form>
  {#snippet footer()}
    <button class="btn" type="button" onclick={() => (ui.snippetEdit = null)}>Отмена</button>
    <button class="btn primary" type="submit" form="snippet-form">Сохранить</button>
  {/snippet}
</Modal>

<style>
  input:not([type='checkbox']),
  textarea {
    width: 100%;
  }
  textarea {
    resize: vertical;
  }
</style>
