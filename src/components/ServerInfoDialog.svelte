<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import { api, errorMessage, type ServerInfo } from '../lib/ipc';
  import { store, ui } from '../stores/app.svelte';
  import Modal from './Modal.svelte';

  let { source }: { source: { host: string | null; session: number | null; title: string } } = $props();

  const REFRESH_MS = 3000;

  let monitor = $state<number | null>(null);
  let keyFingerprint = $state<string | null>(null);
  let info = $state<ServerInfo | null>(null);
  let error = $state('');
  let connecting = $state(true);
  let auto = $state(true);
  let updated = $state<Date | null>(null);
  let closed = false;
  let timer: ReturnType<typeof setTimeout> | undefined;

  async function connect(): Promise<void> {
    connecting = true;
    error = '';
    try {
      const opened = await api.monitorOpen(source.host, source.session);
      if (closed) {
        void api.monitorClose(opened.id);
        return;
      }
      monitor = opened.id;
      keyFingerprint = opened.key_fingerprint;
      await refresh();
    } catch (err) {
      error = errorMessage(err);
    } finally {
      connecting = false;
    }
  }

  async function refresh(): Promise<void> {
    clearTimeout(timer);
    if (monitor === null || closed) return;
    try {
      info = await api.monitorSample(monitor);
      updated = new Date();
      error = '';
    } catch (err) {
      error = errorMessage(err);
    }
    // The next sample starts only after this one finished, so they never pile up.
    if (auto && !closed) timer = setTimeout(() => void refresh(), REFRESH_MS);
  }

  function toggleAuto(): void {
    auto = !auto;
    if (auto) void refresh();
    else clearTimeout(timer);
  }

  async function reconnect(): Promise<void> {
    if (monitor !== null) await api.monitorClose(monitor).catch(() => {});
    monitor = null;
    await connect();
  }

  onMount(() => void connect());
  onDestroy(() => {
    closed = true;
    clearTimeout(timer);
    if (monitor !== null) void api.monitorClose(monitor);
  });

  function bytes(n: number): string {
    const units = ['Б', 'КБ', 'МБ', 'ГБ', 'ТБ'];
    let v = n;
    let i = 0;
    while (v >= 1024 && i < units.length - 1) {
      v /= 1024;
      i++;
    }
    return `${v.toFixed(v >= 100 || i === 0 ? 0 : 1)} ${units[i]}`;
  }

  function uptime(secs: number): string {
    const d = Math.floor(secs / 86400);
    const h = Math.floor((secs % 86400) / 3600);
    const m = Math.floor((secs % 3600) / 60);
    return d > 0 ? `${d} д ${h} ч` : h > 0 ? `${h} ч ${m} мин` : `${m} мин`;
  }

  function level(percent: number): string {
    return percent >= 90 ? 'high' : percent >= 70 ? 'mid' : 'low';
  }

  const memUsed = $derived(info ? info.mem_total - info.mem_available : 0);
  const memPercent = $derived(info && info.mem_total ? (memUsed * 100) / info.mem_total : 0);
  const swapUsed = $derived(info ? info.swap_total - info.swap_free : 0);
  const keyCount = $derived(info ? info.key_files.reduce((n, f) => n + f.keys.length, 0) : 0);

  function localKey(fingerprint: string): string | null {
    if (!fingerprint) return null;
    return store.keys.find((k) => k.fingerprint === fingerprint)?.name ?? null;
  }
</script>

