/** Variable names in order of first appearance: `{{name}}` (same rules as the Rust core). */
export function variables(command: string): string[] {
  const out: string[] = [];
  const re = /\{\{([^{}]*?)\}\}/g;
  for (const m of command.matchAll(re)) {
    const name = m[1].trim();
    if (name && !out.includes(name)) out.push(name);
  }
  return out;
}

/** Substitutes `{{name}}` with values; unknown variables are left as they are. */
export function render(command: string, values: Record<string, string>): string {
  return command.replace(/\{\{([^{}]*?)\}\}/g, (whole, raw: string) => {
    const name = raw.trim();
    return name in values ? values[name] : whole;
  });
}

/** Text to type into the terminal: newlines become Enter (CR), optional final Enter. */
export function toTerminalInput(text: string, pressEnter: boolean): string {
  const body = text.replace(/\r?\n/g, '\r');
  return pressEnter ? `${body}\r` : body;
}
