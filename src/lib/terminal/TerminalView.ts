import { Terminal, type ITheme } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import { Unicode11Addon } from '@xterm/addon-unicode11';
import { WebglAddon } from '@xterm/addon-webgl';
import { readText, writeText } from '@tauri-apps/plugin-clipboard-manager';

export interface TerminalCallbacks {
  /** User input (keyboard, paste) to send to the session. */
  onInput(data: Uint8Array): void;
  /** Terminal grid size changed. */
  onResize(cols: number, rows: number): void;
}

const theme: ITheme = {
  background: '#14161b',
  foreground: '#d8dee9',
  cursor: '#e5e7eb',
  cursorAccent: '#14161b',
  selectionBackground: '#3b82f655',
  black: '#1b1f27',
  red: '#ef4444',
  green: '#22c55e',
  yellow: '#eab308',
  blue: '#3b82f6',
  magenta: '#a855f7',
  cyan: '#06b6d4',
  white: '#d8dee9',
  brightBlack: '#4b5563',
  brightRed: '#f87171',
  brightGreen: '#4ade80',
  brightYellow: '#facc15',
  brightBlue: '#60a5fa',
  brightMagenta: '#c084fc',
  brightCyan: '#22d3ee',
  brightWhite: '#f9fafb',
};

const encoder = new TextEncoder();

/** Thin wrapper around xterm.js: addons, fitting to the container, clipboard. */
export class TerminalView {
  readonly term: Terminal;
  private readonly fitAddon = new FitAddon();
  private webgl: WebglAddon | null = null;
  private readonly observer: ResizeObserver;
  private fitFrame = 0;
  private readonly disposables: { dispose(): void }[] = [];

  constructor(
    private readonly container: HTMLElement,
    private readonly callbacks: TerminalCallbacks,
  ) {
    this.term = new Terminal({
      allowProposedApi: true,
      fontFamily: "'Cascadia Mono', Consolas, 'Courier New', monospace",
      fontSize: 14,
      cursorBlink: true,
      scrollback: 10000,
      theme,
    });
    this.term.loadAddon(this.fitAddon);
    this.term.loadAddon(new Unicode11Addon());
    this.term.unicode.activeVersion = '11';
    this.term.open(container);
    this.enableWebgl();

    this.term.attachCustomKeyEventHandler((e) => this.handleKey(e));
    this.disposables.push(
      this.term.onData((data) => this.callbacks.onInput(encoder.encode(data))),
      this.term.onBinary((data) => {
        const bytes = new Uint8Array(data.length);
        for (let i = 0; i < data.length; i++) bytes[i] = data.charCodeAt(i) & 0xff;
        this.callbacks.onInput(bytes);
      }),
      this.term.onResize(({ cols, rows }) => this.callbacks.onResize(cols, rows)),
    );

    container.addEventListener('contextmenu', this.onContextMenu);
    this.observer = new ResizeObserver(() => this.scheduleFit());
    this.observer.observe(container);
    this.fit();
  }

  get cols(): number {
    return this.term.cols;
  }

  get rows(): number {
    return this.term.rows;
  }

  write(data: Uint8Array | string): void {
    this.term.write(data);
  }

  focus(): void {
    this.term.focus();
  }

  reset(): void {
    this.term.reset();
  }

  fit(): void {
    if (this.container.clientWidth > 0 && this.container.clientHeight > 0) {
      this.fitAddon.fit();
    }
  }

  async copySelection(): Promise<boolean> {
    const text = this.term.getSelection();
    if (!text) return false;
    await writeText(text);
    return true;
  }

  async paste(): Promise<void> {
    const text = await readText();
    if (text) this.term.paste(text);
  }

  dispose(): void {
    cancelAnimationFrame(this.fitFrame);
    this.observer.disconnect();
    this.container.removeEventListener('contextmenu', this.onContextMenu);
    for (const d of this.disposables) d.dispose();
    this.webgl?.dispose();
    this.term.dispose();
  }

  private enableWebgl(): void {
    try {
      const addon = new WebglAddon();
      addon.onContextLoss(() => {
        // Fall back to the built-in DOM renderer if the GPU context is lost.
        addon.dispose();
        this.webgl = null;
      });
      this.term.loadAddon(addon);
      this.webgl = addon;
    } catch {
      this.webgl = null;
    }
  }

  private scheduleFit(): void {
    cancelAnimationFrame(this.fitFrame);
    this.fitFrame = requestAnimationFrame(() => this.fit());
  }

  /** Returns false for keys the terminal must not process itself. */
  private handleKey(e: KeyboardEvent): boolean {
    const ctrlShift = e.ctrlKey && e.shiftKey && !e.altKey && !e.metaKey;
    if (ctrlShift && (e.code === 'KeyC' || e.code === 'KeyV')) {
      e.preventDefault();
      if (e.type === 'keydown') {
        if (e.code === 'KeyC') void this.copySelection();
        else void this.paste();
      }
      return false;
    }
    return true;
  }

  /** Right click: copy the selection if there is one, otherwise paste. */
  private readonly onContextMenu = (e: MouseEvent): void => {
    e.preventDefault();
    void (async () => {
      if (await this.copySelection()) {
        this.term.clearSelection();
      } else {
        await this.paste();
      }
      this.term.focus();
    })();
  };
}
