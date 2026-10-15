<script lang="ts">
  import { saveSettings, store, ui } from '../stores/app.svelte';
  import HostList from './HostList.svelte';
  import SnippetsView from './SnippetsView.svelte';
  import KeysView from './KeysView.svelte';
  import AgentsView from './AgentsView.svelte';
  import Icon from './Icon.svelte';

  const visible = $derived(store.settings?.sidebar_visible ?? true);

  function select(section: typeof ui.sidebarSection): void {
    if (ui.sidebarSection === section && visible) {
      void saveSettings({ sidebar_visible: false });
    } else {
      ui.sidebarSection = section;
      if (!visible) void saveSettings({ sidebar_visible: true });
    }
  }
</script>

<nav class="rail" aria-label="Разделы">
  <button class="icon" class:active={visible && ui.sidebarSection === 'hosts'} type="button" title="Серверы" onclick={() => select('hosts')}>
    <Icon name="server" size={18} />
  </button>
  <button class="icon" class:active={visible && ui.sidebarSection === 'snippets'} type="button" title="Сниппеты" onclick={() => select('snippets')}>
    <Icon name="code" size={18} />
  </button>
  <button class="icon" class:active={visible && ui.sidebarSection === 'keys'} type="button" title="Ключи" onclick={() => select('keys')}>
    <Icon name="key" size={18} />
  </button>
  <button class="icon" class:active={visible && ui.sidebarSection === 'agents'} type="button" title="Агенты Claude" onclick={() => select('agents')}>
    <Icon name="agent" size={18} />
  </button>
  <span class="spacer"></span>
  <button class="icon" type="button" title="Настройки" onclick={() => (ui.settingsOpen = true)}>
    <Icon name="settings" size={18} />
  </button>
</nav>

{#if visible}
  <aside class="panel">
    {#if ui.sidebarSection === 'hosts'}
      <HostList />
    {:else if ui.sidebarSection === 'snippets'}
      <SnippetsView />
    {:else if ui.sidebarSection === 'agents'}
      <AgentsView />
    {:else}
      <KeysView />
    {/if}
  </aside>
{/if}

<style>
  .rail {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 4px;
    width: 42px;
    padding: 8px 0;
    background: var(--bg-panel);
    border-right: 1px solid var(--border);
  }
  .rail .icon {
    width: 32px;
    height: 32px;
  }
  .spacer {
    flex: 1;
  }
  .panel {
    display: flex;
    flex-direction: column;
    width: 270px;
    min-width: 200px;
    background: var(--bg-panel);
    border-right: 1px solid var(--border);
    overflow: hidden;
  }
</style>
