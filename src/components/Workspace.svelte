<script lang="ts">
  import { api, type Tab } from '../lib/ipc';
  import { computeBoxes, type DividerBox } from '../lib/layout/geometry';
  import { myWindow } from '../stores/app.svelte';
  import TerminalPane from './TerminalPane.svelte';
  import PanePicker from './PanePicker.svelte';

  const win = $derived(myWindow());
  const tabs = $derived(win?.tabs ?? []);
  const activeId = $derived(win?.active ?? tabs[0]?.id ?? null);

  function pct(v: number): string {
    return `${(v * 100).toFixed(4)}%`;
  }

  // Divider dragging: ratio updates are sent once per animation frame.
  let frame = 0;
  function startDrag(e: PointerEvent, tab: Tab, d: DividerBox): void {
    e.preventDefault();
    const handle = e.currentTarget as HTMLElement;
    const area = handle.parentElement!.getBoundingClientRect();
    handle.setPointerCapture(e.pointerId);
    const move = (ev: PointerEvent) => {
      const ratio =
        d.direction === 'horizontal'
          ? ((ev.clientX - area.left) / area.width - d.area.x) / d.area.w
          : ((ev.clientY - area.top) / area.height - d.area.y) / d.area.h;
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => {
        api.splitSetRatio(tab.id, d.split, Math.min(0.95, Math.max(0.05, ratio))).catch(() => {});
      });
    };
    const up = () => {
      handle.removeEventListener('pointermove', move);
      handle.removeEventListener('pointerup', up);
      handle.removeEventListener('pointercancel', up);
    };
    handle.addEventListener('pointermove', move);
    handle.addEventListener('pointerup', up);
    handle.addEventListener('pointercancel', up);
  }
</script>

<div class="workspace">
  {#if tabs.length === 0}
    <div class="empty-window">
      <PanePicker />
    </div>
  {/if}
  {#each tabs as tab (tab.id)}
    {@const boxes = computeBoxes(tab.root)}
    {@const visible = tab.id === activeId}
    <div class="tab" class:hidden={!visible}>
      {#each boxes.panes as box (box.pane.id)}
        <div
          class="slot"
          style:left={pct(box.rect.x)}
          style:top={pct(box.rect.y)}
          style:width={pct(box.rect.w)}
          style:height={pct(box.rect.h)}
        >
          <TerminalPane pane={box.pane} {visible} focused={tab.focused === box.pane.id} />
        </div>
      {/each}
      {#each boxes.dividers as d (d.split)}
        <div
          class="divider {d.direction}"
          role="separator"
          aria-orientation={d.direction === 'horizontal' ? 'vertical' : 'horizontal'}
          style:left={d.direction === 'horizontal' ? pct(d.area.x + d.area.w * d.ratio) : pct(d.area.x)}
          style:top={d.direction === 'vertical' ? pct(d.area.y + d.area.h * d.ratio) : pct(d.area.y)}
          style:width={d.direction === 'vertical' ? pct(d.area.w) : undefined}
          style:height={d.direction === 'horizontal' ? pct(d.area.h) : undefined}
          onpointerdown={(e) => startDrag(e, tab, d)}
        ></div>
      {/each}
    </div>
  {/each}
</div>

<style>
  .workspace {
    position: relative;
    flex: 1;
    min-width: 0;
    min-height: 0;
  }
  .tab,
  .empty-window {
    position: absolute;
    inset: 0;
  }
  .tab.hidden {
    visibility: hidden;
    pointer-events: none;
  }
  .slot {
    position: absolute;
  }
  .divider {
    position: absolute;
    z-index: 8;
  }
  .divider.horizontal {
    width: 7px;
    margin-left: -3px;
    cursor: col-resize;
  }
  .divider.vertical {
    height: 7px;
    margin-top: -3px;
    cursor: row-resize;
  }
  .divider:hover,
  .divider:active {
    background: color-mix(in srgb, var(--accent) 40%, transparent);
  }
</style>
