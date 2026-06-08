/**
 * App entry point: single-page layout with scroll-spy navigation
 */

import { initSidebar, updateActiveSection } from './sidebar.js';
import { renderHero, fetchStars, formatStars } from './hero.js';
import { processCodeBlocks } from './code-highlight.js';
import { sections } from './content/index.js';

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
