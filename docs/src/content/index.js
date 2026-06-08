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
import * as installation from './installation.js';
import * as welcomeScreen from './welcome-screen.js';
import * as configuration from './configuration.js';
import * as auth from './auth.js';
import * as providers from './providers.js';
import * as commands from './commands.js';
import * as keybindings from './keybindings.js';
import * as tools from './tools.js';
import * as agents from './agents.js';
import * as familiars from './familiars.js';
import * as mcp from './mcp.js';
import * as plugins from './plugins.js';
import * as hooks from './hooks.js';
import * as advanced from './advanced.js';

const modules = {
  introduction,
  'getting-started': gettingStarted,
  installation,
  'welcome-screen': welcomeScreen,
  configuration,
  auth,
  providers,
  commands,
  keybindings,
  tools,
  agents,
  familiars,
  mcp,
  plugins,
  hooks,
  advanced,
};

export const sections = [
  {
    title: 'Overview',
    pages: [
      { path: '/introduction', title: 'Introduction' },
      { path: '/getting-started', title: 'Getting Started' },
      { path: '/installation', title: 'Installation' },
      { path: '/welcome-screen', title: 'Welcome Screen' },
    ],
  },
  {
    title: 'Setup',
    pages: [
      { path: '/configuration', title: 'Configuration' },
      { path: '/auth', title: 'Authentication' },
      { path: '/providers', title: 'Providers' },
    ],
  },
  {
    title: 'Reference',
    pages: [
      { path: '/commands', title: 'Slash Commands' },
      { path: '/keybindings', title: 'Keybindings' },
      { path: '/tools', title: 'Tools' },
    ],
  },
  {
    title: 'Agents',
    pages: [
      { path: '/agents', title: 'Agents' },
      { path: '/familiars', title: 'Familiars' },
    ],
  },
  {
    title: 'Extending',
    pages: [
      { path: '/mcp', title: 'MCP' },
      { path: '/plugins', title: 'Plugins' },
      { path: '/hooks', title: 'Hooks' },
    ],
  },
  {
    title: 'Advanced',
    pages: [
      { path: '/advanced', title: 'Advanced' },
    ],
  },
];

for (const section of sections) {
  for (const page of section.pages) {
    const key = page.path.slice(1);
    page.load = () => Promise.resolve(modules[key]);
  }
}
