/**
 * Floating navigation with scroll-spy active state
 */

import { sections } from './content/index.js';

let navContainer = null;

export function initSidebar() {
  navContainer = document.getElementById('sidebar-nav');
  renderNav(navContainer);
}

export function updateActiveSection(sectionId) {
  if (!navContainer) return;
  navContainer.querySelectorAll('.nav-link').forEach((link) => {
    link.classList.toggle('active', link.dataset.section === sectionId);
  });
}

function renderNav(container) {
  const isMac = /Mac|iPhone|iPad/.test(navigator.platform);
  const modKey = isMac ? '⌘' : 'Ctrl';

  let html = `
    <button
      type="button"
      class="sidebar-search"
      onclick="window.dispatchEvent(new KeyboardEvent('keydown', { key: 'k', ${isMac ? 'metaKey' : 'ctrlKey'}: true }))"
      aria-label="Open command palette"
    >
      <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
        <circle cx="11" cy="11" r="8"></circle>
        <path d="m21 21-4.3-4.3"></path>
      </svg>
      <span class="sidebar-search-text">Search docs</span>
      <span class="sidebar-search-kbd">${modKey} K</span>
    </button>
  `;
  for (const section of sections) {
    html += `<div class="mb-1.5">`;
    html += `<div class="py-2 font-[var(--font-display)] text-[10.5px] font-semibold uppercase tracking-[0.1em] text-text-dimmer">${section.title}</div>`;
    for (const page of section.pages) {
      const sectionId = page.path.slice(1);
      html += `<a href="#${sectionId}" class="nav-link" data-section="${sectionId}">${page.title}</a>`;
    }
    html += `</div>`;
  }
  container.innerHTML = html;
}
