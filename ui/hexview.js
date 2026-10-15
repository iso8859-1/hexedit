// Virtualised hex view: only the visible rows exist in the DOM, the scrollbar
// is a proxy so files with millions of rows still scroll smoothly.

import { api } from './api.js';
import { state, emit, selection, hex } from './state.js';

const ROW_H = 20;
const MAX_SCROLL_PX = 8_000_000;
const FETCH_PAD = 16 * 1024;

export class HexView {
  constructor() {
    this.el = document.getElementById('hexview');
    this.rowsEl = document.getElementById('hex-rows');
    this.headerEl = document.getElementById('hex-header');
    this.emptyEl = document.getElementById('hex-empty');
    this.vs = document.getElementById('vscroll');
    this.vsInner = document.getElementById('vscroll-inner');
    this.topRow = 0;
    this.cache = { start: 0, data: new Uint8Array(0), valid: false };
    this.drawing = false;
    this.again = false;
    this.lastScrollTop = -1;
    this.dragging = false;

    this.el.addEventListener('wheel', (e) => this.onWheel(e), { passive: false });
    this.vs.addEventListener('scroll', () => this.onScrollbar());
    this.rowsEl.addEventListener('mousedown', (e) => this.onMouseDown(e));
    window.addEventListener('mousemove', (e) => this.onMouseMove(e));
    window.addEventListener('mouseup', () => { this.dragging = false; clearInterval(this.autoScroll); });
    new ResizeObserver(() => { this.clampTop(); this.updateScrollbar(); this.render(); }).observe(this.rowsEl);
    this.renderHeader();
  }

  get visibleRows() { return Math.max(1, Math.floor(this.rowsEl.clientHeight / ROW_H)); }
  // one extra position past the last byte lets the cursor append
  get totalRows() { return Math.floor(state.size / state.bpr) + 1; }
  get maxTop() { return Math.max(0, this.totalRows - this.visibleRows); }
  get offsetDigits() { return state.size > 0xFFFFFFFF ? 12 : 8; }

  invalidate() {
    this.cache.valid = false;
    this.updateScrollbar();
    this.clampTop();
    this.render();
  }

  layoutChanged() {
    this.renderHeader();
    this.invalidate();
    this.scrollTo(state.cursor);
  }

  clampTop() {
    this.topRow = Math.min(Math.max(0, this.topRow), this.maxTop);
  }

  setTop(row) {
    const old = this.topRow;
    this.topRow = row;
    this.clampTop();
    if (this.topRow !== old) {
      this.syncScrollbar();
      this.render();
    }
  }

  /** Scrolls so that `offset` is visible. */
  scrollTo(offset, center = false) {
    const row = Math.floor(offset / state.bpr);
    const vis = this.visibleRows;
    if (center && (row < this.topRow || row >= this.topRow + vis)) {
      this.setTop(row - Math.floor(vis / 2));
    } else if (row < this.topRow) {
      this.setTop(row);
    } else if (row >= this.topRow + vis) {
      this.setTop(row - vis + 1);
    }
  }

  updateScrollbar() {
    const h = Math.min(this.totalRows * ROW_H + ROW_H, MAX_SCROLL_PX);
    this.vsInner.style.height = `${h}px`;
    this.syncScrollbar();
  }

  syncScrollbar() {
    const range = this.vs.scrollHeight - this.vs.clientHeight;
    if (range <= 0) return;
    const max = this.maxTop;
    this.vs.scrollTop = max ? (this.topRow / max) * range : 0;
    this.lastScrollTop = this.vs.scrollTop;
  }

  onScrollbar() {
    if (Math.abs(this.vs.scrollTop - this.lastScrollTop) < 1) return;
    const range = this.vs.scrollHeight - this.vs.clientHeight;
    if (range <= 0) return;
    this.topRow = Math.round((this.vs.scrollTop / range) * this.maxTop);
    this.clampTop();
    this.lastScrollTop = this.vs.scrollTop;
    this.render();
  }

  onWheel(e) {
    e.preventDefault();
    const px = e.deltaMode === 1 ? e.deltaY * ROW_H : e.deltaMode === 2 ? e.deltaY * this.rowsEl.clientHeight : e.deltaY;
    const rows = Math.round(px / ROW_H) || Math.sign(px);
    this.setTop(this.topRow + rows);
  }

  hitTest(x, y) {
    const el = document.elementFromPoint(x, y)?.closest('#hex-rows span[data-o]');
    if (!el) return null;
    return { offset: Number(el.dataset.o), pane: el.parentElement.classList.contains('txt') ? 'text' : 'hex' };
  }

  onMouseDown(e) {
    if (e.button !== 0) return;
    const hit = this.hitTest(e.clientX, e.clientY);
    if (!hit) return;
    e.preventDefault();
    this.el.focus();
    if (e.shiftKey) {
      if (state.anchor === null) state.anchor = state.cursor;
    } else {
      state.anchor = hit.offset;
    }
    state.cursor = hit.offset;
    state.pane = hit.pane;
    state.nibble = 0;
    this.dragging = true;
    emit('cursor');
  }

