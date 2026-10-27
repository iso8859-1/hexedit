import { api, listen } from './api.js';
import { state, on, emit, selection, hasSelection, hex, load, store, parseOffset } from './state.js';
import { HexView } from './hexview.js';
import { Inspector } from './inspector.js';
import { TemplatePanel } from './template.js';

const $ = (id) => document.getElementById(id);
const hexEl = $('hexview');
let view;

// ------------------------------------------------------------------ toast

let toastTimer;
function toast(msg, error = false) {
  const t = $('toast');
  t.textContent = msg;
  t.className = error ? 'error' : '';
  t.hidden = false;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => { t.hidden = true; }, error ? 6000 : 2500);
}

// ------------------------------------------------------------ document

function setInfo(info) {
  state.info = info;
  state.size = info.size;
  if (state.cursor > state.size) state.cursor = state.size;
  if (state.anchor !== null && state.anchor > state.size) state.anchor = state.size;
  $('btn-undo').disabled = !info.canUndo;
  $('btn-redo').disabled = !info.canRedo;
  updateStatus();
}

/** Edits are serialised so fast typing applies in order. */
let queue = Promise.resolve();
function edit(fn) {
  const p = queue.then(async () => {
    const info = await fn();
    if (info) {
      setInfo(info);
      view.invalidate();
      emit('data');
    }
    return info;
  });
  queue = p.catch((e) => toast(String(e), true));
  return queue;
}

function docLoaded(info) {
  navReset();
  setInfo(info);
  remember(info.path);
  state.cursor = 0;
  state.anchor = null;
  state.nibble = 0;
  view.topRow = 0;
  view.layoutChanged();
  emit('doc');
  emit('cursor');
  emit('data');
}

async function openFile(path = null) {
  try {
    const info = await api.openFile(path);
    if (info) docLoaded(info);
  } catch (e) {
    toast(String(e), true);
  }
}

// ------------------------------------------------------------ recent files

const RECENT_MAX = 10;
const RECENT_CLEAR = '\u0000clear';
// Windows paths are case-insensitive
const samePath = (a, b) => a.toLowerCase() === b.toLowerCase();

/** Moves `path` to the top of the recent files list. */
function remember(path) {
  if (!path) return;
  const list = load('ui.recent', []).filter((p) => !samePath(p, path));
  store('ui.recent', [path, ...list].slice(0, RECENT_MAX));
  fillRecentMenu();
}

function forget(path) {
  store('ui.recent', load('ui.recent', []).filter((p) => !samePath(p, path)));
  fillRecentMenu();
}

function fillRecentMenu() {
  const sel = $('sel-recent');
  const list = load('ui.recent', []);
  sel.replaceChildren(new Option('Recent…', ''));
  for (const [i, p] of list.entries()) {
    const parts = p.split(/[\\/]/);
    const name = parts.pop();
    const o = new Option(`${i < 9 ? `${i + 1}  ` : ''}${name}  —  ${parts.join('\\')}`, p);
    o.title = p;
    sel.append(o);
  }
  if (list.length) {
    const actions = document.createElement('optgroup');
    actions.label = '──────────';
    actions.append(new Option('Clear list', RECENT_CLEAR));
    sel.append(actions);
  }
  sel.disabled = !list.length;
  sel.title = list.length ? 'Recent files' : 'No recent files';
}

function initRecent() {
  const sel = $('sel-recent');
  fillRecentMenu();
  sel.addEventListener('change', async () => {
    const path = sel.value;
    sel.value = '';
    hexEl.focus();
    if (path === RECENT_CLEAR) {
      store('ui.recent', []);
      fillRecentMenu();
    } else if (path) {
      try {
        const info = await api.openFile(path);
        if (info) docLoaded(info);
      } catch (e) {
        // usually moved or deleted: drop it so the list stays useful
        forget(path);
        toast(`${e} (removed from recent files)`, true);
      }
    }
  });
}

async function newFile() {
  try {
    const info = await api.newFile();
    if (info) docLoaded(info);
  } catch (e) {
    toast(String(e), true);
  }
}

