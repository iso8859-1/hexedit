// Shared editor state and a tiny event bus.

export const state = {
  info: null,          // DocInfo from the backend
  size: 0,
  cursor: 0,           // byte offset, may equal size (append position)
  anchor: null,        // selection anchor, null = no selection
  nibble: 0,           // 0 = high, 1 = low nibble of the cursor byte (hex pane)
  pane: 'hex',         // 'hex' | 'text'
  insert: false,
  bpr: 16,
  littleEndian: true,
  encoding: 'local',
  charTable: [],       // display char per byte value ('' = not printable)
  reverseTable: new Map(),
  utf8: false,         // selected code page is UTF-8
  highlight: null,     // {start, len} temporary highlight (inspector/tree hover)
  fieldRanges: [],     // sorted, non-overlapping [{s, e, c}] from the applied template
};

const listeners = {};

export function on(evt, fn) {
  (listeners[evt] ??= []).push(fn);
}

export function emit(evt, arg) {
  for (const fn of listeners[evt] ?? []) fn(arg);
}

/** Inclusive selection range; len 0 when nothing is selected. */
export function selection() {
  if (state.anchor === null || state.size === 0) {
    return { start: state.cursor, end: state.cursor, len: 0 };
  }
  const start = Math.min(state.anchor, state.cursor);
  const end = Math.min(Math.max(state.anchor, state.cursor), state.size - 1);
  return { start, end, len: Math.max(0, end - start + 1) };
}

export function hasSelection() {
  return state.anchor !== null && state.anchor !== state.cursor;
}

export const hex = (n, w = 8) => n.toString(16).toUpperCase().padStart(w, '0');

export function store(key, value) {
  try { localStorage.setItem(key, JSON.stringify(value)); } catch { /* storage unavailable */ }
}

export function load(key, fallback) {
  try {
    const v = localStorage.getItem(key);
    return v === null ? fallback : JSON.parse(v);
  } catch {
    return fallback;
  }
}

/** Parses "0x1F", "1Fh", "496", "+16" or "-0x10" (relative to `base`). */
export function parseOffset(text, base = 0) {
  let t = text.trim().replace(/[_\s]/g, '');
  let rel = 0;
  if (t.startsWith('+')) { rel = 1; t = t.slice(1); } else if (t.startsWith('-')) { rel = -1; t = t.slice(1); }
  let v;
  if (/^0x[0-9a-f]+$/i.test(t)) v = parseInt(t.slice(2), 16);
  else if (/^[0-9a-f]+h$/i.test(t)) v = parseInt(t.slice(0, -1), 16);
  else if (/^\d+$/.test(t)) v = parseInt(t, 10);
  else if (/^[0-9a-f]+$/i.test(t)) v = parseInt(t, 16);
  else return null;
  return rel === 0 ? v : base + rel * v;
}