<Modal title="Сервер {source.title}" onclose={() => (ui.serverInfo = null)} width={760}>
  {#if connecting && !info}
    <p class="hint">Подключение...</p>
  {/if}
  {#if error}
    <div class="error-row">
      <p class="error-text">{error}</p>
      <button class="btn" type="button" onclick={reconnect} disabled={connecting}>Переподключить</button>
    </div>
  {/if}

  {#if info}
    <div class="summary">
      <div><span class="k">Хост</span>{info.hostname || '-'}</div>
      <div><span class="k">ОС</span>{info.os || '-'}</div>
      <div><span class="k">Ядро</span>{info.kernel || '-'}</div>
      <div><span class="k">Работает</span>{uptime(info.uptime_secs)}</div>
      <div><span class="k">Пользователь</span>{info.user || '-'}</div>
    </div>

    <div class="cards">
      <section class="card">
        <h4>CPU</h4>
        {#if info.cpu_percent !== null}
          <div class="big">{info.cpu_percent.toFixed(0)}%</div>
          <div class="bar"><span class={level(info.cpu_percent)} style:width="{Math.min(100, info.cpu_percent)}%"></span></div>
        {:else}
          <div class="big">-</div>
        {/if}
        <div class="sub">Ядер: {info.cpu_cores || '-'}, нагрузка {info.load.map((l) => l.toFixed(2)).join(' / ')}</div>
        {#if info.cpu_model}<div class="sub dim" title={info.cpu_model}>{info.cpu_model}</div>{/if}
      </section>
      <section class="card">
        <h4>Память</h4>
        <div class="big">{memPercent.toFixed(0)}%</div>
        <div class="bar"><span class={level(memPercent)} style:width="{memPercent}%"></span></div>
        <div class="sub">{bytes(memUsed)} из {bytes(info.mem_total)}</div>
        <div class="sub dim">
          Swap: {info.swap_total ? `${bytes(swapUsed)} из ${bytes(info.swap_total)}` : 'нет'}
        </div>
      </section>
      <section class="card">
        <h4>Сеть</h4>
        {#if info.net_rx_rate !== null && info.net_tx_rate !== null}
          <div class="net"><span class="dim">Приём</span>{bytes(info.net_rx_rate)}/с</div>
          <div class="net"><span class="dim">Отправка</span>{bytes(info.net_tx_rate)}/с</div>
        {:else}
          <div class="big">-</div>
        {/if}
        <div class="sub dim">Все интерфейсы, кроме lo</div>
      </section>
    </div>

    {#if info.disks.length}
      <h3>Диски</h3>
      <div class="disks">
        {#each info.disks as d (d.mount)}
          {@const p = d.total ? (d.used * 100) / d.total : 0}
          <div class="disk">
            <span class="mount" title={d.filesystem}>{d.mount}</span>
            <div class="bar"><span class={level(p)} style:width="{p}%"></span></div>
            <span class="dim nowrap">{bytes(d.used)} из {bytes(d.total)} ({p.toFixed(0)}%)</span>
          </div>
        {/each}
      </div>
    {/if}

    {#if info.processes.length}
      <h3>Процессы с наибольшей нагрузкой</h3>
      <table>
        <thead><tr><th>PID</th><th>Пользователь</th><th>CPU</th><th>RAM</th><th>Команда</th></tr></thead>
        <tbody>
          {#each info.processes as p (p.pid)}
            <tr><td>{p.pid}</td><td>{p.user}</td><td>{p.cpu.toFixed(1)}%</td><td>{p.mem.toFixed(1)}%</td><td class="cmd">{p.command}</td></tr>
          {/each}
        </tbody>
      </table>
    {/if}

    <h3>Ключи на сервере <span class="dim">({keyCount})</span></h3>
    {#if info.key_files.length === 0}
      <p class="hint">Нет доступных для чтения файлов authorized_keys.</p>
    {/if}
    {#each info.key_files as file (file.path)}
      <div class="keyfile">
        <div class="keyfile-head"><b>{file.user}</b> <span class="dim mono">{file.path}</span></div>
        {#if file.keys.length === 0}
          <p class="hint">Файл пуст.</p>
        {/if}
        {#each file.keys as key, i (i)}
          {@const local = localKey(key.fingerprint)}
          <div class="key">
            <div class="key-line">
              <span class="type">{key.key_type || 'не распознан'}</span>
              <span class="comment">{key.comment || 'без комментария'}</span>
              {#if key.fingerprint && key.fingerprint === keyFingerprint}<span class="badge current">этим ключом подключено</span>{/if}
              {#if local}<span class="badge">ваш ключ: {local}</span>{/if}
            </div>
            {#if key.fingerprint}<div class="mono dim fp">{key.fingerprint}</div>{/if}
            {#if key.options}<div class="mono dim fp" title={key.options}>Опции: {key.options}</div>{/if}
          </div>
        {/each}
      </div>
    {/each}
    {#if info.user !== 'root'}
      <p class="hint">Видны только файлы, которые может прочитать {info.user}. Ключи всех пользователей видны при входе под root.</p>
    {/if}
  {/if}

  {#snippet footer()}
    <span class="dim updated">
      {#if updated}Обновлено {updated.toLocaleTimeString()}{/if}
    </span>
    <label class="auto"><input type="checkbox" checked={auto} onchange={toggleAuto} /> Автообновление</label>
    <button class="btn" type="button" onclick={() => refresh()} disabled={monitor === null}>Обновить</button>
    <button class="btn" type="button" onclick={() => (ui.serverInfo = null)}>Закрыть</button>
  {/snippet}
</Modal>

<style>
  h3 {
    margin: 16px 0 6px;
    font-size: 12px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-dim);
  }
  h4 {
    margin: 0 0 4px;
    font-size: 12px;
    color: var(--text-dim);
    font-weight: 600;
  }
  .dim {
    color: var(--text-dim);
  }
  .mono {
    font-family: var(--mono, monospace);
  }
  .nowrap {
    white-space: nowrap;
  }
  .error-row {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .error-row p {
    flex: 1;
  }
  .summary {
    display: flex;
    flex-wrap: wrap;
    gap: 6px 18px;
    margin: 4px 0 12px;
  }
  .k {
    color: var(--text-dim);
    margin-right: 6px;
  }
  .cards {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 10px;
  }
  .card {
    padding: 10px 12px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg);
    min-width: 0;
  }
  .big {
    font-size: 22px;
    font-weight: 600;
    margin-bottom: 4px;
  }
  .sub {
    margin-top: 4px;
    font-size: 12px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .net {
    display: flex;
    justify-content: space-between;
    font-size: 14px;
    margin: 4px 0;
  }
  .bar {
    height: 6px;
    border-radius: 3px;
    background: var(--bg-hover);
    overflow: hidden;
  }
  .bar span {
    display: block;
    height: 100%;
    transition: width 0.4s;
  }
  .low {
    background: var(--ok);
  }
  .mid {
    background: var(--warn);
  }
  .high {
    background: var(--danger, #ef4444);
  }
  .disks {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .disk {
    display: grid;
    grid-template-columns: 160px 1fr auto;
    align-items: center;
    gap: 10px;
    font-size: 12px;
  }
  .mount {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 12px;
  }
  th,
  td {
    text-align: left;
    padding: 3px 6px;
    border-bottom: 1px solid var(--border);
  }
  th {
    color: var(--text-dim);
    font-weight: 600;
  }
  .cmd {
    max-width: 260px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .keyfile {
    margin-bottom: 10px;
  }
  .keyfile-head {
    margin-bottom: 4px;
  }
  .key {
    padding: 6px 8px;
    border: 1px solid var(--border);
    border-radius: 4px;
    margin-bottom: 4px;
    min-width: 0;
  }
  .key-line {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
  }
  .type {
    font-family: var(--mono, monospace);
    font-size: 12px;
    color: var(--text-dim);
  }
  .comment {
    font-weight: 600;
  }
  .badge {
    padding: 1px 6px;
    border-radius: 8px;
    font-size: 11px;
    background: var(--bg-hover);
    border: 1px solid var(--border);
  }
  .badge.current {
    border-color: var(--ok);
    color: var(--ok);
  }
  .fp {
    font-size: 11px;
    margin-top: 2px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .updated {
    margin-right: auto;
    font-size: 12px;
  }
  .auto {
    display: flex;
    align-items: center;
    gap: 4px;
    font-size: 12px;
  }
</style>