async function saveFile(saveAs) {
  await queue;
  try {
    const r = await api.saveFile(saveAs, $('opt-backup').checked);
    if (r) {
      setInfo(r.info);
      remember(r.info.path);
      const backup = r.backup ? ` (original kept as ${r.backup.split(/[\\/]/).pop()})` : '';
      toast(`Saved ${r.info.path}${backup}`);
    }
  } catch (e) {
    toast(String(e), true);
  }
}

async function undoRedo(redo) {
  await edit(async () => {
    const r = redo ? await api.redo() : await api.undo();
    if (r.offset !== null) {
      // select what changed (or put the cursor where it happened)
      state.anchor = r.len > 1 ? r.offset : null;
      state.cursor = Math.min(r.len > 1 ? r.offset + r.len - 1 : r.offset, r.info.size);
      state.nibble = 0;
    }
    return r.info;
  });
  view.scrollTo(state.cursor, true);
  emit('cursor');
}

// ------------------------------------------------------------ context menu

const DECOMPRESS = [
  ['auto', 'Open decompressed'],
  ['zlib', 'Open decompressed as zlib'],
  ['gzip', 'Open decompressed as gzip'],
  ['deflate', 'Open decompressed as raw deflate'],
  ['zstd', 'Open decompressed as zstd'],
  ['lz4', 'Open decompressed as LZ4 frame'],
];

/** Decompresses the selection into a new window. */
async function decompressSelection(format) {
  const sel = selection();
  toast('Decompressing…');
  try {
    const r = await api.decompress(sel.start, sel.len, format);
    const opened = `Opened ${r.size.toLocaleString()} bytes (${r.format}) in a new window`;
    if (r.error) toast(`${opened}, incomplete: ${r.error}`, true);
    else if (r.consumed < sel.len) toast(`${opened}; the last ${(sel.len - r.consumed).toLocaleString()} selected bytes are not part of the stream`);
    else toast(opened);
  } catch (e) {
    toast(String(e), true);
  }
}

function hideMenu() {
  $('ctxmenu').hidden = true;
}

/**
 * Shows a menu at x, y. Items are {label, action} with optional `disabled`,
 * `current` (highlighted, not clickable) and `detail` (muted text on the
 * right); `hint` is an optional first line.
 */
function openMenu(x, y, items, hint = null) {
  const menu = $('ctxmenu');
  menu.replaceChildren();
  if (hint) {
    const h = document.createElement('div');
    h.className = 'hint';
    h.textContent = hint;
    menu.append(h);
  }
  for (const item of items) {
    const b = document.createElement('button');
    b.setAttribute('role', 'menuitem');
    b.textContent = item.label;
    if (item.detail) {
      const d = document.createElement('span');
      d.className = 'detail';
      d.textContent = item.detail;
      b.append(d);
    }
    b.disabled = item.disabled || item.current;
    b.classList.toggle('current', !!item.current);
    b.onclick = () => { hideMenu(); item.action(); };
    menu.append(b);
  }
  menu.hidden = false;
  // keep it inside the window
  const r = menu.getBoundingClientRect();
  menu.style.left = `${Math.max(0, Math.min(x, innerWidth - r.width - 4))}px`;
  menu.style.top = `${Math.max(0, Math.min(y, innerHeight - r.height - 4))}px`;
  menu.querySelector('button:not(:disabled)')?.focus();
}

function showDecompressMenu(x, y) {
  const has = state.anchor !== null && selection().len > 1;
  const items = DECOMPRESS.map(([format, label]) => ({
    label: `${label}${format === 'auto' ? ' (detect format)' : ''}`,
    disabled: !has,
    action: () => decompressSelection(format),
  }));
  openMenu(x, y, items, has ? null : 'Select the compressed bytes first');
}

