// Template panel: editor, root/offset selection and the result tree.

import { api } from './api.js';
import { state, on, emit, hex, load, store, parseOffset } from './state.js';

const INSTANCES = '\u0000instances';
const COLORS = 6;
const MAX_RANGES = 200_000;

const DEFAULT_SOURCE = `// Templates are C declarations, click "?" for the syntax.
// Pick a root type and an offset below, then Apply (Ctrl+Enter).
#pragma pack(1)

typedef struct {
    char     magic[4];
    uint16_t version;
    uint16_t flags;
    uint32_t count;
    uint32_t offsets[count];
} Example;
`;

export class TemplatePanel {
  constructor({ selectRange, toast }) {
    this.selectRange = selectRange;
    this.toast = toast;
    this.editor = document.getElementById('tpl-editor');
    this.gutter = document.getElementById('tpl-gutter');
    this.rootSel = document.getElementById('tpl-root');
    this.offsetIn = document.getElementById('tpl-offset');
    this.follow = document.getElementById('tpl-follow');
    this.auto = document.getElementById('tpl-auto');
    this.errorEl = document.getElementById('tpl-error');
    this.tree = document.getElementById('tpl-tree');
    this.path = load('tpl.path', null);
    this.errorLine = null;
    this.seq = 0;
    this.activeRow = null;

    this.editor.value = load('tpl.source', DEFAULT_SOURCE);
    this.offsetIn.value = load('tpl.offset', '0x0');
    this.follow.checked = load('tpl.follow', false);
    this.auto.checked = load('tpl.auto', true);
    this.wantedRoot = load('tpl.root', null);

    let parseTimer;
    this.editor.addEventListener('input', () => {
      this.updateGutter();
      clearTimeout(parseTimer);
      parseTimer = setTimeout(() => this.parse(true), 300);
    });
    this.editor.addEventListener('scroll', () => { this.gutter.scrollTop = this.editor.scrollTop; });
    this.editor.addEventListener('keydown', (e) => this.onEditorKey(e));
    this.rootSel.addEventListener('change', () => {
      this.wantedRoot = this.rootSel.value;
      store('tpl.root', this.wantedRoot);
      this.apply();
    });
    this.offsetIn.addEventListener('change', () => {
      store('tpl.offset', this.offsetIn.value);
      this.apply();
    });
    this.follow.addEventListener('change', () => {
      store('tpl.follow', this.follow.checked);
      this.apply();
    });
    this.auto.addEventListener('change', () => store('tpl.auto', this.auto.checked));
    document.getElementById('tpl-apply').addEventListener('click', () => this.apply());
    document.getElementById('tpl-open').addEventListener('click', () => this.open());
    document.getElementById('tpl-save').addEventListener('click', () => this.save(false));
    document.getElementById('tpl-saveas').addEventListener('click', () => this.save(true));
    document.getElementById('tpl-help').addEventListener('click', () => document.getElementById('help-dialog').showModal());
    this.initExamples();
    this.initSplitter();

    let followTimer;
    on('cursor', () => {
      if (!this.follow.checked || !this.auto.checked || this.isInstances()) return;
      clearTimeout(followTimer);
      followTimer = setTimeout(() => this.apply(), 60);
    });
    let dataTimer;
    on('data', () => {
      if (!this.auto.checked || !this.applied) return;
      clearTimeout(dataTimer);
      dataTimer = setTimeout(() => this.apply(), 150);
    });
    on('settings', () => { if (this.applied) this.apply(); });
    on('doc', () => { if (this.auto.checked && this.applied) this.apply(); });

    this.updateGutter();
    this.parse(false);
  }

  isInstances() {
    return this.rootSel.value === INSTANCES;
  }

  onEditorKey(e) {
    if (e.key === 'Tab' && !e.ctrlKey) {
      e.preventDefault();
      document.execCommand('insertText', false, '    ');
    } else if (e.key === 'Enter' && e.ctrlKey) {
      e.preventDefault();
      this.apply();
    } else if (e.key.toLowerCase() === 's' && e.ctrlKey) {
      e.preventDefault();
      e.stopPropagation();
      this.save(e.shiftKey);
    }
  }

  updateGutter() {
    const lines = this.editor.value.split('\n').length;
    let out = '';
    for (let i = 1; i <= lines; i++) {
      out += i === this.errorLine ? `<span class="err">${i}</span>\n` : `${i}\n`;
    }
    this.gutter.innerHTML = out;
    this.gutter.scrollTop = this.editor.scrollTop;
  }

  setSource(text) {
    // keeps the textarea's undo history
    this.editor.focus();
    this.editor.select();
    if (!document.execCommand('insertText', false, text)) this.editor.value = text;
    this.editor.setSelectionRange(0, 0);
    this.editor.scrollTop = 0;
    this.updateGutter();
    this.parse(true);
  }

