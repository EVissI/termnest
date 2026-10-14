import type { LayoutNode, Pane, PaneId, SplitId, Direction } from '../ipc';

/** Rectangle in fractions of the tab area (0..1). */
export interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface PaneBox {
  pane: Pane;
  rect: Rect;
}

export interface DividerBox {
  split: SplitId;
  direction: Direction;
  /** The split's whole area; the divider sits at `ratio` inside it. */
  area: Rect;
  ratio: number;
}

/** Flattens the split tree into absolute pane and divider rectangles. */
export function computeBoxes(root: LayoutNode): { panes: PaneBox[]; dividers: DividerBox[] } {
  const panes: PaneBox[] = [];
  const dividers: DividerBox[] = [];
  const walk = (node: LayoutNode, r: Rect) => {
    if (node.type === 'pane') {
      panes.push({ pane: node, rect: r });
      return;
    }
    const ratio = node.ratio;
    if (node.direction === 'horizontal') {
      const w1 = r.w * ratio;
      walk(node.first, { x: r.x, y: r.y, w: w1, h: r.h });
      walk(node.second, { x: r.x + w1, y: r.y, w: r.w - w1, h: r.h });
    } else {
      const h1 = r.h * ratio;
      walk(node.first, { x: r.x, y: r.y, w: r.w, h: h1 });
      walk(node.second, { x: r.x, y: r.y + h1, w: r.w, h: r.h - h1 });
    }
    dividers.push({ split: node.id, direction: node.direction, area: r, ratio });
  };
  walk(root, { x: 0, y: 0, w: 1, h: 1 });
  return { panes, dividers };
}

export function allPanes(root: LayoutNode): Pane[] {
  return computeBoxes(root).panes.map((b) => b.pane);
}

export type Side = 'left' | 'right' | 'up' | 'down';

/** The pane next to `from` in the given direction, preferring the most overlapping one. */
export function neighbor(root: LayoutNode, from: PaneId, side: Side): PaneId | null {
  const { panes } = computeBoxes(root);
  const src = panes.find((p) => p.pane.id === from);
  if (!src) return null;
  const a = src.rect;
  const eps = 1e-6;
  let best: { id: PaneId; overlap: number; dist: number } | null = null;
  for (const { pane, rect: b } of panes) {
    if (pane.id === from) continue;
    let dist: number;
    let overlap: number;
    if (side === 'left' || side === 'right') {
      dist = side === 'left' ? a.x - (b.x + b.w) : b.x - (a.x + a.w);
      overlap = Math.min(a.y + a.h, b.y + b.h) - Math.max(a.y, b.y);
    } else {
      dist = side === 'up' ? a.y - (b.y + b.h) : b.y - (a.y + a.h);
      overlap = Math.min(a.x + a.w, b.x + b.w) - Math.max(a.x, b.x);
    }
    if (dist < -eps || overlap <= eps) continue;
    if (!best || dist < best.dist - eps || (Math.abs(dist - best.dist) <= eps && overlap > best.overlap)) {
      best = { id: pane.id, overlap, dist };
    }
  }
  return best?.id ?? null;
}