function initMenu() {
  hexEl.addEventListener('contextmenu', (e) => {
    e.preventDefault();
    showDecompressMenu(e.clientX, e.clientY);
  });
  on('menu', ({ x, y }) => showDecompressMenu(x, y));
  const menu = $('ctxmenu');
  document.addEventListener('mousedown', (e) => { if (!menu.contains(e.target)) hideMenu(); }, true);
  document.addEventListener('wheel', hideMenu, true);
  window.addEventListener('blur', hideMenu);
  menu.addEventListener('keydown', (e) => {
    const items = [...menu.querySelectorAll('button:not(:disabled)')];
    const i = items.indexOf(document.activeElement);
    if (e.key === 'Escape') { e.preventDefault(); hideMenu(); hexEl.focus(); }
    else if (e.key === 'ArrowDown') { e.preventDefault(); items[(i + 1) % items.length]?.focus(); }
    else if (e.key === 'ArrowUp') { e.preventDefault(); items[(i - 1 + items.length) % items.length]?.focus(); }
  });
}

// ------------------------------------------------------ navigation history

// Locations to go back and forward to, as in a browser. Go to, find results and
// followed links are always recorded; other cursor moves only when they leave
// the part of the file that was visible.
const NAV_MAX = 50;
const nav = {
  back: [],       // older locations, the most recent last
  forward: [],    // locations left by going back, the nearest last
  here: null,     // {cursor, anchor, label}: the current location
  pending: null,  // {label, force} for the cursor move about to happen
  moving: false,  // moving through the history itself
};

/** Describes the next cursor move for the history; `force` records it whatever the distance. */
function navNote(label, force = false) {
  nav.pending = { label, force };
}

function onNavCursor() {
  const p = nav.pending;
  nav.pending = null;
  const prev = nav.here;
  const now = { cursor: state.cursor, anchor: state.anchor };
  if (nav.moving || !prev) {
    nav.here = { ...now, label: prev?.label ?? 'Start' };
  } else if (now.cursor !== prev.cursor
    && (p?.force || Math.abs(now.cursor - prev.cursor) >= view.visibleRows * state.bpr)) {
    nav.back.push(prev);
    if (nav.back.length > NAV_MAX) nav.back.shift();
    nav.forward = [];
    nav.here = { ...now, label: p?.label ?? 'Cursor' };
  } else {
    // a small move: still the same place, now with this selection
    nav.here = { ...now, label: p?.label ?? prev.label };
  }
  updateNavButtons();
}

/** Goes `n` steps back (negative) or forward (positive). */
function navGo(n) {
  const from = n < 0 ? nav.back : nav.forward;
  const to = n < 0 ? nav.forward : nav.back;
  if (n === 0 || from.length < Math.abs(n)) return;
  let loc = nav.here;
  for (let i = 0; i < Math.abs(n); i++) {
    to.push(loc);
    loc = from.pop();
  }
  nav.here = loc;
  // edits since then may have made the file shorter
  state.cursor = Math.min(loc.cursor, state.size);
  state.anchor = loc.anchor === null ? null : Math.min(loc.anchor, state.size);
  state.nibble = 0;
  nav.moving = true;
  try {
    view.scrollTo(state.cursor, true);
    emit('cursor');
  } finally {
    nav.moving = false;
  }
  hexEl.focus();
}

function navReset() {
  nav.back = [];
  nav.forward = [];
  nav.here = null;
}

function updateNavButtons() {
  $('btn-back').disabled = !nav.back.length;
  $('btn-forward').disabled = !nav.forward.length;
  $('btn-history').disabled = !nav.back.length && !nav.forward.length;
}

/** The visited locations, newest at the top, the current one highlighted. */
function showHistoryMenu() {
  const entry = (loc, steps) => {
    const len = loc.anchor === null ? 0 : Math.abs(loc.cursor - loc.anchor) + 1;
    return {
      label: `0x${hex(loc.cursor)}  ${loc.label}`,
      detail: len > 1 ? `${len.toLocaleString()} bytes` : '',
      current: steps === 0,
      action: () => navGo(steps),
    };
  };
  const items = [
    ...nav.forward.map((loc, i) => entry(loc, nav.forward.length - i)),
    entry(nav.here, 0),
    ...nav.back.map((loc, i) => entry(loc, i - nav.back.length)).reverse(),
  ];
  const r = $('btn-history').getBoundingClientRect();
  openMenu(r.left, r.bottom + 2, items);
}

