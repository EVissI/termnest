<script lang="ts">
  import { onMount } from 'svelte';
  import { open } from '@tauri-apps/plugin-dialog';
  import { api, errorMessage, type KnownHost, type Settings } from '../lib/ipc';
  import { themes } from '../lib/themes';
  import { comboFor, comboOf, hotkeyActions, normalizeCombo, type HotkeyAction } from '../lib/hotkeys';
  import { confirmDialog, store, toast, ui } from '../stores/app.svelte';
  import Modal from './Modal.svelte';

  type Section = 'appearance' | 'behavior' | 'hotkeys' | 'hosts' | 'about';
  let section = $state<Section>('appearance');
  let form = $state<Settings>(JSON.parse(JSON.stringify(store.settings)));
  let capturing = $state<HotkeyAction | null>(null);
  let known = $state<KnownHost[]>([]);
  let info = $state<{ version: string; data_dir: string } | null>(null);
  let error = $state('');

  onMount(() => {
    void api.knownHosts().then((k) => (known = k));
    void api.appInfo().then((i) => (info = i));
  });

  async function save(): Promise<void> {
    try {
      await api.settingsSet($state.snapshot(form));
      ui.settingsOpen = false;
    } catch (e) {
      error = errorMessage(e);
    }
  }

  function startCapture(action: HotkeyAction): void {
    capturing = action;
    const handler = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();
      if (e.key === 'Escape') {
        stop();
        return;
      }
      const combo = comboOf(e);
      if (!combo) return; // bare modifier, keep waiting
      form.hotkeys[action] = normalizeCombo(combo);
      stop();
    };
    const stop = () => {
      capturing = null;
      window.removeEventListener('keydown', handler, true);
    };
    window.addEventListener('keydown', handler, true);
  }

  function reset(action: HotkeyAction): void {
    delete form.hotkeys[action];
  }

  function conflict(action: HotkeyAction): string | null {
    const combo = normalizeCombo(comboFor(action, form.hotkeys));
    if (!combo) return null;
    const other = hotkeyActions.find((a) => a.id !== action && normalizeCombo(comboFor(a.id, form.hotkeys)) === combo);
    return other ? other.label : null;
  }

  async function browseDefaultKey(): Promise<void> {
    const dir = await api.sshDir().catch(() => null);
    const path = await open({ title: 'Ключ по умолчанию', defaultPath: dir ?? undefined, multiple: false, directory: false });
    if (typeof path === 'string') form.default_key_path = path;
  }

  async function forgetHost(h: KnownHost): Promise<void> {
    const ok = await confirmDialog({
      title: 'Удалить ключ хоста',
      message: `Удалить сохранённый ключ ${h.host}:${h.port}? При следующем подключении отпечаток спросят заново.`,
      confirmLabel: 'Удалить',
      danger: true,
    });
    if (!ok) return;
    await api.knownHostRemove(h.host, h.port);
    known = await api.knownHosts();
  }

  async function clearPassphrases(): Promise<void> {
    await api.passphrasesClear();
    toast('Запомненные passphrase забыты', 'ok', 2500);
  }

  const sections: { id: Section; label: string }[] = [
    { id: 'appearance', label: 'Внешний вид' },
    { id: 'behavior', label: 'Поведение' },
    { id: 'hotkeys', label: 'Горячие клавиши' },
    { id: 'hosts', label: 'Известные хосты' },
    { id: 'about', label: 'О программе' },
  ];
</script>

