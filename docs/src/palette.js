/**
 * Global command palette — Cmd+K / Ctrl+K / "/".
 *
 * Indexes static explorer items (palette-data.js) plus runtime-built
 * section + sub-heading entries from main.js. Free-text filter, keyboard
 * navigation (↑ / ↓ / Enter), Esc to close.
 */

import { STATIC_PALETTE_ITEMS } from './palette-data.js';

/** Lower-case substring match against label + category + desc. */
function matches(item, q) {
  if (!q) return true;
  const hay = `${item.label} ${item.category} ${item.desc}`.toLowerCase();
  return hay.includes(q);
}

/** Escape HTML so user-supplied text never breaks markup. */
function escapeHtml(s) {
  return String(s ?? '')
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;');
}
function escapeRegex(s) {
  return s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

/** Wrap case-insensitive substring matches of `q` in <mark>. */
function highlight(text, q) {
  const esc = escapeHtml(text);
  if (!q) return esc;
  const re = new RegExp(escapeRegex(escapeHtml(q.trim())), 'gi');
  return esc.replace(re, (m) => `<mark>${m}</mark>`);
}

/**
 * Register the Alpine factory. `dynamicItems` is a function that returns
 * the section + sub-heading entries (built by main.js after content render).
 */
export function registerPalette(Alpine, dynamicItems) {
  Alpine.data('commandPalette', () => ({
    open: false,
    query: '',
    cursor: 0,

    init() {
      // Global key handler — Cmd+K / Ctrl+K to toggle; Esc to close
      // when open. "/" to open only when no input element has focus,
      // so it doesn't hijack typing inside the explorer search boxes.
      window.addEventListener('keydown', (e) => {
        if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'k') {
          e.preventDefault();
          this.toggle();
          return;
        }
        if (this.open && e.key === 'Escape') {
          e.preventDefault();
          this.close();
          return;
        }
        if (!this.open && e.key === '/' && !this.isInInput(e.target)) {
          e.preventDefault();
          this.show();
        }
      });
    },

    isInInput(el) {
      if (!el) return false;
      const tag = el.tagName;
      return tag === 'INPUT' || tag === 'TEXTAREA' || el.isContentEditable;
    },

    show() {
      this.open = true;
      this.query = '';
      this.cursor = 0;
      // focus the input on next tick so x-show has rendered it
      this.$nextTick(() => {
        const input = document.getElementById('palette-input');
        if (input) input.focus();
      });
    },
    close() {
      this.open = false;
    },
    toggle() {
      if (this.open) this.close(); else this.show();
    },

    get all() {
      return [...STATIC_PALETTE_ITEMS, ...dynamicItems()];
    },
    get results() {
      const q = this.query.trim().toLowerCase();
      const out = [];
      for (const item of this.all) {
        if (matches(item, q)) {
          out.push(item);
          if (out.length >= 50) break;
        }
      }
      return out;
    },
    get count() {
      return this.results.length;
    },

    /** Used by x-html in the modal so matches render with <mark>. */
    mark(text) {
      return highlight(text, this.query);
    },

    onInput() {
      this.cursor = 0;
    },
    moveCursor(delta) {
      if (this.count === 0) return;
      this.cursor = (this.cursor + delta + this.count) % this.count;
      this.$nextTick(() => {
        const el = document.querySelector('.palette-result.active');
        if (el) el.scrollIntoView({ block: 'nearest' });
      });
    },
    select(idx) {
      const item = this.results[idx];
      if (!item) return;
      window.location.hash = item.href;
      // Force a hashchange-style scroll even if the hash is already current
      const target = document.querySelector(item.href);
      if (target) target.scrollIntoView({ behavior: 'smooth' });
      this.close();
    },
    onEnter() {
      this.select(this.cursor);
    },
  }));
}