function initNav() {
  on('cursor', onNavCursor);
  $('btn-back').onclick = () => navGo(-1);
  $('btn-forward').onclick = () => navGo(1);
  $('btn-history').onclick = showHistoryMenu;
  // the mouse's back and forward buttons
  hexEl.addEventListener('mouseup', (e) => {
    if (e.button === 3 || e.button === 4) {
      e.preventDefault();
      navGo(e.button === 3 ? -1 : 1);
    }
  });
  updateNavButtons();
}

// ------------------------------------------------------------ cursor

function moveCursor(to, extend) {
  to = Math.max(0, Math.min(state.size, to));
  if (extend) {
    if (state.anchor === null) state.anchor = state.cursor;
  } else {
    state.anchor = null;
  }
  state.cursor = to;
  state.nibble = 0;
  view.scrollTo(to);
  emit('cursor');
}

/**
 * Selects `len` bytes from `start`, with the cursor on the last byte, or on the
 * first with `cursorAtStart`. `jump` names the location for the navigation
 * history, and `force` records it however close it is.
 */
function selectRange(start, len, { cursorAtStart = false, jump = null, force = false } = {}) {
  if (jump) navNote(jump, force);
  const end = Math.max(start, start + len - 1);
  state.anchor = cursorAtStart ? end : start;
  state.cursor = cursorAtStart ? start : end;
  state.nibble = 0;
  view.scrollTo(end);
  view.scrollTo(start);
  emit('cursor');
}

function updateStatus() {
  const info = state.info;
  if (!info) return;
  const sel = selection();
  $('st-file').textContent = (info.path ?? info.name) + (info.dirty ? ' ●' : '');
  $('st-offset').textContent = `Offset 0x${hex(state.cursor)} (${state.cursor})`;
  $('st-sel').textContent = hasSelection()
    ? `Selection 0x${hex(sel.start)}–0x${hex(sel.end)} (${sel.len} bytes)`
    : '';
  $('st-size').textContent = `Size ${state.size.toLocaleString()} bytes`;
  $('st-mode').textContent = state.insert ? 'INS' : 'OVR';
  hexEl.classList.toggle('ins', state.insert);
}

// ------------------------------------------------------------- editing

let pendingByte = 0;

// Cursor and selection are read inside the queued function: earlier
// keystrokes may still be in flight and move the cursor.
function typeHexDigit(n) {
  edit(async () => {
    const o = state.cursor;
    if (o > state.size) return null;
    let info;
    if (state.nibble === 0) {
      const insert = state.insert || o === state.size;
      let val = n << 4;
      if (!insert) {
        await view.ensure(o, o + 1);
        val |= (view.byteAt(o) ?? 0) & 0x0F;
      }
      info = await api.writeBytes(o, [val], insert, true);
      pendingByte = val;
      state.nibble = 1;
    } else {
      const val = (pendingByte & 0xF0) | n;
      info = await api.writeBytes(o, [val], false, true);
      state.cursor = o + 1;
      state.nibble = 0;
    }
    state.anchor = null;
    return info;
  }).then(() => {
    view.scrollTo(state.cursor);
    emit('cursor');
  });
}

function encodeText(text) {
  if (state.utf8) return new TextEncoder().encode(text);
  const out = [];
  for (const ch of text) {
    let b = state.reverseTable.get(ch);
    if (b === undefined && ch.codePointAt(0) < 0x80) b = ch.codePointAt(0);
    if (b === undefined) throw new Error(`'${ch}' cannot be represented in the selected code page`);
    out.push(b);
  }
  return out;
}

function writeAtCursor(bytes, coalesce) {
  edit(async () => {
    const o = hasSelection() ? selection().start : state.cursor;
    const info = await api.writeBytes(o, bytes, state.insert || o === state.size, coalesce);
    state.cursor = Math.min(o + bytes.length, info.size);
    state.anchor = null;
    state.nibble = 0;
    return info;
  }).then(() => {
    view.scrollTo(state.cursor);
    emit('cursor');
  });
}

