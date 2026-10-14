<script lang="ts">
  import { onMount } from 'svelte';
  import { open } from '@tauri-apps/plugin-dialog';
  import { writeText } from '@tauri-apps/plugin-clipboard-manager';
  import { api, type KeyInfo } from '../lib/ipc';
  import { attempt } from '../lib/actions';
  import { refreshKeys, store, toast, ui } from '../stores/app.svelte';
  import Icon from './Icon.svelte';

  onMount(() => {
    void refreshKeys();
  });

  async function copyPublic(k: KeyInfo): Promise<void> {
    const line = await attempt(() => api.keyPublic(k.path));
    if (line) {
      await writeText(line);
      toast('Публичный ключ скопирован в буфер', 'ok', 2500);
    }
  }

  async function addFile(): Promise<void> {
    const path = await open({ title: 'Добавить приватный ключ', multiple: false, directory: false });
    if (typeof path !== 'string') return;
    if (await attempt(() => api.keyAdd(path))) await refreshKeys();
  }

  async function forget(k: KeyInfo): Promise<void> {
    await attempt(() => api.keyForget(k.path));
    await refreshKeys();
  }
</script>

<div class="head">
  <h3>Ключи</h3>
  <button class="icon" type="button" title="Обновить" onclick={refreshKeys}><Icon name="refresh" /></button>
  <button class="icon" type="button" title="Добавить файл ключа" onclick={addFile}><Icon name="file" /></button>
  <button class="icon" type="button" title="Сгенерировать ключ" onclick={() => (ui.keyGenOpen = true)}><Icon name="plus" /></button>
</div>

<div class="actions">
  <button class="btn primary" type="button" onclick={() => (ui.deploy = {})}>
    <Icon name="upload" size={14} /> Установить ключ на сервер
  </button>
</div>

<div class="list">
  {#each store.keys as k (k.path)}
    <div class="key">
      <div class="top">
        <Icon name={k.encrypted ? 'lock' : 'key'} size={13} />
        <span class="name" title={k.path}>{k.name}</span>
        <span class="type">{k.algorithm}</span>
      </div>
      <div class="fp mono" title={k.fingerprint}>{k.fingerprint}</div>
      {#if k.comment}<div class="hint">{k.comment}</div>{/if}
      <div class="hint">{k.encrypted ? 'С passphrase' : 'Без passphrase'}{k.manual ? ' - добавлен вручную' : ''}</div>
      <div class="buttons">
        <button class="icon" type="button" title="Скопировать публичный ключ" onclick={() => copyPublic(k)}><Icon name="copy" size={14} /></button>
        <button class="icon" type="button" title="Установить на сервер" onclick={() => (ui.deploy = { keyPath: k.path })}><Icon name="upload" size={14} /></button>
        {#if k.manual}
          <button class="icon" type="button" title="Убрать из списка (файл не удаляется)" onclick={() => forget(k)}><Icon name="x" size={14} /></button>
        {/if}
      </div>
    </div>
  {:else}
    <p class="hint empty">В %USERPROFILE%\.ssh ключей не найдено. Сгенерируйте новый ключ кнопкой «+».</p>
  {/each}
</div>

<style>
  .head {
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 8px 6px 4px 12px;
  }
  h3 {
    flex: 1;
    margin: 0;
    font-size: 12px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-dim);
  }
  .actions {
    padding: 4px 8px 8px;
  }
  .actions .btn {
    width: 100%;
  }
  .list {
    flex: 1;
    overflow: auto;
    padding: 0 8px 8px;
  }
  .key {
    position: relative;
    padding: 8px 10px;
    margin-bottom: 6px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg);
  }
  .top {
    display: flex;
    align-items: center;
    gap: 6px;
    padding-right: 4px;
  }
  .name {
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .type {
    margin-left: auto;
    color: var(--text-dim);
    font-size: 11px;
  }
  .fp {
    margin: 4px 0 2px;
    color: var(--text-dim);
    font-size: 11px;
  }
  .buttons {
    display: flex;
    justify-content: flex-end;
    gap: 2px;
    margin-top: 2px;
  }
  .empty {
    padding: 4px;
  }
</style>
