const CSI_FINAL_KEYS: Record<string, string> = {
  ArrowUp: 'A',
  ArrowDown: 'B',
  ArrowRight: 'C',
  ArrowLeft: 'D',
  Home: 'H',
  End: 'F',
};

const CSI_TILDE_KEYS: Record<string, number> = {
  Insert: 2,
  Delete: 3,
  PageUp: 5,
  PageDown: 6,
  F5: 15,
  F6: 17,
  F7: 18,
  F8: 19,
  F9: 20,
  F10: 21,
  F11: 23,
  F12: 24,
};

const SS3_FUNCTION_KEYS: Record<string, string> = {
  F1: 'P',
  F2: 'Q',
  F3: 'R',
  F4: 'S',
};

function modifierParameter(event: KeyboardEvent): number {
  return 1 + (event.shiftKey ? 1 : 0) + (event.altKey ? 2 : 0) + (event.ctrlKey ? 4 : 0);
}

function printableKey(event: KeyboardEvent): string {
  if (event.key?.length === 1) return event.key;
  const letter = /^Key([A-Z])$/.exec(event.code || '');
  if (letter) return event.shiftKey ? letter[1]! : letter[1]!.toLowerCase();
  const digit = /^Digit([0-9])$/.exec(event.code || '');
  if (digit) return digit[1]!;
  return '';
}

function ctrlCharacter(key: string): string {
  if (!key || key.length !== 1) return '';
  const lower = key.toLowerCase();
  const code = lower.charCodeAt(0);
  if (code >= 97 && code <= 122) return String.fromCharCode(code - 96);
  if (key === ' ' || key === '@' || key === '`') return '\x00';
  if (key === '[' || key === '{') return '\x1b';
  if (key === '\\' || key === '|') return '\x1c';
  if (key === ']' || key === '}') return '\x1d';
  if (key === '^' || key === '~') return '\x1e';
  if (key === '_' || key === '?') return '\x1f';
  return '';
}

export function encodeTerminalShortcut(event: KeyboardEvent): string {
  if (event.metaKey || (!event.ctrlKey && !event.altKey)) return '';

  const modifier = modifierParameter(event);
  const final = CSI_FINAL_KEYS[event.key];
  if (final) return `\x1b[1;${modifier}${final}`;

  const tilde = CSI_TILDE_KEYS[event.key];
  if (tilde) return `\x1b[${tilde};${modifier}~`;

  const functionFinal = SS3_FUNCTION_KEYS[event.key];
  if (functionFinal) return `\x1b[1;${modifier}${functionFinal}`;

  const printable = printableKey(event);
  if (event.ctrlKey) {
    const control = ctrlCharacter(printable);
    if (control) return event.altKey ? `\x1b${control}` : control;
  }

  if (event.altKey && printable) return `\x1b${printable}`;
  if (event.altKey && event.key === 'Enter') return '\x1b\r';
  if (event.altKey && event.key === 'Backspace') return '\x1b\x7f';
  if (event.altKey && event.key === 'Tab') return '\x1b\t';
  return '';
}
