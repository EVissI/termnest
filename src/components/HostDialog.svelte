<script lang="ts">
  import { open } from '@tauri-apps/plugin-dialog';
  import { onMount } from 'svelte';
  import { api, errorMessage, switchCommand, type Host, type PasswordLoginProbe } from '../lib/ipc';
  import { confirmDialog, store, toast, ui } from '../stores/app.svelte';
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
    switch_user: host.switch_user ?? '',
    switch_method: host.switch_method ?? 'sudo',
    quick_users: (host.quick_users ?? []).join(', '),
  });
  let error = $state('');
  let saving = $state(false);

  const isNew = $derived(!host.id);

  // What the server itself answers about password logins (not the stored settings).
  let login = $state<PasswordLoginProbe | null>(null);
  let loginBusy = $state<'probe' | 'change' | null>(null);
  let loginError = $state('');
  let loginSteps = $state<string[]>([]);

  async function probeLogin(): Promise<void> {
    loginBusy = 'probe';
    loginError = '';
    try {
      login = await api.hostPasswordLogin(host.id);
    } catch (err) {
      login = null;
      loginError = errorMessage(err);
    } finally {
      loginBusy = null;
    }
  }

  async function changeLogin(): Promise<void> {
    if (!login) return;
    const allow = !login.password_login;
    const ok = await confirmDialog({
      title: allow ? 'Разрешить вход без ключа' : 'Отключить вход без ключа',
      message: allow
        ? `Сервер «${host.name}» снова будет принимать пароли. Это ослабляет защиту от подбора паролей.`
        : `Сервер «${host.name}» перестанет принимать пароли, войти можно будет только по ключу. termnest проверит вход по ключу новым подключением и откатит изменение, если он не работает.`,
      confirmLabel: allow ? 'Разрешить' : 'Отключить',
      danger: allow,
    });
    if (!ok) return;
    loginBusy = 'change';
    loginError = '';
    loginSteps = [];
    try {
      const report = await api.hostSetPasswordLogin(host.id, allow);
      login = report;
      loginSteps = report.steps;
      if (report.password_login === allow) toast(allow ? 'Вход по паролю разрешён' : 'Вход по паролю отключён', 'ok');
    } catch (err) {
      loginError = errorMessage(err);
    } finally {
      loginBusy = null;
    }
  }

  onMount(() => {
    if (host.id) void probeLogin();
  });

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
        switch_user: form.switch_user.trim() || null,
        quick_users: form.quick_users
          .split(/[,\s]+/)
          .map((u) => u.trim())
          .filter(Boolean),
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
        <span>Логин</span>
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
    <div class="row">
      <label class="field grow">
        <span>Работать под пользователем</span>
        <input bind:value={form.switch_user} placeholder="Тот же, что логин" spellcheck="false" />
      </label>
      <label class="field method">
        <span>Как переключаться</span>
        <select bind:value={form.switch_method}>
          <option value="sudo">sudo -i</option>
          <option value="su">su -</option>
        </select>
      </label>
    </div>
    <label class="field">
      <span>Быстрое переключение (пользователи через запятую)</span>
      <input bind:value={form.quick_users} placeholder="Например, www-data, postgres" spellcheck="false" />
    </label>
    {#if form.switch_user.trim() && form.switch_user.trim() !== form.user.trim()}
      <p class="hint">
        После входа termnest наберёт «{switchCommand(form.switch_method, form.switch_user.trim())}», в том числе после переподключения.
        Пароль sudo или su вводится в терминале и не сохраняется. SFTP, информация о сервере и MCP работают под логином.
      </p>
    {/if}
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
    {#if !isNew}
      <div class="field">
        <span>Вход без ключа (проверяется на самом сервере)</span>
        <div class="row login">
          <span class="login-state" class:open={login?.password_login}>
            {#if loginBusy === 'probe'}
              Проверка...
            {:else if loginBusy === 'change'}
              Меняю настройки sshd...
            {:else if login}
              {login.password_login ? 'Сервер принимает пароль' : 'Только по ключу'}
              <span class="methods">({login.methods.join(', ') || 'нет методов'})</span>
            {:else}
              Не проверено
            {/if}
          </span>
          <button class="btn" type="button" onclick={probeLogin} disabled={loginBusy !== null}>Проверить</button>
          {#if login}
            <button class="btn" class:danger={!login.password_login} type="button" onclick={changeLogin} disabled={loginBusy !== null}>
              {login.password_login ? 'Отключить вход без ключа' : 'Разрешить вход без ключа'}
            </button>
          {/if}
        </div>
        {#if loginError}<p class="error-text">{loginError}</p>{/if}
        {#if loginSteps.length}
          <ul class="steps">
            {#each loginSteps as step, i (i)}<li>{step}</li>{/each}
          </ul>
        {/if}
      </div>
    {/if}
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
  .method {
    width: 130px;
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
  .login {
    align-items: center;
  }
  .login-state {
    flex: 1;
    min-width: 0;
    color: var(--ok, var(--text));
  }
  .login-state.open {
    color: var(--warn, var(--text));
  }
  .methods {
    color: var(--text-dim);
    font-size: 11px;
  }
  .steps {
    margin: 6px 0 0;
    padding-left: 18px;
    color: var(--text-dim);
    font-size: 12px;
  }
</style>
