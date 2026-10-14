import { Terminal } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import { SearchAddon } from '@xterm/addon-search';
import { Unicode11Addon } from '@xterm/addon-unicode11';
import { WebglAddon } from '@xterm/addon-webgl';
import { WebLinksAddon } from '@xterm/addon-web-links';
import { readText, writeText } from '@tauri-apps/plugin-clipboard-manager';
import { openUrl } from '@tauri-apps/plugin-opener';
import type { Settings } from '../ipc';
import { themeById } from '../themes';

export interface TerminalCallbacks {
  /** User input (keyboard, paste) to send to the session. */
  onInput(data: Uint8Array): void;
  /** Terminal grid size changed. */
  onResize(cols: number, rows: number): void;
  /** The remote side reported its working directory (OSC 7 or window title). */
  onCwd?(path: string): void;
  /** Asks whether a multi-line paste should go through. */
  confirmPaste?(text: string): Promise<boolean>;
  onFocus?(): void;
}

const encoder = new TextEncoder();

/** `user@host: ~/dir` style titles set by default bash/zsh prompts. */
function cwdFromTitle(title: string): string | null {
  const m = /^[^@\s]+@[^:\s]+:\s*(.+)$/.exec(title.trim());
  if (!m) return null;
  const path = m[1].trim();
  return path.startsWith('/') || path.startsWith('~') ? path : null;
}

/** Thin wrapper around xterm.js: addons, fitting to the container, clipboard. */
export class TerminalView {
  readonly term: Terminal;
  private readonly fitAddon = new FitAddon();
  private readonly searchAddon = new SearchAddon();
  private webgl: WebglAddon | null = null;
  private visible = true;
  private readonly observer: ResizeObserver;
  private fitFrame = 0;
  private readonly disposables: { dispose(): void }[] = [];
  private settings: Settings;

  constructor(
    private readonly container: HTMLElement,
    settings: Settings,
    private readonly callbacks: TerminalCallbacks,
  ) {
    this.settings = settings;
    this.term = new Terminal({
      allowProposedApi: true,
      ...this.options(settings),
    });
    this.term.loadAddon(this.fitAddon);
    this.term.loadAddon(this.searchAddon);
    this.term.loadAddon(new Unicode11Addon());
    this.term.loadAddon(
      new WebLinksAddon((event, uri) => {
        if (event.ctrlKey || event.button === 0) openUrl(uri).catch(() => {});
      }),
    );
    this.term.unicode.activeVersion = '11';
    this.term.open(container);
    this.enableWebgl();

    this.disposables.push(
      this.term.onData((data) => this.callbacks.onInput(encoder.encode(data))),
      this.term.onBinary((data) => {
        const bytes = new Uint8Array(data.length);
        for (let i = 0; i < data.length; i++) bytes[i] = data.charCodeAt(i) & 0xff;
        this.callbacks.onInput(bytes);
      }),
      this.term.onResize(({ cols, rows }) => this.callbacks.onResize(cols, rows)),
      this.term.onSelectionChange(() => {
        if (this.settings.copy_on_select && this.term.hasSelection()) {
          writeText(this.term.getSelection()).catch(() => {});
        }
      }),
      this.term.onTitleChange((title) => {
        const cwd = cwdFromTitle(title);
        if (cwd) this.callbacks.onCwd?.(cwd);
      }),
      this.term.parser.registerOscHandler(7, (data) => {
        try {
          const url = new URL(data);
          if (url.protocol === 'file:') this.callbacks.onCwd?.(decodeURIComponent(url.pathname));
        } catch {
          // not a file URL
        }
        return true;
      }),
    );
    this.term.textarea?.addEventListener('focus', this.onFocus);

    container.addEventListener('contextmenu', this.onContextMenu);
    this.observer = new ResizeObserver(() => this.scheduleFit());
    this.observer.observe(container);
    this.fit();
  }

  private options(s: Settings) {
    return {
      fontFamily: s.font_family,
      fontSize: s.font_size,
      lineHeight: s.line_height,
      cursorStyle: s.cursor_style,
      cursorBlink: s.cursor_blink,
      scrollback: s.scrollback,
      theme: themeById(s.theme).terminal,
    } as const;
  }

  applySettings(settings: Settings): void {
    this.settings = settings;
    const o = this.options(settings);
    this.term.options.fontFamily = o.fontFamily;
    this.term.options.fontSize = o.fontSize;
    this.term.options.lineHeight = o.lineHeight;
    this.term.options.cursorStyle = o.cursorStyle;
    this.term.options.cursorBlink = o.cursorBlink;
    this.term.options.scrollback = o.scrollback;
    this.term.options.theme = o.theme;
    this.scheduleFit();
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

  /** Hidden terminals (inactive tabs) release their WebGL context. */
  setVisible(visible: boolean): void {
    if (visible === this.visible) return;
    this.visible = visible;
    if (visible) {
      this.enableWebgl();
      this.scheduleFit();
    } else {
      this.webgl?.dispose();
      this.webgl = null;
    }
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
    const text = await readText().catch(() => '');
    if (!text) return;
    if (this.settings.warn_multiline_paste && /\r|\n/.test(text.replace(/\r?\n$/, '')) && this.callbacks.confirmPaste) {
      if (!(await this.callbacks.confirmPaste(text))) return;
    }
    this.term.paste(text);
  }

  findNext(query: string): boolean {
    return this.searchAddon.findNext(query, { decorations: searchDecorations });
  }

  findPrevious(query: string): boolean {
    return this.searchAddon.findPrevious(query, { decorations: searchDecorations });
  }

  clearSearch(): void {
    this.searchAddon.clearDecorations();
    this.term.clearSelection();
  }

  dispose(): void {
    cancelAnimationFrame(this.fitFrame);
    this.observer.disconnect();
    this.container.removeEventListener('contextmenu', this.onContextMenu);
    this.term.textarea?.removeEventListener('focus', this.onFocus);
    for (const d of this.disposables) d.dispose();
    this.webgl?.dispose();
    this.term.dispose();
  }

  private enableWebgl(): void {
    if (this.webgl) return;
    try {
      const addon = new WebglAddon();
      addon.onContextLoss(() => {
        // Fall back to the built-in DOM renderer if the GPU context is lost.
        addon.dispose();
        if (this.webgl === addon) this.webgl = null;
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

  private readonly onFocus = (): void => {
    this.callbacks.onFocus?.();
  };

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

const searchDecorations = {
  matchBackground: '#eab30855',
  matchOverviewRuler: '#eab308',
  activeMatchBackground: '#f97316aa',
  activeMatchColorOverviewRuler: '#f97316',
};
