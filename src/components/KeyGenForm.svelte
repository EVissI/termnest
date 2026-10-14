<script lang="ts">
  import { api, errorMessage, type KeyInfo } from '../lib/ipc';
  import { confirmDialog, refreshKeys } from '../stores/app.svelte';

  /** Key generation form, used in its own dialog and inside the deploy wizard. */
  let { ondone, oncancel }: { ondone: (key: KeyInfo) => void; oncancel?: () => void } = $props();

  let name = $state('id_ed25519');
  let kind = $state<'ed25519' | 'rsa4096'>('ed25519');
  let comment = $state('termnest');
  let passphrase = $state('');
  let passphrase2 = $state('');
  let busy = $state(false);
  let error = $state('');

  $effect(() => {
    // Suggest the conventional file name for the chosen type.
    if (kind === 'rsa4096' && name === 'id_ed25519') name = 'id_rsa';
    if (kind === 'ed25519' && name === 'id_rsa') name = 'id_ed25519';
  });

  async function generate(overwrite: boolean): Promise<void> {
    if (passphrase !== passphrase2) {
      error = 'Passphrase и подтверждение не совпадают';
      return;
    }
    busy = true;
    error = '';
    try {
      const key = await api.keyGenerate(name.trim(), kind, comment.trim(), passphrase || null, overwrite);
      passphrase = '';
      passphrase2 = '';
      await refreshKeys();
      ondone(key);
    } catch (e) {
      const err = e as { kind?: string };
      if (err.kind === 'already_exists' && !overwrite) {
        const ok = await confirmDialog({
          title: 'Файл уже существует',
          message: `Ключ ~/.ssh/${name} уже есть. Перезаписать его? Старый ключ будет потерян, доступ к серверам с ним пропадёт.`,
          confirmLabel: 'Перезаписать',
          danger: true,
        });
        busy = false;
        if (ok) await generate(true);
        return;
      }
      error = errorMessage(e);
    } finally {
      busy = false;
    }
  }

  function submit(e: SubmitEvent): void {
    e.preventDefault();
    void generate(false);
  }
</script>

<form class="gen" onsubmit={submit}>
  <div class="row">
    <label class="field grow">
      <span>Тип</span>
      <select bind:value={kind}>
        <option value="ed25519">Ed25519 (рекомендуется)</option>
        <option value="rsa4096">RSA 4096</option>
      </select>
    </label>
    <label class="field grow">
      <span>Имя файла в .ssh</span>
      <input bind:value={name} required spellcheck="false" />
    </label>
  </div>
  <label class="field">
    <span>Комментарий</span>
    <input bind:value={comment} spellcheck="false" />
  </label>
  <div class="row">
    <label class="field grow">
      <span>Passphrase (необязательно)</span>
      <input type="password" bind:value={passphrase} autocomplete="new-password" />
    </label>
    <label class="field grow">
      <span>Повтор passphrase</span>
      <input type="password" bind:value={passphrase2} autocomplete="new-password" />
    </label>
  </div>
  <p class="hint">Пара сохраняется в %USERPROFILE%\.ssh в формате OpenSSH, доступ к приватному файлу - только у текущего пользователя.</p>
  {#if error}<p class="error-text">{error}</p>{/if}
  <div class="row end">
    {#if oncancel}<button class="btn" type="button" onclick={oncancel}>Отмена</button>{/if}
    <button class="btn primary" type="submit" disabled={busy || !name.trim()}>{busy ? 'Генерация...' : 'Сгенерировать'}</button>
  </div>
</form>

<style>
  .grow {
    flex: 1;
    min-width: 0;
  }
  input,
  select {
    width: 100%;
  }
  .end {
    justify-content: flex-end;
  }
</style>