  showError(msg, line = null) {
    this.errorEl.hidden = !msg;
    this.errorEl.textContent = msg ?? '';
    if (this.errorLine !== line) {
      this.errorLine = line;
      this.updateGutter();
    }
  }

  async parse(applyAfter) {
    const source = this.editor.value;
    store('tpl.source', source);
    const r = await api.templateParse(source);
    if (r.error) {
      this.showError(`line ${r.error.line}:${r.error.col}: ${r.error.msg}`, r.error.line);
      return;
    }
    this.showError(null);
    const prev = this.rootSel.value || this.wantedRoot;
    this.rootSel.replaceChildren();
    if (r.has_instances) this.rootSel.append(new Option('(template declarations)', INSTANCES));
    for (const t of r.types) this.rootSel.append(new Option(t, t));
    const values = [...this.rootSel.options].map((o) => o.value);
    if (values.includes(prev)) this.rootSel.value = prev;
    else if (r.has_instances) this.rootSel.value = INSTANCES;
    else if (values.length) this.rootSel.value = values[values.length - 1];
    if (applyAfter && this.auto.checked) this.apply();
    else if (!applyAfter && this.wantedRoot !== null && values.length) this.apply();
  }

  async apply() {
    if (!this.rootSel.value) {
      this.clear();
      return;
    }
    let offset;
    if (this.follow.checked && !this.isInstances()) {
      offset = state.cursor;
      this.offsetIn.value = `0x${hex(offset, 1)}`;
    } else {
      offset = parseOffset(this.offsetIn.value);
      if (offset === null) {
        this.showError(`invalid offset '${this.offsetIn.value}'`);
        return;
      }
    }
    const seq = ++this.seq;
    try {
      const root = this.isInstances() ? null : this.rootSel.value;
      const node = await api.templateApply(this.editor.value, root, offset, state.littleEndian, state.encoding);
      if (seq !== this.seq) return;
      this.showError(null);
      this.applied = true;
      // remembered so the template is re-applied on the next start
      this.wantedRoot = this.rootSel.value;
      store('tpl.root', this.wantedRoot);
      this.render(node);
    } catch (e) {
      if (seq !== this.seq) return;
      const m = /^line (\d+):/.exec(String(e));
      this.showError(String(e), m ? Number(m[1]) : null);
    }
  }

  clear() {
    this.applied = false;
    store('tpl.root', null);
    this.tree.replaceChildren();
    state.fieldRanges = [];
    emit('fields');
  }

  // ------------------------------------------------------------------ tree

  render(root) {
    // colour leaves in order, then flatten into non-overlapping ranges for the hex view
    const ranges = [];
    let n = 0;
    const walk = (node) => {
      if (node.children?.length) {
        node.children.forEach(walk);
        if (node.more) {
          const last = node.children[node.children.length - 1];
          const s = last.offset + last.size;
          node.c = last.c;
          if (ranges.length < MAX_RANGES) ranges.push({ s, e: node.offset + node.size, c: node.c });
        }
      } else if (node.size > 0) {
        node.c = n++ % COLORS;
        if (ranges.length < MAX_RANGES) ranges.push({ s: node.offset, e: node.offset + node.size, c: node.c });
      }
    };
    walk(root);
    ranges.sort((a, b) => a.s - b.s);
    const flat = [];
    let covered = 0;
    for (const r of ranges) {
      const s = Math.max(r.s, covered);
      if (s < r.e) {
        flat.push({ s, e: r.e, c: r.c });
        covered = r.e;
      }
    }
    state.fieldRanges = flat;
    emit('fields');

    const expanded = this.expandedPaths();
    const head = document.createElement('div');
    head.className = 'trow head';
    head.innerHTML = '<span>Name</span><span>Value</span><span>Type</span><span>Offset</span><span>Size</span>';
    const body = document.createElement('div');
    const top = root.name === 'template' && !root.type ? root.children ?? [] : [root];
    for (const child of top) body.append(this.nodeEl(child, 0, child.name, expanded, true));
    this.tree.replaceChildren(head, body);
  }

  expandedPaths() {
    const set = new Set();
    for (const el of this.tree.querySelectorAll('.tnode.open')) set.add(el.dataset.path);
    return set;
  }

