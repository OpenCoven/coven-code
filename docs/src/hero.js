/**
 * Hero section with large logo, tagline, install command, and GitHub stars
 */

import { fetchGithubStarCount } from '../shared/githubStars.js';

export function renderHero(starCount) {
  return `
    <div class="hero relative pt-20 pb-16 px-8 overflow-hidden max-sm:pt-14 max-sm:pb-12 max-sm:px-5">
      <div class="hero-scanlines absolute inset-0 pointer-events-none z-0"></div>
      <div class="hero-grid relative z-1">
        <div class="hero-copy">
          <div class="hero-kicker" style="animation: fade-up 0.7s cubic-bezier(0.16,1,0.3,1) both">Field manual / coven-code</div>
          <p class="hero-title" style="animation: fade-up 0.7s 0.1s cubic-bezier(0.16,1,0.3,1) both">A terminal coding agent in Rust</p>
          <p class="hero-subtitle" style="animation: fade-up 0.7s 0.2s cubic-bezier(0.16,1,0.3,1) both">Open-source agent for your terminal. 40+ tools, 15+ LLM providers, multi-account auth, plugin system, and native integration with the Coven daemon.</p>
          <div class="hero-actions" style="animation: fade-up 0.7s 0.3s cubic-bezier(0.16,1,0.3,1) both">
            <a href="#getting-started" class="hero-btn-primary inline-flex items-center gap-2 px-7 h-10 rounded-[10px] font-[var(--font-body)] text-sm font-semibold cursor-pointer max-sm:w-full max-sm:justify-center max-sm:max-w-[280px]">Read the field manual</a>
            <a href="https://github.com/OpenCoven/coven-code" target="_blank" rel="noopener" class="hero-secondary-action inline-flex items-center gap-2 px-5 h-10 rounded-[10px] font-[var(--font-body)] text-sm font-semibold cursor-pointer max-sm:w-full max-sm:justify-center max-sm:max-w-[280px]">
              <svg viewBox="0 0 24 24" width="15" height="15" fill="currentColor" stroke="none"><path d="M12 2l3.09 6.26L22 9.27l-5 4.87 1.18 6.88L12 17.77l-6.18 3.25L7 14.14 2 9.27l6.91-1.01L12 2z"/></svg>
              Star
              <span class="hero-star-badge px-1.5 py-px rounded-lg text-[11.5px] font-semibold tabular-nums" ${starCount ? '' : 'style="display:none"'}>${starCount ? formatStars(starCount) : ''}</span>
            </a>
            <button id="hero-copy-btn" title="Copy to clipboard" class="hero-install-btn group inline-flex items-center gap-2.5 rounded-[10px] px-5 h-10 cursor-pointer">
              <code class="font-[var(--font-mono)] text-sm font-medium text-accent tracking-[-0.02em] !bg-transparent !border-0 !p-0">npm i -g coven-code</code>
              <svg class="hero-copy-icon text-text-dimmer group-hover:text-accent transition-colors" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><rect x="9" y="9" width="13" height="13" rx="2"/><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/></svg>
              <svg class="hero-check-icon hidden text-text-secondary" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5"><polyline points="20 6 9 17 4 12"/></svg>
            </button>
          </div>
          <div class="hero-proof-strip" style="animation: fade-up 0.7s 0.38s cubic-bezier(0.16,1,0.3,1) both">
            <span>Rust-native</span>
            <span>multi-provider</span>
            <span>daemon-aware</span>
          </div>
        </div>
        <aside class="hero-manual" aria-label="Coven Code quickstart" style="animation: fade-up 0.7s 0.18s cubic-bezier(0.16,1,0.3,1) both">
          <div class="manual-topline">
            <span>coven-code / live ops</span>
            <span>docs online</span>
          </div>
          <div class="manual-window">
            <div class="manual-row manual-row-active">
              <span class="manual-index">01</span>
              <span>Install with npm or cargo</span>
            </div>
            <div class="manual-row">
              <span class="manual-index">02</span>
              <span>Authenticate or set an API key</span>
            </div>
            <div class="manual-row">
              <span class="manual-index">03</span>
              <span>Pick a provider, model, familiar</span>
            </div>
            <div class="manual-row">
              <span class="manual-index">04</span>
              <span>Run a goal or chat in the TUI</span>
            </div>
          </div>
          <div class="manual-doc-links">
            <a href="#welcome-screen">Welcome screen</a>
            <a href="#providers">Providers</a>
            <a href="#familiars">Familiars</a>
          </div>
          <div class="manual-command">
            <span>$</span>
            <code>coven-code</code>
          </div>
        </aside>
      </div>
    </div>
  `;
}

export function formatStars(count) {
  if (count >= 1000) return (count / 1000).toFixed(1).replace(/\.0$/, '') + 'k';
  return String(count);
}

const CACHE_KEY = 'coven_code_gh_stars_v1';
const REFETCH_INTERVAL = 60000;

export function fetchStars(onUpdate) {
  let cached = null;
  try {
    const raw = sessionStorage.getItem(CACHE_KEY);
    if (raw) cached = JSON.parse(raw);
  } catch {}

  if (cached && cached.count != null) {
    if (Date.now() - cached.ts > REFETCH_INTERVAL) {
      fetchFresh().then((count) => {
        if (count && count !== cached.count && onUpdate) onUpdate(count);
      });
    }
    return cached.count;
  }

  fetchFresh().then((count) => {
    if (count && onUpdate) onUpdate(count);
  });
  return null;
}

async function fetchFresh() {
  let count = null;
  try {
    const res = await fetch('/api/stars');
    const ct = (res.headers.get('content-type') || '').toLowerCase();
    if (res.ok && ct.includes('application/json')) {
      const data = await res.json();
      if (data.stars != null) count = data.stars;
    }
  } catch {}

  if (count == null) {
    try {
      count = await fetchGithubStarCount(fetch, null);
    } catch {}
  }

  if (count != null) {
    try { sessionStorage.setItem(CACHE_KEY, JSON.stringify({ count, ts: Date.now() })); } catch {}
  }
  return count;
}