<Modal title="Настройки" onclose={() => (ui.settingsOpen = false)} width={720}>
  <div class="layout">
    <nav>
      {#each sections as s (s.id)}
        <button type="button" class:active={section === s.id} onclick={() => (section = s.id)}>{s.label}</button>
      {/each}
    </nav>
    <div class="content">
      {#if section === 'appearance'}
        <label class="field">
          <span>Тема</span>
          <select bind:value={form.theme}>
            {#each themes as t (t.id)}<option value={t.id}>{t.name}</option>{/each}
          </select>
        </label>
        <label class="field">
          <span>Шрифт</span>
          <input bind:value={form.font_family} spellcheck="false" />
        </label>
        <div class="row">
          <label class="field">
            <span>Размер</span>
            <input type="number" min="6" max="48" bind:value={form.font_size} />
          </label>
          <label class="field">
            <span>Межстрочный интервал</span>
            <input type="number" min="0.8" max="2" step="0.05" bind:value={form.line_height} />
          </label>
          <label class="field">
            <span>Прокрутка (строк)</span>
            <input type="number" min="100" max="200000" step="1000" bind:value={form.scrollback} />
          </label>
        </div>
        <div class="row">
          <label class="field">
            <span>Курсор</span>
            <select bind:value={form.cursor_style}>
              <option value="block">Блок</option>
              <option value="underline">Подчёркивание</option>
              <option value="bar">Вертикальная черта</option>
            </select>
          </label>
          <label class="check"><input type="checkbox" bind:checked={form.cursor_blink} /> Мигающий курсор</label>
        </div>
        <div class="preview" style:font-family={form.font_family} style:font-size="{form.font_size}px" style:line-height={form.line_height}>
          user@server:~$ ls -la<br />drwxr-xr-x 4 user user 4096 .
        </div>
      {:else if section === 'behavior'}
        <label class="check"><input type="checkbox" bind:checked={form.copy_on_select} /> Копировать при выделении</label>
        <label class="check"><input type="checkbox" bind:checked={form.warn_multiline_paste} /> Предупреждать при вставке нескольких строк</label>
        <label class="field">
          <span>Keepalive, секунд (0 - выключен)</span>
          <input type="number" min="0" max="3600" bind:value={form.keepalive_secs} />
        </label>
        <label class="field">
          <span>Закрытие окна с активными сессиями</span>
          <select bind:value={form.on_window_close}>
            <option value="ask">Спросить</option>
            <option value="close_sessions">Закрыть сессии</option>
          </select>
        </label>
        <label class="field">
          <span>При запуске</span>
          <select bind:value={form.restore}>
            <option value="auto">Восстановить окна и переподключиться</option>
            <option value="ask">Восстановить окна, переподключение - по запросу</option>
            <option value="off">Начать с пустого окна</option>
          </select>
        </label>
        <label class="field">
          <span>Ключ по умолчанию (для быстрого подключения и серверов без своего ключа)</span>
          <div class="row">
            <select bind:value={form.default_key_path} class="grow">
              <option value={null}>Автоматически (id_ed25519, id_ecdsa, id_rsa)</option>
              {#each store.keys as k (k.path)}<option value={k.path}>{k.name} ({k.algorithm})</option>{/each}
              {#if form.default_key_path && !store.keys.some((k) => k.path === form.default_key_path)}
                <option value={form.default_key_path}>{form.default_key_path}</option>
              {/if}
            </select>
            <button class="btn" type="button" onclick={browseDefaultKey}>Обзор...</button>
          </div>
        </label>
        <button class="btn" type="button" onclick={clearPassphrases}>Забыть запомненные passphrase</button>
      {:else if section === 'hotkeys'}
        <table class="hotkeys">
          <tbody>
            {#each hotkeyActions as a (a.id)}
              {@const c = conflict(a.id)}
              <tr>
                <td>{a.label}{#if c}<div class="error-text small">Совпадает с «{c}»</div>{/if}</td>
                <td class="combo mono">
                  {#if capturing === a.id}
                    <span class="capturing">Нажмите сочетание (Esc - отмена)</span>
                  {:else}
                    {comboFor(a.id, form.hotkeys) || 'не назначено'}
                  {/if}
                </td>
                <td class="actions">
                  <button class="btn" type="button" onclick={() => startCapture(a.id)}>Изменить</button>
                  <button class="btn" type="button" onclick={() => (form.hotkeys[a.id] = '')} title="Отключить">Откл.</button>
                  <button class="btn" type="button" onclick={() => reset(a.id)} disabled={!(a.id in form.hotkeys)}>Сброс</button>
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
      {:else if section === 'hosts'}
        {#each known as h (`${h.host}:${h.port}:${h.key_type}`)}
          <div class="known">
            <div class="grow">
              <b>{h.host}{h.port !== 22 ? `:${h.port}` : ''}</b> <span class="hint">{h.key_type}</span>
              <div class="mono hint">{h.fingerprint}</div>
            </div>
            <button class="btn" type="button" onclick={() => forgetHost(h)}>Удалить</button>
          </div>
        {:else}
          <p class="hint">Пока нет сохранённых ключей хостов.</p>
        {/each}
      {:else}
        <p><b>termnest</b> {info?.version ?? ''}</p>
        <p class="hint">SSH-клиент для Windows: Tauri 2, Rust (russh), Svelte, xterm.js.</p>
        <p class="hint">Данные: <span class="mono">{info?.data_dir ?? ''}</span></p>
        <p class="hint">Пароли и passphrase никогда не сохраняются на диск.</p>
      {/if}
      {#if error}<p class="error-text">{error}</p>{/if}
    </div>
  </div>
  {#snippet footer()}
    <button class="btn" type="button" onclick={() => (ui.settingsOpen = false)}>Отмена</button>
    <button class="btn primary" type="button" onclick={save}>Сохранить</button>
  {/snippet}
</Modal>

<style>
  .layout {
    display: flex;
    gap: 14px;
    min-height: 380px;
  }
  nav {
    display: flex;
    flex-direction: column;
    gap: 2px;
    width: 160px;
    flex-shrink: 0;
  }
  nav button {
    padding: 6px 10px;
    border: none;
    border-radius: 4px;
    background: none;
    text-align: left;
    cursor: pointer;
  }
  nav button:hover {
    background: var(--bg-hover);
  }
  nav button.active {
    background: var(--bg-active);
  }
  .content {
    flex: 1;
    min-width: 0;
  }
  .content .row .field {
    flex: 1;
  }
  input,
  select {
    width: 100%;
  }
  .grow {
    flex: 1;
    min-width: 0;
  }
  .preview {
    padding: 8px 10px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg);
    white-space: pre;
  }
  .hotkeys {
    width: 100%;
    border-collapse: collapse;
  }
  .hotkeys td {
    padding: 4px 4px;
    border-bottom: 1px solid var(--border);
  }
  .combo {
    width: 170px;
  }
  .capturing {
    color: var(--accent);
  }
  .actions {
    white-space: nowrap;
    text-align: right;
  }
  .actions .btn {
    padding: 2px 8px;
    font-size: 12px;
  }
  .small {
    font-size: 11px;
  }
  .known {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 0;
    border-bottom: 1px solid var(--border);
  }
</style>