  nodeEl(node, depth, path, expanded, topLevel = false) {
    const wrap = document.createElement('div');
    wrap.className = 'tnode';
    wrap.dataset.path = path;
    const row = document.createElement('div');
    row.className = 'trow' + (node.error ? ' err' : '');
    row.setAttribute('role', 'treeitem');

    const hasKids = (node.children?.length ?? 0) > 0;
    const name = document.createElement('span');
    name.className = 'name';
    name.style.paddingLeft = `${depth * 14}px`;
    const tog = document.createElement('span');
    tog.className = 'tog';
    tog.textContent = hasKids ? '▸' : '';
    name.append(tog);
    if (!hasKids && node.c !== undefined) {
      const sw = document.createElement('span');
      sw.className = `swatch f${node.c}`;
      name.append(sw);
    }
    name.append(document.createTextNode(node.name || '(anonymous)'));

    const cell = (cls, text) => {
      const s = document.createElement('span');
      s.className = cls;
      s.textContent = text;
      s.title = text;
      return s;
    };
    let value = node.error ? `⚠ ${node.error}` + (node.value ? ` · ${node.value}` : '') : node.value;
    if (node.bits) value += `  [bits ${node.bits[0]}..${node.bits[0] + node.bits[1] - 1}]`;
    row.append(
      name,
      cell('value', value),
      cell('type', node.type),
      cell('off', hex(node.offset, 8)),
      cell('size', String(node.size)),
    );
    wrap.append(row);

    let kids = null;
    const toggle = (open) => {
      if (!hasKids) return;
      if (open && !kids) {
        kids = document.createElement('div');
        node.children.forEach((c, i) => kids.append(this.nodeEl(c, depth + 1, `${path}/${c.name || i}`, expanded)));
        if (node.more) {
          const more = document.createElement('div');
          more.className = 'trow more';
          more.innerHTML = `<span style="padding-left:${(depth + 1) * 14 + 16}px">… ${node.more} more elements</span>`;
          kids.append(more);
        }
        wrap.append(kids);
      }
      if (kids) kids.hidden = !open;
      wrap.classList.toggle('open', open);
      tog.textContent = open ? '▾' : '▸';
    };
    if (expanded.has(path) || (topLevel && expanded.size === 0 && node.children?.length <= 64)) toggle(true);

    tog.addEventListener('click', (e) => {
      e.stopPropagation();
      toggle(!wrap.classList.contains('open'));
    });
    row.addEventListener('dblclick', () => toggle(!wrap.classList.contains('open')));
    row.addEventListener('click', () => {
      this.activeRow?.classList.remove('active');
      row.classList.add('active');
      this.activeRow = row;
      if (node.size > 0) this.selectRange(node.offset, node.size);
    });
    row.addEventListener('mouseenter', () => {
      state.highlight = node.size > 0 ? { start: node.offset, len: node.size } : null;
      emit('highlight');
    });
    row.addEventListener('mouseleave', () => {
      state.highlight = null;
      emit('highlight');
    });
    return wrap;
  }

  // ------------------------------------------------------------- files

  async open() {
    try {
      const f = await api.templateOpen();
      if (!f) return;
      this.path = f.path;
      store('tpl.path', f.path);
      this.setSource(f.text);
      this.toast(`Loaded ${f.path}`);
    } catch (e) {
      this.toast(String(e), true);
    }
  }

  async save(saveAs) {
    try {
      const p = await api.templateSave(saveAs ? null : this.path, this.editor.value);
      if (!p) return;
      this.path = p;
      store('tpl.path', p);
      this.toast(`Saved ${p}`);
    } catch (e) {
      this.toast(String(e), true);
    }
  }

  async initExamples() {
    const sel = document.getElementById('tpl-examples');
    const examples = await api.templateExamples();
    for (const [i, ex] of examples.entries()) sel.append(new Option(ex.name, String(i)));
    sel.addEventListener('change', () => {
      const ex = examples[Number(sel.value)];
      sel.value = '';
      if (!ex) return;
      this.path = null;
      store('tpl.path', null);
      this.wantedRoot = INSTANCES;
      this.rootSel.value = '';
      this.setSource(ex.text);
      if (!this.follow.checked) this.offsetIn.value = '0x0';
    });
  }

  initSplitter() {
    const split = document.getElementById('tpl-hsplit');
    const wrap = document.getElementById('tpl-editor-wrap');
    const saved = load('ui.editorHeight', null);
    if (saved) wrap.style.height = `${saved}px`;
    split.addEventListener('mousedown', (e) => {
      e.preventDefault();
      split.classList.add('drag');
      const startY = e.clientY;
      const startH = wrap.offsetHeight;
      const move = (ev) => { wrap.style.height = `${Math.max(80, startH + ev.clientY - startY)}px`; };
      const up = () => {
        split.classList.remove('drag');
        store('ui.editorHeight', wrap.offsetHeight);
        window.removeEventListener('mousemove', move);
        window.removeEventListener('mouseup', up);
      };
      window.addEventListener('mousemove', move);
      window.addEventListener('mouseup', up);
    });
  }
}
