/**
 * Content registry — sections and pages rendered in order on the single page.
 *
 * To add a new page:
 *   1. Create docs/src/content/<slug>.js exporting { meta, render() }
 *   2. Add an import here
 *   3. Drop it into the `modules` map and into a section's `pages` array
 */

import * as introduction from './introduction.js';
import * as gettingStarted from './getting-started.js';
import * as welcomeScreen from './welcome-screen.js';
import * as configuration from './configuration.js';
import * as providers from './providers.js';
import * as familiars from './familiars.js';

const modules = {
  introduction,
  'getting-started': gettingStarted,
  'welcome-screen': welcomeScreen,
  configuration,
  providers,
  familiars,
};

export const sections = [
  {
    title: 'Overview',
    pages: [
      { path: '/introduction', title: 'Introduction' },
      { path: '/getting-started', title: 'Getting Started' },
      { path: '/welcome-screen', title: 'Welcome Screen' },
    ],
  },
  {
    title: 'Setup',
    pages: [
      { path: '/configuration', title: 'Configuration' },
      { path: '/providers', title: 'Providers' },
    ],
  },
  {
    title: 'Ecosystem',
    pages: [
      { path: '/familiars', title: 'Familiars' },
    ],
  },
];

for (const section of sections) {
  for (const page of section.pages) {
    const key = page.path.slice(1);
    page.load = () => Promise.resolve(modules[key]);
  }
}
