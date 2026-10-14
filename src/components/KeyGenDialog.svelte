<script lang="ts">
  import { writeText } from '@tauri-apps/plugin-clipboard-manager';
  import { api, type KeyInfo } from '../lib/ipc';
  import { toast, ui } from '../stores/app.svelte';
  import Modal from './Modal.svelte';
  import KeyGenForm from './KeyGenForm.svelte';

  let created = $state<KeyInfo | null>(null);

  async function copy(): Promise<void> {
    if (!created) return;
    await writeText(await api.keyPublic(created.path));
    toast('Публичный ключ скопирован', 'ok', 2500);
  }
</script>

<Modal title="Новый ключ" onclose={() => (ui.keyGenOpen = false)} width={520}>
  {#if created}
    <div class="ok-box">
      Ключ создан: <b>{created.name}</b> ({created.algorithm})
      <div class="mono">{created.fingerprint}</div>
    </div>
    <div class="row">
      <button class="btn" type="button" onclick={copy}>Скопировать публичный ключ</button>
      <button
        class="btn primary"
        type="button"
        onclick={() => {
          ui.keyGenOpen = false;
          ui.deploy = { keyPath: created!.path };
        }}>Установить на сервер</button
      >
    </div>
  {:else}
    <KeyGenForm ondone={(k) => (created = k)} oncancel={() => (ui.keyGenOpen = false)} />
  {/if}
</Modal>