  onMouseMove(e) {
    if (!this.dragging) return;
    const r = this.rowsEl.getBoundingClientRect();
    clearInterval(this.autoScroll);
    if (e.clientY < r.top || e.clientY > r.bottom) {
      // keep extending the selection while the pointer is above/below the view
      const dir = e.clientY < r.top ? -1 : 1;
      const step = () => {
        this.setTop(this.topRow + dir);
        const row = dir < 0 ? this.topRow : this.topRow + this.visibleRows - 1;
        const col = state.cursor % state.bpr;
        state.cursor = Math.min(state.size, row * state.bpr + col);
        emit('cursor');
      };
      step();
      this.autoScroll = setInterval(step, 40);
      return;
    }
    const hit = this.hitTest(Math.min(Math.max(e.clientX, r.left + 1), r.right - 20), e.clientY);
    if (hit && hit.offset !== state.cursor) {
      state.cursor = hit.offset;
      emit('cursor');
    }
  }

  async ensure(start, end) {
    end = Math.min(end, state.size);
    const c = this.cache;
    if (c.valid && start >= c.start && end <= c.start + c.data.length) return;
    const fs = Math.max(0, start - FETCH_PAD);
    const fe = Math.min(state.size, end + FETCH_PAD);
    const data = await api.readBytes(fs, fe - fs);
    this.cache = { start: fs, data, valid: true };
  }

  /** Byte at `o` if it is cached, else undefined. */
  byteAt(o) {
    const c = this.cache;
    return c.valid ? c.data[o - c.start] : undefined;
  }

  renderHeader() {
    const bpr = state.bpr;
    let h = `<span class="off">Offset</span><span class="hex">`;
    for (let i = 0; i < bpr; i++) {
      const gap = i % 8 === 7 && i !== bpr - 1 ? ' class="gap"' : '';
      h += `<span${gap}>${hex(i, 2)}</span>`;
    }
    h += `</span><span class="muted">Decoded text</span>`;
    this.headerEl.innerHTML = h;
  }

  render() {
    if (this.drawing) {
      this.again = true;
      return;
    }
    this.drawing = true;
    requestAnimationFrame(async () => {
      try {
        await this.draw();
      } catch (e) {
        console.error(e);
      } finally {
        this.drawing = false;
        if (this.again) {
          this.again = false;
          this.render();
        }
      }
    });
  }

  /** Template colour class per byte for [ws, we). */
  colors(ws, we) {
    const out = new Array(we - ws).fill(-1);
    const r = state.fieldRanges;
    let lo = 0, hi = r.length;
    while (lo < hi) {
      const mid = (lo + hi) >> 1;
      if (r[mid].e <= ws) lo = mid + 1; else hi = mid;
    }
    for (let i = lo; i < r.length && r[i].s < we; i++) {
      for (let o = Math.max(ws, r[i].s); o < Math.min(we, r[i].e); o++) out[o - ws] = r[i].c;
    }
    return out;
  }

  async draw() {
    const bpr = state.bpr;
    const vis = this.visibleRows + 1;
    const start = this.topRow * bpr;
    const end = start + vis * bpr;
    await this.ensure(start, end);

    this.emptyEl.hidden = state.size > 0;
    const { data, start: cs } = this.cache;
    const sel = selection();
    const hasSel = state.anchor !== null && sel.len > 0;
    const hl = state.highlight;
    const colors = this.colors(start, end);
    const table = state.charTable;
    const digits = this.offsetDigits;
    const curHex = state.pane === 'hex' ? ' act' : '';
    const curTxt = state.pane === 'text' ? ' act' : '';

    let html = '';
    for (let r = 0; r < vis; r++) {
      const rowStart = start + r * bpr;
      if (rowStart > state.size) break;
      let hx = '', tx = '';
      for (let i = 0; i < bpr; i++) {
        const o = rowStart + i;
        const gap = i % 8 === 7 && i !== bpr - 1 ? ' gap' : '';
        if (o > state.size) {
          hx += `<span class="${gap}">  </span>`;
          tx += `<span> </span>`;
          continue;
        }
        let cls = '';
        const c = colors[o - start];
        if (c >= 0) cls += ` f${c}`;
        if (hl && o >= hl.start && o < hl.start + hl.len) cls += ' hl';
        if (hasSel && o >= sel.start && o <= sel.end) cls += ' sel';
        const isCur = o === state.cursor;
        if (o === state.size) {
          // append position
          hx += `<span data-o="${o}" class="eof${gap}${isCur ? ' cur' + curHex : ''}">${isCur ? '__' : '  '}</span>`;
          tx += `<span data-o="${o}" class="${isCur ? 'cur' + curTxt : ''}"> </span>`;
          continue;
        }
        const b = data[o - cs];
        const h = b === undefined ? '??' : hex(b, 2);
        let hcell = h;
        if (isCur && state.pane === 'hex') {
          hcell = state.nibble === 0 ? `<i>${h[0]}</i>${h[1]}` : `${h[0]}<i>${h[1]}</i>`;
        }
        const ch = b === undefined ? '' : table[b];
        hx += `<span data-o="${o}" class="${cls}${gap}${b === 0 ? ' z' : ''}${isCur ? ' cur' + curHex : ''}">${hcell}</span>`;
        tx += `<span data-o="${o}" class="${cls}${ch ? '' : ' np'}${isCur ? ' cur' + curTxt : ''}">${ch ? escapeHtml(ch) : '.'}</span>`;
      }
      html += `<div class="hexrow"><span class="off">${hex(rowStart, digits)}</span><span class="hex">${hx}</span><span class="txt">${tx}</span></div>`;
    }
    this.rowsEl.innerHTML = html;
  }
}

function escapeHtml(s) {
  return s === '<' ? '&lt;' : s === '>' ? '&gt;' : s === '&' ? '&amp;' : s;
}