function deleteBytes(backspace) {
  edit(async () => {
    const sel = selection();
    let start, len;
    if (hasSelection()) {
      start = sel.start;
      len = sel.len;
    } else if (backspace) {
      if (state.cursor === 0) return null;
      start = state.cursor - 1;
      len = 1;
    } else {
      if (state.cursor >= state.size) return null;
      start = state.cursor;
      len = 1;
    }
    const info = await api.deleteBytes(start, len);
    state.cursor = start;
    state.anchor = null;
    state.nibble = 0;
    return info;
  }).then(() => {
    view.scrollTo(state.cursor);
    emit('cursor');
  });
}

async function copySelection() {
  const sel = selection();
  const start = hasSelection() ? sel.start : state.cursor;
  const len = hasSelection() ? sel.len : Math.min(1, state.size - state.cursor);
  if (len <= 0) return;
  if (len > 16 * 1024 * 1024) {
    toast('Selection too large to copy (max 16 MiB)', true);
    return;
  }
  const bytes = await api.readBytes(start, len);
  let text;
  if (state.pane === 'hex') {
    text = Array.from(bytes, (b) => hex(b, 2)).join(' ');
  } else if (state.utf8) {
    text = new TextDecoder().decode(bytes);
  } else {
    text = Array.from(bytes, (b) => state.charTable[b] || '.').join('');
  }
  try {
    await navigator.clipboard.writeText(text);
    toast(`Copied ${len} bytes`);
  } catch (e) {
    toast(`Copy failed: ${e}`, true);
  }
}

function paste(text) {
  try {
    let bytes;
    if (state.pane === 'hex') {
      const digits = text.replace(/0x/gi, '').replace(/[\s,;:-]/g, '');
      if (!/^([0-9a-f]{2})*$/i.test(digits)) throw new Error('Clipboard does not contain hex bytes');
      bytes = digits.match(/../g)?.map((h) => parseInt(h, 16)) ?? [];
    } else {
      bytes = encodeText(text);
    }
    if (bytes.length) writeAtCursor(bytes, false);
  } catch (e) {
    toast(e.message ?? String(e), true);
  }
}

function onHexKey(e) {
  const ctrl = e.ctrlKey || e.metaKey;
  const rows = view.visibleRows;
  const bpr = state.bpr;
  const c = state.cursor;
  const ext = e.shiftKey;
  switch (e.key) {
    case 'ArrowLeft': moveCursor(c - 1, ext); break;
    case 'ArrowRight': moveCursor(c + 1, ext); break;
    case 'ArrowUp': moveCursor(c - bpr, ext); break;
    case 'ArrowDown': moveCursor(c + bpr, ext); break;
    case 'PageUp': view.setTop(view.topRow - rows); moveCursor(c - bpr * rows, ext); break;
    case 'PageDown': view.setTop(view.topRow + rows); moveCursor(c + bpr * rows, ext); break;
    case 'Home': moveCursor(ctrl ? 0 : c - (c % bpr), ext); break;
    case 'End': moveCursor(ctrl ? state.size : Math.min(state.size, c - (c % bpr) + bpr - 1), ext); break;
    case 'Tab':
      state.pane = state.pane === 'hex' ? 'text' : 'hex';
      state.nibble = 0;
      emit('cursor');
      break;
    case 'Insert':
      state.insert = !state.insert;
      updateStatus();
      view.render();
      break;
    case 'Delete': deleteBytes(false); break;
    case 'Backspace': deleteBytes(true); break;
    case 'Escape':
      if (state.anchor !== null) { state.anchor = null; emit('cursor'); }
      break;
    default:
      if (ctrl && !e.altKey) {
        const k = e.key.toLowerCase();
        if (k === 'a') { state.anchor = 0; state.cursor = Math.max(0, state.size - 1); emit('cursor'); break; }
        if (k === 'c') { copySelection(); break; }
        return; // let global shortcuts and paste through
      }
      if (e.key.length !== 1 || e.metaKey) return;
      if (state.pane === 'hex') {
        if (/^[0-9a-f]$/i.test(e.key)) typeHexDigit(parseInt(e.key, 16));
      } else {
        try {
          writeAtCursor(encodeText(e.key), true);
        } catch (err) {
          toast(err.message, true);
        }
      }
  }
  e.preventDefault();
}

