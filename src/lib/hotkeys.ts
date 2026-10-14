/** Actions that can be bound to keys, with their default combinations (SPEC section 6). */
export const hotkeyActions = [
  { id: 'tab.new', label: 'Новая вкладка (выбор сервера)', combo: 'Ctrl+Shift+T' },
  { id: 'pane.close', label: 'Закрыть панель/вкладку', combo: 'Ctrl+Shift+W' },
  { id: 'window.new', label: 'Новое окно', combo: 'Ctrl+Shift+N' },
  { id: 'split.right', label: 'Сплит вправо', combo: 'Alt+Shift+=' },
  { id: 'split.down', label: 'Сплит вниз', combo: 'Alt+Shift+-' },
  { id: 'focus.left', label: 'Фокус: панель слева', combo: 'Alt+Left' },
  { id: 'focus.right', label: 'Фокус: панель справа', combo: 'Alt+Right' },
  { id: 'focus.up', label: 'Фокус: панель сверху', combo: 'Alt+Up' },
  { id: 'focus.down', label: 'Фокус: панель снизу', combo: 'Alt+Down' },
  { id: 'tab.next', label: 'Следующая вкладка', combo: 'Ctrl+Tab' },
  { id: 'tab.prev', label: 'Предыдущая вкладка', combo: 'Ctrl+Shift+Tab' },
  { id: 'session.duplicate', label: 'Дублировать сессию', combo: 'Ctrl+Shift+D' },
  { id: 'snippets.palette', label: 'Палитра сниппетов', combo: 'Ctrl+Shift+P' },
  { id: 'terminal.search', label: 'Поиск в терминале', combo: 'Ctrl+Shift+F' },
  { id: 'terminal.copy', label: 'Копировать', combo: 'Ctrl+Shift+C' },
  { id: 'terminal.paste', label: 'Вставить', combo: 'Ctrl+Shift+V' },
  { id: 'broadcast.toggle', label: 'Режим broadcast', combo: 'Ctrl+Shift+B' },
  { id: 'sftp.toggle', label: 'Панель SFTP', combo: 'Ctrl+Shift+S' },
  { id: 'sidebar.toggle', label: 'Боковая панель', combo: 'Ctrl+Shift+E' },
] as const;

export type HotkeyAction = (typeof hotkeyActions)[number]['id'];

const codeNames: Record<string, string> = {
  Equal: '=',
  Minus: '-',
  BracketLeft: '[',
  BracketRight: ']',
  Semicolon: ';',
  Quote: "'",
  Comma: ',',
  Period: '.',
  Slash: '/',
  Backslash: '\\',
  Backquote: '`',
  ArrowLeft: 'Left',
  ArrowRight: 'Right',
  ArrowUp: 'Up',
  ArrowDown: 'Down',
  NumpadAdd: '=',
  NumpadSubtract: '-',
};

/** Physical key name (layout independent, so Cyrillic layouts work the same). */
function keyName(e: KeyboardEvent): string | null {
  const code = e.code;
  if (code.startsWith('Key')) return code.slice(3);
  if (code.startsWith('Digit')) return code.slice(5);
  if (/^F\d{1,2}$/.test(code)) return code;
  if (code in codeNames) return codeNames[code];
  if (['Tab', 'Enter', 'Space', 'Escape', 'Backspace', 'Delete', 'Insert', 'Home', 'End', 'PageUp', 'PageDown'].includes(code)) {
    return code;
  }
  return null;
}

/** "Ctrl+Shift+T" for a key event, or null for bare modifiers. */
export function comboOf(e: KeyboardEvent): string | null {
  const key = keyName(e);
  if (!key) return null;
  const parts: string[] = [];
  if (e.ctrlKey) parts.push('Ctrl');
  if (e.altKey) parts.push('Alt');
  if (e.shiftKey) parts.push('Shift');
  if (e.metaKey) parts.push('Meta');
  parts.push(key);
  return parts.join('+');
}

export function normalizeCombo(combo: string): string {
  const parts = combo.split('+').map((p) => p.trim()).filter(Boolean);
  const key = parts.pop() ?? '';
  const order = ['Ctrl', 'Alt', 'Shift', 'Meta'];
  const mods = order.filter((m) => parts.some((p) => p.toLowerCase() === m.toLowerCase()));
  return [...mods, key.length === 1 ? key.toUpperCase() : key].join('+');
}

/** Effective bindings: defaults with user overrides applied. Empty override = unbound. */
export function bindings(overrides: Record<string, string>): Map<string, HotkeyAction> {
  const map = new Map<string, HotkeyAction>();
  for (const a of hotkeyActions) {
    const combo = a.id in overrides ? overrides[a.id] : a.combo;
    if (combo) map.set(normalizeCombo(combo), a.id);
  }
  return map;
}

export function comboFor(action: HotkeyAction, overrides: Record<string, string>): string {
  if (action in overrides) return overrides[action];
  return hotkeyActions.find((a) => a.id === action)?.combo ?? '';
}
