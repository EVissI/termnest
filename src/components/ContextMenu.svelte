<script lang="ts" module>
  export interface MenuItem {
    label: string;
    action?: () => void;
    danger?: boolean;
    disabled?: boolean;
    separator?: boolean;
    children?: MenuItem[];
    /** Small color swatch shown before the label. */
    swatch?: string;
  }
</script>

<script lang="ts">
  import { onMount } from 'svelte';
  import Icon from './Icon.svelte';

  let { x, y, items, onclose }: { x: number; y: number; items: MenuItem[]; onclose: () => void } = $props();

  let menu: HTMLDivElement;
  let left = $state(0);
  let top = $state(0);
  let openChild = $state<number | null>(null);

  onMount(() => {
    // Keep the menu inside the window.
    const r = menu.getBoundingClientRect();
    left = Math.min(x, window.innerWidth - r.width - 4);
    top = Math.min(y, window.innerHeight - r.height - 4);
    const close = (e: Event) => {
      if (!menu.contains(e.target as Node)) onclose();
    };
    const key = (e: KeyboardEvent) => e.key === 'Escape' && onclose();
    setTimeout(() => {
      window.addEventListener('mousedown', close, true);
      window.addEventListener('blur', onclose);
    });
    window.addEventListener('keydown', key, true);
    return () => {
      window.removeEventListener('mousedown', close, true);
      window.removeEventListener('blur', onclose);
      window.removeEventListener('keydown', key, true);
    };
  });

  function run(item: MenuItem): void {
    if (item.disabled || item.children) return;
    onclose();
    item.action?.();
  }
</script>

<div class="menu" bind:this={menu} style:left="{left || x}px" style:top="{top || y}px" role="menu">
  {#each items as item, i (i)}
    {#if item.separator}
      <div class="sep"></div>
    {:else}
      <div class="item-wrap" role="none" onmouseenter={() => (openChild = item.children ? i : null)}>
        <button
          type="button"
          role="menuitem"
          class="item"
          class:danger={item.danger}
          disabled={item.disabled}
          onclick={() => run(item)}
        >
          {#if item.swatch !== undefined}
            <span class="swatch" style:background={item.swatch || 'transparent'}></span>
          {/if}
          <span class="label">{item.label}</span>
          {#if item.children}<Icon name="chevron-right" size={12} />{/if}
        </button>
        {#if item.children && openChild === i}
          <div class="submenu" role="menu">
            {#each item.children as child, j (j)}
              <button type="button" role="menuitem" class="item" disabled={child.disabled} onclick={() => run(child)}>
                {#if child.swatch !== undefined}
                  <span class="swatch" style:background={child.swatch || 'transparent'}></span>
                {/if}
                <span class="label">{child.label}</span>
              </button>
            {/each}
          </div>
        {/if}
      </div>
    {/if}
  {/each}
</div>

<style>
  .menu,
  .submenu {
    position: fixed;
    z-index: 200;
    min-width: 180px;
    padding: 4px;
    background: var(--bg-panel);
    border: 1px solid var(--border);
    border-radius: 6px;
    box-shadow: 0 8px 24px rgb(0 0 0 / 35%);
  }
  .submenu {
    position: absolute;
    left: calc(100% - 4px);
    top: -4px;
  }
  .item-wrap {
    position: relative;
  }
  .item {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 5px 8px;
    border: none;
    background: none;
    border-radius: 4px;
    text-align: left;
    cursor: pointer;
  }
  .item:hover:not(:disabled) {
    background: var(--bg-hover);
  }
  .item:disabled {
    opacity: 0.45;
    cursor: default;
  }
  .label {
    flex: 1;
  }
  .danger {
    color: var(--err);
  }
  .sep {
    height: 1px;
    margin: 4px 2px;
    background: var(--border);
  }
  .swatch {
    width: 10px;
    height: 10px;
    border-radius: 50%;
    border: 1px solid var(--border);
  }
</style>