// ------------------------------------------------------------- find/goto

const findbar = $('findbar');
const findInput = $('find-input');
const findMode = $('find-mode');

function showFind() {
  findbar.hidden = false;
  findInput.focus();
  findInput.select();
}

function hideFind() {
  findbar.hidden = true;
  hexEl.focus();
}

async function findNext(backwards) {
  if (findbar.hidden) { showFind(); return; }
  const q = findInput.value;
  if (!q) return;
  store('find.mode', findMode.value);
  const sel = selection();
  const from = backwards ? sel.start : hasSelection() ? sel.start + 1 : state.cursor;
  const msg = $('find-msg');
  try {
    let r = await api.find(q, findMode.value, state.encoding, state.littleEndian, from, backwards);
    let wrapped = false;
    if (!r) {
      r = await api.find(q, findMode.value, state.encoding, state.littleEndian, backwards ? state.size : 0, backwards);
      wrapped = !!r;
    }
    if (!r) {
      msg.textContent = 'Not found';
      return;
    }
    msg.textContent = `Found at 0x${hex(r.offset)}${wrapped ? ' (wrapped)' : ''}`;
    selectRange(r.offset, r.len, { cursorAtStart: true, jump: `Find ${q}`, force: true });
    view.scrollTo(r.offset, true);
  } catch (e) {
    msg.textContent = String(e);
  }
}

function showGoto() {
  const dlg = $('goto-dialog');
  const input = $('goto-input');
  input.value = '';
  dlg.returnValue = '';
  dlg.showModal();
  $('goto-cancel').onclick = () => dlg.close('cancel');
  dlg.onclose = () => {
    hexEl.focus();
    if (dlg.returnValue !== 'ok') return;
    const v = parseOffset(input.value, state.cursor);
    if (v === null || v < 0 || v > state.size) {
      toast(`Invalid offset '${input.value}'`, true);
      return;
    }
    navNote(`Go to ${input.value.trim()}`, true);
    moveCursor(v, false);
    view.scrollTo(v, true);
  };
}

// ------------------------------------------------------------- settings

async function setEncoding(name) {
  state.encoding = name;
  const table = await api.charTable(name);
  state.charTable = table;
  state.reverseTable = new Map();
  table.forEach((ch, b) => { if (ch && !state.reverseTable.has(ch)) state.reverseTable.set(ch, b); });
  state.utf8 = (name === 'local' ? state.localEncoding : name) === 'UTF-8';
  store('ui.encoding', name);
  view?.render();
  emit('settings');
}

async function initSettings() {
  const enc = await api.encodingList();
  state.localEncoding = enc.local;
  const sel = $('sel-encoding');
  sel.append(new Option(`Local (${enc.local}, CP ${enc.local_cp})`, 'local'));
  for (const name of enc.list) sel.append(new Option(name, name));
  const saved = load('ui.encoding', 'local');
  sel.value = [...sel.options].some((o) => o.value === saved) ? saved : 'local';
  await setEncoding(sel.value);
  sel.addEventListener('change', () => setEncoding(sel.value));

  const bpr = $('sel-bpr');
  bpr.value = String(load('ui.bpr', 16));
  state.bpr = Number(bpr.value) || 16;
  bpr.addEventListener('change', () => {
    state.bpr = Number(bpr.value);
    store('ui.bpr', state.bpr);
    view.layoutChanged();
  });

  const endian = $('sel-endian');
  endian.value = load('ui.endian', 'le');
  state.littleEndian = endian.value === 'le';
  endian.addEventListener('change', () => {
    state.littleEndian = endian.value === 'le';
    store('ui.endian', endian.value);
    emit('settings');
  });
}

