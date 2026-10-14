<script lang="ts">
  import { api, type ConflictAction, type PendingPrompt, type PromptReply } from '../lib/ipc';
  import { store } from '../stores/app.svelte';
  import Modal from './Modal.svelte';
  import Icon from './Icon.svelte';

  // One prompt at a time, oldest first.
  const prompt = $derived<PendingPrompt | undefined>(store.prompts[0]);

  let passphrase = $state('');
  let forget = $state(false);
  let applyToAll = $state(false);

  function reply(r: PromptReply): void {
    if (!prompt) return;
    const id = prompt.id;
    store.prompts = store.prompts.filter((p) => p.id !== id);
    passphrase = '';
    applyToAll = false;
    void api.promptRespond(id, r);
  }

  function conflict(action: ConflictAction): void {
    reply({ kind: 'conflict', action, apply_to_all: applyToAll });
  }

  function submitPassphrase(e: SubmitEvent): void {
    e.preventDefault();
    reply({ kind: 'passphrase', value: passphrase, remember: !forget });
  }
</script>

{#if prompt}
  {@const r = prompt.request}
  {#if r.kind === 'host_key_unknown'}
    <Modal title="Неизвестный сервер" onclose={() => reply({ kind: 'cancel' })} width={520}>
      <p>Подключение к <b>{r.host}:{r.port}</b> выполняется впервые. Проверьте отпечаток ключа сервера:</p>
      <div class="fp">
        <div class="hint">{r.key_type}</div>
        <div class="mono">{r.fingerprint}</div>
      </div>
      <p class="hint">Если отпечаток совпадает с ожидаемым, нажмите «Доверять». Ключ будет сохранён в known_hosts termnest.</p>
      {#snippet footer()}
        <button class="btn" type="button" onclick={() => reply({ kind: 'cancel' })}>Отмена</button>
        <button class="btn primary" type="button" onclick={() => reply({ kind: 'confirm' })}>Доверять</button>
      {/snippet}
    </Modal>
  {:else if r.kind === 'passphrase'}
    <Modal title="Ключ защищён паролем" onclose={() => reply({ kind: 'cancel' })}>
      <form id="passphrase-form" onsubmit={submitPassphrase}>
        <p>Введите passphrase для ключа:</p>
        <div class="mono path">{r.key_path}</div>
        {#if r.retry}
          <p class="error-text"><Icon name="warning" size={13} /> Неверная passphrase, попробуйте ещё раз.</p>
        {/if}
        <!-- svelte-ignore a11y_autofocus -->
        <input type="password" bind:value={passphrase} autofocus autocomplete="off" aria-label="Passphrase" />
        <label class="check"><input type="checkbox" bind:checked={forget} /> Не запоминать до выхода из программы</label>
      </form>
      {#snippet footer()}
        <button class="btn" type="button" onclick={() => reply({ kind: 'cancel' })}>Отмена</button>
        <button class="btn primary" type="submit" form="passphrase-form">Разблокировать</button>
      {/snippet}
    </Modal>
  {:else if r.kind === 'file_conflict'}
    <Modal title="Файл уже существует" onclose={() => reply({ kind: 'cancel' })} width={520}>
      <p>{r.remote ? 'На сервере уже есть' : 'На компьютере уже есть'}:</p>
      <div class="mono path">{r.name}</div>
      <label class="check"><input type="checkbox" bind:checked={applyToAll} /> Применить ко всем конфликтам этой передачи</label>
      {#snippet footer()}
        <button class="btn" type="button" onclick={() => reply({ kind: 'cancel' })}>Отменить передачу</button>
        <button class="btn" type="button" onclick={() => conflict('skip')}>Пропустить</button>
        <button class="btn" type="button" onclick={() => conflict('rename')}>Переименовать</button>
        <button class="btn primary" type="button" onclick={() => conflict('replace')}>Заменить</button>
      {/snippet}
    </Modal>
  {/if}
{/if}

<style>
  .fp {
    padding: 10px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg-input);
    margin: 8px 0;
  }
  .fp .mono {
    font-size: 13px;
    margin-top: 2px;
  }
  .path {
    margin: 4px 0 10px;
    color: var(--text-dim);
  }
  input[type='password'] {
    width: 100%;
  }
  p {
    margin: 4px 0 8px;
  }
</style>
