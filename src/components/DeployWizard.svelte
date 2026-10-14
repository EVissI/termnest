<script lang="ts">
  import { onDestroy } from 'svelte';
  import { api, errorMessage, type DisableReport, type Host, type InstallReport } from '../lib/ipc';
  import { refreshKeys, store, toast, ui } from '../stores/app.svelte';
  import Modal from './Modal.svelte';
  import KeyGenForm from './KeyGenForm.svelte';
  import Icon from './Icon.svelte';

  let { hostId, keyPath }: { hostId?: string; keyPath?: string } = $props();

  type Step = 'setup' | 'installing' | 'result' | 'confirm-disable' | 'disabling' | 'disabled';

  // svelte-ignore state_referenced_locally
  let selectedHost = $state(hostId ?? '');
  let manualTarget = $state('');
  // svelte-ignore state_referenced_locally
  let key = $state(keyPath ?? '');
  let password = $state('');
  let generating = $state(false);
  let step = $state<Step>('setup');
  let error = $state('');
  let report = $state<InstallReport | null>(null);
  let disableReport = $state<DisableReport | null>(null);
  let saveName = $state('');
  let bound = $state(false);

  const host = $derived<Host | undefined>(store.hosts.hosts.find((h) => h.id === selectedHost));
  const target = $derived(host ? `${host.user}@${host.host}:${host.port}` : manualTarget.trim());

  $effect(() => {
    // Preselect the host's own key, otherwise the first key in the list.
    if (!key) key = host?.key_path ?? store.keys[0]?.path ?? '';
  });

  void refreshKeys();

  async function install(e: SubmitEvent): Promise<void> {
    e.preventDefault();
    error = '';
    step = 'installing';
    try {
      report = await api.deployInstall(target, password, key);
      step = 'result';
    } catch (err) {
      error = errorMessage(err);
      step = 'setup';
    } finally {
      // The password is only needed for this one call.
      password = '';
    }
  }

  async function bindKey(): Promise<void> {
    try {
      if (host) {
        await api.hostUpsert({ ...host, key_path: key });
      } else {
        const [userHost, port] = splitTarget(target);
        const [user, hostname] = userHost;
        await api.hostUpsert({
          id: '',
          name: saveName.trim() || hostname,
          host: hostname,
          port,
          user,
          key_path: key,
          group: null,
          note: '',
        });
      }
      bound = true;
      toast(host ? 'Ключ привязан к серверу' : 'Сервер сохранён', 'ok', 2500);
    } catch (err) {
      toast(errorMessage(err), 'error');
    }
  }

  function splitTarget(t: string): [[string, string], number] {
    const at = t.lastIndexOf('@');
    const user = t.slice(0, at);
    let rest = t.slice(at + 1);
    let port = 22;
    const m = /^(.*):(\d+)$/.exec(rest);
    if (m && !rest.startsWith('[')) {
      rest = m[1];
      port = Number(m[2]);
    }
    return [[user, rest.replace(/^\[|\]$/g, '')], port];
  }

  async function disablePassword(): Promise<void> {
    if (!report) return;
    step = 'disabling';
    error = '';
    try {
      disableReport = await api.deployDisablePassword(report.deploy_id);
      step = 'disabled';
    } catch (err) {
      error = errorMessage(err);
      step = 'result';
    }
  }

  function close(): void {
    ui.deploy = null;
  }

  onDestroy(() => {
    password = '';
    if (report) void api.deployFinish(report.deploy_id);
  });
</script>