// Themes are TOML files in the themes folder, the standard Light and Dark included.
// Each names a base (light/dark) whose standard palette fills in the colors it leaves
// out. "System" uses the themes named Light and Dark, following Windows, or the
// standard palettes when those are not installed.
const darkQuery = matchMedia('(prefers-color-scheme: dark)');
const THEME_RELOAD = '\u0000reload';
const THEME_FOLDER = '\u0000folder';
const themes = { list: [], fallback: null, choice: 'system' };

function findTheme(name) {
  return themes.list.find((t) => t.name.toLowerCase() === name.toLowerCase());
}

async function applyTheme() {
  const chosen = themes.choice === 'system' ? null : findTheme(themes.choice);
  const win = window.__TAURI__?.window?.getCurrentWindow();
  // reset the window first: a forced window theme also changes what darkQuery reports
  try { await win?.setTheme(chosen ? chosen.base : null); } catch { /* not permitted */ }
  const osBase = darkQuery.matches ? 'dark' : 'light';
  const theme = chosen ?? findTheme(osBase) ?? { base: osBase, colors: {} };
  // without the palettes (backend unreachable) keep what index.html applied
  if (!themes.fallback) return;
  const colors = { ...themes.fallback[theme.base], ...theme.colors };
  const root = document.documentElement;
  for (const p of [...root.style]) if (p.startsWith('--')) root.style.removeProperty(p);
  root.dataset.theme = theme.base;
  for (const [k, v] of Object.entries(colors)) root.style.setProperty(`--${k}`, v);
  // for the first paint of the next start (see index.html)
  store('ui.themeApplied', { base: theme.base, colors });
}

function fillThemeMenu() {
  const sel = $('sel-theme');
  sel.replaceChildren(new Option('System', 'system'));
  for (const t of themes.list) {
    const o = new Option(t.name, t.name);
    o.title = `${t.file} (${t.base})`;
    sel.append(o);
  }
  const actions = document.createElement('optgroup');
  actions.label = '──────────';
  actions.append(new Option('Reload themes', THEME_RELOAD), new Option('Open themes folder…', THEME_FOLDER));
  sel.append(actions);
  // a theme whose file is gone shows as System, but stays chosen in case it comes back
  sel.value = findTheme(themes.choice)?.name ?? 'system';
}

async function loadThemes(report) {
  try {
    const r = await api.themeList();
    themes.list = r.themes;
    themes.fallback = r.fallback;
    store('ui.themeFallback', r.fallback);
    if (r.warnings.length) toast(`Themes: ${r.warnings.join(' · ')}`, true);
    else if (report) toast(`Loaded ${r.themes.length} theme(s) from ${r.dir}`);
  } catch (e) {
    toast(String(e), true);
  }
  fillThemeMenu();
  await applyTheme();
}

function initBackup() {
  const box = $('opt-backup');
  box.checked = load('ui.backup', false);
  box.addEventListener('change', () => store('ui.backup', box.checked));
}

function initTheme() {
  const sel = $('sel-theme');
  themes.choice = load('ui.theme', 'system') || 'system';
  let shown = sel.value;
  sel.addEventListener('focus', () => { shown = sel.value; });
  sel.addEventListener('change', async () => {
    if (sel.value === THEME_RELOAD || sel.value === THEME_FOLDER) {
      const action = sel.value;
      sel.value = shown;
      if (action === THEME_RELOAD) await loadThemes(true);
      else api.themeOpenFolder().catch((e) => toast(String(e), true));
      return;
    }
    shown = sel.value;
    themes.choice = sel.value;
    store('ui.theme', themes.choice);
    applyTheme();
  });
  darkQuery.addEventListener('change', () => { if (themes.choice === 'system') applyTheme(); });
  loadThemes(false);
}

