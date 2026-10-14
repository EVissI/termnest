<script lang="ts">
  import { onMount } from 'svelte';
  import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow';
  import { api, errorMessage } from './lib/ipc';
  import { bindings, comboOf } from './lib/hotkeys';
  import { attempt, runAction } from './lib/actions';
  import { initStore, store, toast, ui, windowLabel } from './stores/app.svelte';
  import Sidebar from './components/Sidebar.svelte';
  import TabBar from './components/TabBar.svelte';
  import Workspace from './components/Workspace.svelte';
  import SftpPanel from './components/SftpPanel.svelte';
  import HostDialog from './components/HostDialog.svelte';
  import KeyGenDialog from './components/KeyGenDialog.svelte';
  import DeployWizard from './components/DeployWizard.svelte';
  import SnippetDialog from './components/SnippetDialog.svelte';
  import SnippetPalette from './components/SnippetPalette.svelte';
  import SettingsDialog from './components/SettingsDialog.svelte';
  import PromptDialog from './components/PromptDialog.svelte';
  import ConfirmDialog from './components/ConfirmDialog.svelte';
  import TextPromptDialog from './components/TextPromptDialog.svelte';
  import WindowDialogs from './components/WindowDialogs.svelte';
  import Toasts from './components/Toasts.svelte';

  let ready = $state(false);
  let fatal = $state('');

  const keymap = $derived(bindings(store.settings?.hotkeys ?? {}));

  /** Dialogs that take the keyboard: global hotkeys are off while one is open. */
  const modalOpen = $derived(
    !!ui.confirm ||
      !!ui.textPrompt ||
      !!ui.hostDialog ||
      ui.settingsOpen ||
      ui.keyGenOpen ||
      !!ui.deploy ||
      !!ui.snippetEdit ||
      !!ui.closeRequest ||
      !!ui.hostKeyChanged ||
      store.prompts.length > 0,
  );

  onMount(() => {
    initStore()
      .then(async () => {
        ready = true;
        const info = await api.startupInfo();
        ui.restorePending = info.restore_pending;
        for (const w of info.warnings) toast(w, 'error', 15000);
      })
      .catch((e) => (fatal = errorMessage(e)));

    const onKey = (e: KeyboardEvent) => {
      const combo = comboOf(e);
      if (!combo) return;
      const action = keymap.get(combo);
      if (!action) return;
      if (modalOpen && action !== 'snippets.palette') return;
      if (ui.palette && action !== 'snippets.palette') return;
      e.preventDefault();
      e.stopPropagation();
      void runAction(action);
    };
    window.addEventListener('keydown', onKey, true);
    // Disable the browser's own context menu outside of our handlers.
    const noMenu = (e: MouseEvent) => {
      const t = e.target as HTMLElement;
      if (!t.closest('input, textarea')) e.preventDefault();
    };
    window.addEventListener('contextmenu', noMenu);
    return () => {
      window.removeEventListener('keydown', onKey, true);
      window.removeEventListener('contextmenu', noMenu);
    };
  });

  // Window title: "termnest" for the first window, "termnest (2)" for others.
  $effect(() => {
    const index = store.layout.windows.findIndex((w) => w.label === windowLabel);
    const title = index > 0 ? `termnest (${index + 1})` : 'termnest';
    getCurrentWebviewWindow().setTitle(title).catch(() => {});
  });

  // Selection only makes sense for panels that still exist.
  $effect(() => {
    const ids = new Set(
      store.layout.windows.flatMap((w) => w.tabs).flatMap((t) => collect(t.root)),
    );
    if (ui.selected.some((id) => !ids.has(id))) ui.selected = ui.selected.filter((id) => ids.has(id));
  });

  function collect(node: import('./lib/ipc').LayoutNode): number[] {
    return node.type === 'pane' ? [node.id] : [...collect(node.first), ...collect(node.second)];
  }

  async function reconnectAll(): Promise<void> {
    ui.restorePending = false;
    await attempt(() => api.restoreConnectAll());
  }
</script>

{#if fatal}
  <div class="fatal">
    <h2>Не удалось запустить termnest</h2>
    <p>{fatal}</p>
  </div>
{:else if ready && store.settings}
  <div class="app">
    <Sidebar />
    <div class="main">
      {#if ui.restorePending}
        <div class="restore">
          <span>Раскладка восстановлена. Переподключить все сессии?</span>
          <button class="btn primary" type="button" onclick={reconnectAll}>Подключить все</button>
          <button class="btn" type="button" onclick={() => (ui.restorePending = false)}>Не сейчас</button>
        </div>
      {/if}
      <TabBar />
      <Workspace />
    </div>
    {#if ui.sftpOpen}
      <SftpPanel />
    {/if}
  </div>

  {#if ui.hostDialog}<HostDialog host={ui.hostDialog} />{/if}
  {#if ui.keyGenOpen}<KeyGenDialog />{/if}
  {#if ui.deploy}<DeployWizard hostId={ui.deploy.hostId} keyPath={ui.deploy.keyPath} />{/if}
  {#if ui.snippetEdit}<SnippetDialog snippet={ui.snippetEdit} />{/if}
  {#if ui.palette}<SnippetPalette />{/if}
  {#if ui.settingsOpen}<SettingsDialog />{/if}
  <WindowDialogs />
  <PromptDialog />
  <TextPromptDialog />
  <ConfirmDialog />
{/if}
<Toasts />

<style>
  .app {
    display: flex;
    height: 100%;
  }
  .main {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-width: 0;
  }
  .restore {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 10px;
    background: color-mix(in srgb, var(--accent) 18%, var(--bg-panel));
    border-bottom: 1px solid var(--border);
  }
  .restore span {
    flex: 1;
  }
  .fatal {
    padding: 40px;
  }
</style>
