/**
 * App entry point: single-page layout with scroll-spy navigation
 */

import Alpine from 'alpinejs';
import { initSidebar, updateActiveSection } from './sidebar.js';
import { renderHero, fetchStars, formatStars } from './hero.js';
import { processCodeBlocks } from './code-highlight.js';
import { sections } from './content/index.js';
import { registerDemos } from './demos.js';

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
  window.Alpine = Alpine;
  Alpine.start();

  initSidebar();
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
