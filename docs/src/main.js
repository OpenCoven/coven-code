/**
 * App entry point: single-page layout with scroll-spy navigation
 */

import Alpine from 'alpinejs';
import { initSidebar, updateActiveSection } from './sidebar.js';
import { renderHero, fetchStars, formatStars } from './hero.js';
import { processCodeBlocks } from './code-highlight.js';
import { sections } from './content/index.js';
import { registerDemos } from './demos.js';
import { registerPalette } from './palette.js';

async function init() {
  const heroContainer = document.getElementById('hero-container');
  heroContainer.innerHTML = renderHero(null);
  bindCopyBtn();

  const contentEl = document.getElementById('content');
  let html = '';

  for (const section of sections) {
    for (const page of section.pages) {
      const mod = await page.load();
      const sectionId = page.path.slice(1);
      const rendered = mod.render().replace(/href="#\//g, 'href="#');
      html += `<section id="${sectionId}" class="doc-section">${rendered}</section>`;
    }
  }

  contentEl.innerHTML = html;
  processCodeBlocks(contentEl);
  addHeadingAnchors(contentEl);

  // Register Alpine.data() factories for each demo, then start Alpine.
  // Alpine scans the DOM once on start; doing this after innerHTML is set
  // means directives in rendered content modules get bound.
  registerDemos(Alpine);
  registerPalette(Alpine, buildDynamicPaletteItems);
  window.Alpine = Alpine;
  Alpine.start();

  initSidebar();
  initPageToc(contentEl);
  setupScrollSpy();

  function showStars(count) {
    const badge = heroContainer.querySelector('.hero-star-badge');
    if (badge && count) {
      badge.textContent = formatStars(count);
      badge.style.display = '';
    }
  }
  const cachedCount = fetchStars(showStars);
  showStars(cachedCount);

  if (window.location.hash) {
    const target = document.getElementById(window.location.hash.slice(1));
    if (target) {
      setTimeout(() => target.scrollIntoView({ behavior: 'smooth' }), 100);
    }
  }
}

function setupScrollSpy() {
  const sectionEls = document.querySelectorAll('.doc-section');
  const observer = new IntersectionObserver(
    (entries) => {
      for (const entry of entries) {
        if (entry.isIntersecting) {
          updateActiveSection(entry.target.id);
          updatePageToc(entry.target.id);
          break;
        }
      }
    },
    { rootMargin: '-10% 0px -80% 0px' }
  );

  for (const el of sectionEls) {
    observer.observe(el);
  }
}

// --- in-page (right-rail) table of contents --------------------------------
const tocBySection = new Map(); // sectionId -> [{id, text, level}]
let tocContainer = null;
let tocHeadingObserver = null;
let activeTocId = null;

function initPageToc(contentEl) {
  tocContainer = document.getElementById('page-toc');
  if (!tocContainer) return;
  // Build per-section index of H2/H3 headings (slug ids set by addHeadingAnchors)
  for (const sec of contentEl.querySelectorAll('.doc-section')) {
    const items = [];
    for (const h of sec.querySelectorAll('h2, h3')) {
      const id = h.id;
      if (!id) continue;
      // Clone + strip the .heading-anchor so text comes out clean,
      // even when the heading contains nested <code>/<em>/etc.
      const clone = h.cloneNode(true);
      clone.querySelectorAll('.heading-anchor').forEach((n) => n.remove());
      const text = clone.textContent.trim();
      if (!text) continue;
      items.push({ id, text, level: h.tagName === 'H3' ? 3 : 2 });
    }
    tocBySection.set(sec.id, items);
  }
}

function updatePageToc(sectionId) {
  if (!tocContainer) return;
  const items = tocBySection.get(sectionId) || [];
  if (items.length === 0) {
    tocContainer.innerHTML = '';
    return;
  }
  let html = `<div class="page-toc-label">On this page</div>`;
  for (const it of items) {
    html += `<a href="#${it.id}" class="page-toc-link page-toc-l${it.level}" data-toc-id="${it.id}">${it.text}</a>`;
  }
  tocContainer.innerHTML = html;
  setupTocScrollSpy(items);
}

function setupTocScrollSpy(items) {
  if (tocHeadingObserver) tocHeadingObserver.disconnect();
  tocHeadingObserver = new IntersectionObserver(
    (entries) => {
      for (const entry of entries) {
        if (entry.isIntersecting) {
          highlightToc(entry.target.id);
          break;
        }
      }
    },
    { rootMargin: '-15% 0px -70% 0px' }
  );
  for (const it of items) {
    const el = document.getElementById(it.id);
    if (el) tocHeadingObserver.observe(el);
  }
}

function highlightToc(headingId) {
  if (!tocContainer || headingId === activeTocId) return;
  activeTocId = headingId;
  tocContainer.querySelectorAll('.page-toc-link').forEach((link) => {
    link.classList.toggle('active', link.dataset.tocId === headingId);
  });
}

/**
 * Builds the runtime portion of the palette index: every section in the
 * sidebar plus every H2/H3 we slugged in addHeadingAnchors. Called via
 * a callback from the palette factory each time `all` is computed.
 */
function buildDynamicPaletteItems() {
  const items = [];
  for (const section of sections) {
    for (const page of section.pages) {
      const id = page.path.slice(1);
      items.push({
        kind: 'Section',
        label: page.title,
        category: section.title,
        desc: `Top of the ${page.title} section`,
        href: `#${id}`,
      });
      const headings = tocBySection.get(id) || [];
      for (const h of headings) {
        items.push({
          kind: 'Heading',
          label: h.text,
          category: page.title,
          desc: `Sub-section under ${page.title}`,
          href: `#${h.id}`,
        });
      }
    }
  }
  return items;
}

/**
 * Walk every H2 / H3 inside a .doc-section, generate stable slug ids
 * ("<section-id>-<slug>"), and attach an anchor-link button that reveals
 * on hover. Lets readers deep-link into sub-sections.
 */
function addHeadingAnchors(container) {
  const sections = container.querySelectorAll('.doc-section');
  for (const sec of sections) {
    const sectionId = sec.id;
    const seen = new Set();
    const headings = sec.querySelectorAll('h2, h3');
    for (const h of headings) {
      const text = h.textContent.trim();
      let slug = text
        .toLowerCase()
        .replace(/[^a-z0-9]+/g, '-')
        .replace(/^-|-$/g, '');
      if (!slug) continue;
      let id = `${sectionId}-${slug}`;
      let n = 1;
      while (seen.has(id)) id = `${sectionId}-${slug}-${++n}`;
      seen.add(id);
      h.id = id;

      const link = document.createElement('a');
      link.className = 'heading-anchor';
      link.href = `#${id}`;
      link.setAttribute('aria-label', `Link to ${text}`);
      link.textContent = '#';
      h.appendChild(link);
    }
  }
}

function bindCopyBtn() {
  const btn = document.getElementById('hero-copy-btn');
  if (!btn) return;
  btn.addEventListener('click', () => {
    navigator.clipboard.writeText('npm i -g coven-code').then(() => {
      btn.querySelector('.hero-copy-icon').classList.add('hidden');
      btn.querySelector('.hero-check-icon').classList.remove('hidden');
      setTimeout(() => {
        btn.querySelector('.hero-copy-icon').classList.remove('hidden');
        btn.querySelector('.hero-check-icon').classList.add('hidden');
      }, 1500);
    });
  });
}

init();