function initSplitter() {
  const split = $('splitter');
  const side = $('side');
  const saved = load('ui.sideWidth', null);
  if (saved) side.style.width = `${saved}px`;
  split.addEventListener('mousedown', (e) => {
    e.preventDefault();
    split.classList.add('drag');
    const startX = e.clientX;
    const startW = side.offsetWidth;
    const move = (ev) => {
      const w = Math.min(window.innerWidth - 300, Math.max(280, startW - (ev.clientX - startX)));
      side.style.width = `${w}px`;
    };
    const up = () => {
      split.classList.remove('drag');
      store('ui.sideWidth', side.offsetWidth);
      window.removeEventListener('mousemove', move);
      window.removeEventListener('mouseup', up);
    };
    window.addEventListener('mousemove', move);
    window.addEventListener('mouseup', up);
  });
}

function initTabs() {
  const tabs = document.querySelectorAll('.tabs button');
  const show = (name) => {
    for (const t of tabs) t.classList.toggle('active', t.dataset.tab === name);
    $('tab-inspector').hidden = name !== 'inspector';
    $('tab-template').hidden = name !== 'template';
    store('ui.tab', name);
  };
  for (const t of tabs) t.addEventListener('click', () => show(t.dataset.tab));
  show(load('ui.tab', 'inspector'));
}

// ------------------------------------------------------------------ init

async function main() {
  initTheme();
  initBackup();
  await initSettings();
  view = new HexView();
  initRecent();
  setInfo(await api.docInfo());
  remember(state.info.path);  // a file given on the command line
  view.layoutChanged();

  new Inspector({ edit, toast });
  new TemplatePanel({ selectRange, toast });
  initSplitter();
  initTabs();

  on('cursor', () => { updateStatus(); view.render(); });
  on('highlight', () => view.render());
  on('fields', () => view.render());

  hexEl.addEventListener('keydown', onHexKey);
  document.addEventListener('paste', (e) => {
    if (document.activeElement !== hexEl) return;
    e.preventDefault();
    paste(e.clipboardData.getData('text'));
  });

  $('btn-new').onclick = newFile;
  $('btn-open').onclick = () => openFile();
  $('btn-save').onclick = () => saveFile(false);
  $('btn-saveas').onclick = () => saveFile(true);
  $('btn-undo').onclick = () => undoRedo(false);
  $('btn-redo').onclick = () => undoRedo(true);
  $('btn-goto').onclick = showGoto;
  $('btn-find').onclick = showFind;
  $('st-mode').onclick = () => { state.insert = !state.insert; updateStatus(); view.render(); };

  findMode.value = load('find.mode', 'hex');
  $('find-next').onclick = () => findNext(false);
  $('find-prev').onclick = () => findNext(true);
  $('find-close').onclick = hideFind;
  findInput.addEventListener('keydown', (e) => {
    if (e.key === 'Enter') { e.preventDefault(); findNext(e.shiftKey); }
    if (e.key === 'Escape') { e.preventDefault(); hideFind(); }
  });

  document.addEventListener('keydown', (e) => {
    const ctrl = e.ctrlKey || e.metaKey;
    const inField = e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement;
    const k = e.key.toLowerCase();
    if (e.key === 'F3') { e.preventDefault(); findNext(e.shiftKey); return; }
    if (!ctrl || e.altKey) return;
    if (k === 'o') { e.preventDefault(); openFile(); }
    else if (k === 'n') { e.preventDefault(); newFile(); }
    else if (k === 's') { e.preventDefault(); saveFile(e.shiftKey); }
    else if (k === 'g') { e.preventDefault(); showGoto(); }
    else if (k === 'f') { e.preventDefault(); showFind(); }
    else if (e.key === '-' || e.key === '_' || e.code === 'NumpadSubtract') {
      // Ctrl+- / Ctrl+Shift+- as in Visual Studio; this also keeps WebView2 from zooming
      e.preventDefault();
      navGo(e.shiftKey ? 1 : -1);
    }
    else if (inField) return;
    else if (k === 'z') { e.preventDefault(); undoRedo(e.shiftKey); }
    else if (k === 'y') { e.preventDefault(); undoRedo(true); }
  });

  initMenu();
  initNav();

  await listen('tauri://drag-drop', (e) => {
    const p = e.payload?.paths?.[0];
    if (p) openFile(p);
  });

  emit('cursor');
  emit('data');
  hexEl.focus();
}

main().catch((e) => toast(String(e), true));
