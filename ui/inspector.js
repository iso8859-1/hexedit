// Data inspector: basic types decoded at the cursor (or selection start).

import { api } from './api.js';
import { state, on, emit, selection, hasSelection, hex } from './state.js';
import { addCopyButton } from './copy.js';

const GROUPS = [
  ['Integers', ['bin', 'i8', 'u8', 'i16', 'u16', 'i24', 'u24', 'i32', 'u32', 'i64', 'u64']],
  ['Floating point', ['f16', 'f32', 'f64']],
  ['Variable length', ['uleb128', 'sleb128']],
  ['Characters', ['char_cp', 'char_utf8', 'char_utf16']],
  ['Strings', ['str_cp', 'str_utf8', 'str_utf16']],
  ['Date / time', ['unix32', 'unix64', 'filetime', 'dos', 'oledate']],
  ['Other', ['guid']],
];

export class Inspector {
  constructor({ edit, toast }) {
    this.edit = edit;          // (fn) => queued edit returning DocInfo
    this.toast = toast;
    this.body = document.querySelector('#inspector tbody');
    this.head = document.getElementById('inspector-head');
    this.offset = 0;
    this.timer = null;
    this.seq = 0;
    this.editing = false;

    for (const evt of ['cursor', 'data', 'settings']) on(evt, () => this.schedule());
    document.getElementById('inspector').addEventListener('mouseleave', () => {
      state.highlight = null;
      emit('highlight');
    });
  }

  schedule() {
    if (this.editing) return;
    clearTimeout(this.timer);
    this.timer = setTimeout(() => this.refresh(), 25);
  }

  async refresh() {
    const sel = selection();
    const offset = hasSelection() ? sel.start : state.cursor;
    const selLen = hasSelection() ? sel.len : 0;
    const seq = ++this.seq;
    const entries = await api.inspect(offset, selLen, state.littleEndian, state.encoding);
    if (seq !== this.seq || this.editing) return;
    this.offset = offset;
    const order = state.littleEndian ? 'little endian' : 'big endian';
    this.head.textContent = `@ 0x${hex(offset, 8)} (${offset}) · ${order}${selLen ? ` · ${selLen} bytes selected` : ''}`;

    const byKey = new Map(entries.map((e) => [e.key, e]));
    this.body.replaceChildren();
    for (const [title, keys] of GROUPS) {
      const sep = document.createElement('tr');
      sep.className = 'sep';
      sep.innerHTML = `<td colspan="2"></td>`;
      sep.firstChild.textContent = title;
      this.body.append(sep);
      for (const k of keys) {
        const e = byKey.get(k);
        if (e) this.body.append(this.row(e));
      }
    }
  }

  row(e) {
    const tr = document.createElement('tr');
    const label = document.createElement('td');
    const value = document.createElement('td');
    label.textContent = e.label;
    value.textContent = e.value ?? '—';
    value.title = e.value ?? 'not enough data';
    tr.append(label, value);
    if (e.value === null) tr.classList.add('na');
    else addCopyButton(value, e.value, (m) => this.toast?.(m, true));
    tr.addEventListener('mouseenter', () => {
      state.highlight = e.value === null ? null : { start: this.offset, len: e.size };
      emit('highlight');
    });
    if (e.editable && e.value !== null) {
      tr.classList.add('editable');
      tr.title = 'Double-click to edit';
      value.addEventListener('dblclick', () => this.startEdit(value, e));
    }
    return tr;
  }

  startEdit(td, e) {
    this.editing = true;
    const input = document.createElement('input');
    input.type = 'text';
    input.value = e.value;
    td.replaceChildren(input);
    input.focus();
    input.select();
    const offset = this.offset;
    let done = false;
    const finish = async (commit) => {
      if (done) return;
      done = true;
      this.editing = false;
      if (commit && input.value !== e.value) {
        await this.edit(() => api.inspectorWrite(offset, e.key, input.value, state.littleEndian));
      }
      this.refresh();
      document.getElementById('hexview').focus();
    };
    input.addEventListener('keydown', (ev) => {
      ev.stopPropagation();
      if (ev.key === 'Enter') finish(true);
      if (ev.key === 'Escape') finish(false);
    });
    input.addEventListener('blur', () => finish(false));
  }
}
