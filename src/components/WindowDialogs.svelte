<script lang="ts">
  import { api } from '../lib/ipc';
  import { attempt } from '../lib/actions';
  import { confirmDialog, ui, windowLabel } from '../stores/app.svelte';
  import Modal from './Modal.svelte';
  import Icon from './Icon.svelte';

  /** "Window has active sessions" and "host key changed" dialogs. */

  async function closeWindow(action: 'close_sessions' | 'move_tabs'): Promise<void> {
    ui.closeRequest = null;
    await attempt(() => api.windowClose(windowLabel, action));
  }

  async function removeHostKey(): Promise<void> {
    const req = ui.hostKeyChanged;
    if (!req) return;
    const ok = await confirmDialog({
      title: 'Удалить старый ключ хоста?',
      message:
        'Делайте это, только если точно знаете, почему ключ сервера изменился (переустановка ОС, новый сервер на том же адресе). ' +
        'После удаления при подключении будет показан новый отпечаток для проверки.',
      confirmLabel: 'Удалить и подключиться',
      danger: true,
    });
    if (!ok) return;
    ui.hostKeyChanged = null;
    await attempt(() => api.knownHostRemove(req.info.host, req.info.port));
    await attempt(() => api.retry(req.session));
  }
</script>

{#if ui.closeRequest}
  {@const req = ui.closeRequest}
  <Modal title="Закрыть окно?" onclose={() => (ui.closeRequest = null)}>
    <p>В окне активных сессий: {req.sessions}. Что с ними сделать?</p>
    {#snippet footer()}
      <button class="btn" type="button" onclick={() => (ui.closeRequest = null)}>Отмена</button>
      {#if req.other_windows}
        <button class="btn" type="button" onclick={() => closeWindow('move_tabs')}>Перенести вкладки в другое окно</button>
      {/if}
      <button class="btn danger" type="button" onclick={() => closeWindow('close_sessions')}>Закрыть сессии</button>
    {/snippet}
  </Modal>
{/if}

{#if ui.hostKeyChanged}
  {@const req = ui.hostKeyChanged}
  <Modal title="Ключ сервера изменился" onclose={() => (ui.hostKeyChanged = null)} width={560}>
    <div class="err-box">
      <Icon name="warning" size={14} />
      <b>Внимание!</b> Ключ сервера <b>{req.info.host}:{req.info.port}</b> не совпадает с сохранённым. Это может означать атаку
      «человек посередине». Подключение заблокировано.
    </div>
    <div class="fp">
      <div class="hint">Сохранённый отпечаток</div>
      <div class="mono">{req.info.known_fingerprint ?? 'неизвестен'}</div>
      <div class="hint">Новый отпечаток ({req.info.key_type})</div>
      <div class="mono">{req.info.fingerprint}</div>
    </div>
    <p class="hint">Продолжить можно только явным удалением старой записи из known_hosts.</p>
    {#snippet footer()}
      <button class="btn" type="button" onclick={() => (ui.hostKeyChanged = null)}>Закрыть</button>
      <button class="btn danger" type="button" onclick={removeHostKey}>Удалить старую запись...</button>
    {/snippet}
  </Modal>
{/if}

<style>
  .fp {
    display: grid;
    gap: 2px;
    padding: 10px;
    margin: 8px 0;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg-input);
  }
  .fp .hint:not(:first-child) {
    margin-top: 6px;
  }
</style>
