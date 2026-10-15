<script lang="ts">
  import { open } from '@tauri-apps/plugin-dialog';
  import { api, errorMessage, type Host } from '../lib/ipc';
  import { store, toast, ui } from '../stores/app.svelte';
  import Modal from './Modal.svelte';
  import { pickProject } from '../lib/projects';

  let { host }: { host: Host } = $props();

  // svelte-ignore state_referenced_locally
  let form = $state({
    ...host,
    port: host.port || 22,
    key_path: host.key_path ?? '',
    group: host.group ?? '',
    project: host.project ?? '',
  });
  let error = $state('');
  let saving = $state(false);

  const isNew = $derived(!host.id);

  async function browseKey(): Promise<void> {
    const dir = await api.sshDir().catch(() => null);
    const path = await open({ title: 'Приватный ключ', defaultPath: dir ?? undefined, multiple: false, directory: false });
    if (typeof path === 'string') form.key_path = path;
  }

  async function browseProject(): Promise<void> {
    const path = await pickProject('Проект сервера', form.project || null);
    if (path) form.project = path;
  }

  async function save(e: SubmitEvent): Promise<void> {
    e.preventDefault();
    saving = true;
    error = '';
    try {
      await api.hostUpsert({
        ...form,
        port: Number(form.port) || 22,
        key_path: form.key_path.trim() || null,
        group: form.group.trim() || null,
        project: form.project.trim() || null,
      });
      toast(isNew ? 'Сервер добавлен' : 'Сервер сохранён', 'ok', 2500);
      ui.hostDialog = null;
    } catch (err) {
      error = errorMessage(err);
    } finally {
      saving = false;
    }
  }
</script>

<Modal title={isNew ? 'Новый сервер' : 'Редактировать сервер'} onclose={() => (ui.hostDialog = null)} width={500}>
  <form id="host-form" onsubmit={save}>
    <label class="field">
      <span>Название</span>
      <!-- svelte-ignore a11y_autofocus -->
      <input bind:value={form.name} placeholder="Например, prod-web" autofocus />
    </label>
    <div class="row">
      <label class="field grow">
        <span>Пользователь</span>
        <input bind:value={form.user} placeholder="root" required spellcheck="false" />
      </label>
      <label class="field grow2">
        <span>Адрес</span>
        <input bind:value={form.host} placeholder="example.com или 10.0.0.5" required spellcheck="false" />
      </label>
      <label class="field port">
        <span>Порт</span>
        <input type="number" min="1" max="65535" bind:value={form.port} />
      </label>
    </div>
    <label class="field">
      <span>Приватный ключ (пусто - ключ по умолчанию)</span>
      <div class="row">
        <select bind:value={form.key_path} class="grow">
          <option value="">По умолчанию</option>
          {#each store.keys as k (k.path)}
            <option value={k.path}>{k.name} ({k.algorithm})</option>
          {/each}
          {#if form.key_path && !store.keys.some((k) => k.path === form.key_path)}
            <option value={form.key_path}>{form.key_path}</option>
          {/if}
        </select>
        <button class="btn" type="button" onclick={browseKey}>Обзор...</button>
      </div>
    </label>
    <label class="field">
      <span>Группа</span>
      <input bind:value={form.group} list="host-groups" placeholder="Без группы" />
      <datalist id="host-groups">
        {#each store.hosts.groups as g (g.name)}
          <option value={g.name}></option>
        {/each}
      </datalist>
    </label>
    <div class="field">
      <span>Проект (локальная папка репозитория, для Claude Code)</span>
      <div class="row">
        <input class="grow" bind:value={form.project} placeholder="Не привязан" spellcheck="false" aria-label="Проект" />
        <button class="btn" type="button" onclick={browseProject}>Обзор...</button>
      </div>
    </div>
    <label class="field">
      <span>Заметка</span>
      <textarea bind:value={form.note} rows="2"></textarea>
    </label>
    <p class="hint">Пароли не сохраняются: вход только по ключу. Чтобы поставить ключ на сервер с паролем, используйте мастер в разделе «Ключи».</p>
    {#if error}<p class="error-text">{error}</p>{/if}
  </form>
  {#snippet footer()}
    <button class="btn" type="button" onclick={() => (ui.hostDialog = null)}>Отмена</button>
    <button class="btn primary" type="submit" form="host-form" disabled={saving}>Сохранить</button>
  {/snippet}
</Modal>

<style>
  .grow {
    flex: 1;
  }
  .grow2 {
    flex: 2;
  }
  .port {
    width: 80px;
  }
  .row .field {
    min-width: 0;
  }
  input,
  select,
  textarea {
    width: 100%;
  }
  textarea {
    resize: vertical;
  }
</style>
