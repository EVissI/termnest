<script lang="ts">
  import Modal from './Modal.svelte';
  import { formatMode } from '../lib/format';

  let {
    name,
    mode: initial,
    onapply,
    onclose,
  }: { name: string; mode: number; onapply: (mode: number) => void; onclose: () => void } = $props();

  // svelte-ignore state_referenced_locally
  let mode = $state(initial & 0o7777);
  const octal = $derived((mode & 0o777).toString(8).padStart(3, '0'));

  const who = ['Владелец', 'Группа', 'Остальные'];
  const what = ['Чтение', 'Запись', 'Выполнение'];

  function bit(w: number, p: number): number {
    return 1 << (8 - (w * 3 + p));
  }

  function toggle(w: number, p: number): void {
    mode ^= bit(w, p);
  }

  function setOctal(v: string): void {
    if (/^[0-7]{3,4}$/.test(v)) {
      const parsed = parseInt(v, 8);
      mode = (mode & ~0o777) | (parsed & 0o777) | (v.length === 4 ? parsed & 0o7000 : mode & 0o7000);
    }
  }
</script>

<Modal title="Права доступа" {onclose} width={400}>
  <p class="mono name">{name}</p>
  <table>
    <thead>
      <tr><th></th>{#each what as p (p)}<th>{p}</th>{/each}</tr>
    </thead>
    <tbody>
      {#each who as w, wi (w)}
        <tr>
          <td>{w}</td>
          {#each what as p, pi (p)}
            <td><input type="checkbox" checked={(mode & bit(wi, pi)) !== 0} onchange={() => toggle(wi, pi)} aria-label="{w}: {p}" /></td>
          {/each}
        </tr>
      {/each}
    </tbody>
  </table>
  <div class="row">
    <label class="field">
      <span>Восьмеричный вид</span>
      <input value={octal} oninput={(e) => setOctal((e.target as HTMLInputElement).value)} maxlength="4" class="mono" />
    </label>
    <span class="mono dim">{formatMode(mode)}</span>
  </div>
  {#snippet footer()}
    <button class="btn" type="button" onclick={onclose}>Отмена</button>
    <button class="btn primary" type="button" onclick={() => onapply(mode)}>Применить</button>
  {/snippet}
</Modal>

<style>
  .name {
    margin: 0 0 8px;
    color: var(--text-dim);
  }
  table {
    border-collapse: collapse;
    margin-bottom: 10px;
  }
  th,
  td {
    padding: 4px 10px;
    text-align: center;
    font-weight: 400;
  }
  td:first-child {
    text-align: left;
  }
  th {
    color: var(--text-dim);
    font-size: 12px;
  }
  .field input {
    width: 80px;
  }
  .dim {
    color: var(--text-dim);
  }
</style>