<Modal title="Установить ключ на сервер" onclose={close} width={560}>
  {#if step === 'setup' || step === 'installing'}
    <form id="deploy-form" onsubmit={install}>
      <h4>1. Сервер</h4>
      <div class="row">
        <select bind:value={selectedHost} class="grow" aria-label="Сервер">
          <option value="">Ввести адрес вручную</option>
          {#each store.hosts.hosts as h (h.id)}
            <option value={h.id}>{h.name} ({h.user}@{h.host})</option>
          {/each}
        </select>
      </div>
      {#if !selectedHost}
        <input class="full" bind:value={manualTarget} placeholder="user@host:port" spellcheck="false" aria-label="Адрес" />
      {/if}

      <h4>2. Ключ</h4>
      {#if generating}
        <div class="gen">
          <KeyGenForm
            ondone={(k) => {
              key = k.path;
              generating = false;
            }}
            oncancel={() => (generating = false)}
          />
        </div>
      {:else}
        <div class="row">
          <select bind:value={key} class="grow" aria-label="Ключ">
            {#each store.keys as k (k.path)}
              <option value={k.path}>{k.name} ({k.algorithm}){k.encrypted ? ', с passphrase' : ''}</option>
            {/each}
          </select>
          <button class="btn" type="button" onclick={() => (generating = true)}>Новый ключ...</button>
        </div>
      {/if}

      <h4>3. Пароль от сервера</h4>
      <input
        class="full"
        type="password"
        bind:value={password}
        autocomplete="off"
        placeholder="Используется один раз и не сохраняется"
        aria-label="Пароль"
      />
      <p class="hint">
        Termnest подключится по паролю (с проверкой ключа хоста), добавит публичный ключ в ~/.ssh/authorized_keys, если его там
        ещё нет, и проверит вход по ключу новым подключением.
      </p>
      {#if error}<div class="err-box">{error}</div>{/if}
    </form>
  {:else if step === 'result' && report}
    {#if report.added}
      <div class="ok-box"><Icon name="check" size={14} /> Ключ добавлен в authorized_keys.</div>
    {:else}
      <div class="ok-box"><Icon name="check" size={14} /> Ключ уже был на сервере, дубль не создан.</div>
    {/if}
    {#if report.verified}
      <div class="ok-box"><Icon name="check" size={14} /> Вход по ключу работает.</div>
    {:else}
      <div class="err-box"><Icon name="warning" size={14} /> Вход по ключу не работает: {report.verify_error}</div>
    {/if}

    {#if report.verified}
      <h4>Привязать ключ</h4>
      {#if bound}
        <p class="hint">Готово.</p>
      {:else if host}
        <div class="row">
          <span class="grow">Использовать этот ключ для сервера «{host.name}»?</span>
          <button class="btn" type="button" onclick={bindKey}>Привязать</button>
        </div>
      {:else}
        <div class="row">
          <input class="grow" bind:value={saveName} placeholder="Название сервера" aria-label="Название" />
          <button class="btn" type="button" onclick={bindKey}>Сохранить сервер</button>
        </div>
      {/if}

      <h4>Вход по паролю (необязательно)</h4>
      {#if report.can_disable_password}
        <p class="hint">{report.privilege_note}. Можно отключить вход по паролю на этом сервере.</p>
        <button class="btn" type="button" onclick={() => (step = 'confirm-disable')}>Отключить вход по паролю...</button>
      {:else}
        <p class="hint">{report.privilege_note}.</p>
        <p class="hint">
          Вручную: от root создайте /etc/ssh/sshd_config.d/00-termnest-keys-only.conf со строкой «PasswordAuthentication no»,
          выполните «sshd -t», затем «systemctl reload ssh» и, не закрывая текущую сессию, проверьте вход по ключу в новом окне.
        </p>
      {/if}
    {/if}
    {#if error}<div class="err-box">{error}</div>{/if}
  {:else if step === 'confirm-disable' || step === 'disabling'}
    <div class="warn-box">
      <b>Что произойдёт:</b>
      <ol>
        <li>Текущее соединение остаётся открытым до конца операции.</li>
        <li>Создаётся /etc/ssh/sshd_config.d/00-termnest-keys-only.conf с «PasswordAuthentication no».</li>
        <li>Проверка конфигурации «sshd -t»; при ошибке файл удаляется.</li>
        <li>Перезагрузка sshd (systemctl reload ssh или sshd).</li>
        <li>Новое подключение по ключу; если не удалось, файл удаляется и sshd перезагружается снова.</li>
      </ol>
      После этого войти на сервер по паролю будет нельзя - только по ключу.
    </div>
  {:else if step === 'disabled' && disableReport}
    <div class="ok-box"><Icon name="check" size={14} /> Вход по паролю отключён.</div>
    <ul class="steps">
      {#each disableReport.steps as s, i (i)}<li>{s}</li>{/each}
    </ul>
    {#if disableReport.effective}<p class="hint mono">sshd -T: {disableReport.effective}</p>{/if}
  {/if}

  {#snippet footer()}
    {#if step === 'setup' || step === 'installing'}
      <button class="btn" type="button" onclick={close}>Отмена</button>
      <button
        class="btn primary"
        type="submit"
        form="deploy-form"
        disabled={step === 'installing' || !target || !key || !password || generating}
      >
        {step === 'installing' ? 'Установка...' : 'Установить'}
      </button>
    {:else if step === 'confirm-disable' || step === 'disabling'}
      <button class="btn" type="button" onclick={() => (step = 'result')} disabled={step === 'disabling'}>Назад</button>
      <button class="btn danger" type="button" onclick={disablePassword} disabled={step === 'disabling'}>
        {step === 'disabling' ? 'Выполняется...' : 'Отключить вход по паролю'}
      </button>
    {:else}
      <button class="btn primary" type="button" onclick={close}>Готово</button>
    {/if}
  {/snippet}
</Modal>

<style>
  h4 {
    margin: 12px 0 6px;
    font-size: 13px;
  }
  h4:first-child {
    margin-top: 0;
  }
  .grow {
    flex: 1;
    min-width: 0;
  }
  .full {
    width: 100%;
    margin-top: 6px;
  }
  .gen {
    padding: 10px;
    border: 1px solid var(--border);
    border-radius: 6px;
  }
  ol,
  .steps {
    margin: 6px 0;
    padding-left: 20px;
  }
  li {
    margin: 2px 0;
  }
</style>
