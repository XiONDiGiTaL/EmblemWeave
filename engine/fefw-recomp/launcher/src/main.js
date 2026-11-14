import './theme.css';
import './layout.css';
import './components.css';
import { listen } from '@tauri-apps/api/event';
import { api, icon, refreshIcons, errText, toast } from './util.js';
import * as play from './pages/play.js';
import * as settings from './pages/settings.js';
import * as setup from './pages/setup.js';
import * as about from './pages/about.js';
import { upd, notify, checkNow, available } from './update.js';

const PAGES = {
  play: { mod: play, label: 'Play', icon: 'play' },
  setup: { mod: setup, label: 'Setup', icon: 'hammer' },
  settings: { mod: settings, label: 'Settings', icon: 'settings' },
  about: { mod: about, label: 'About', icon: 'info' },
};

const nav = document.getElementById('nav');
const view = document.getElementById('view');
let current = null;
let statusHandler = null;
let setupHandler = null;
let logHandler = null;

const ctx = {
  go,
  onStatus(fn) { statusHandler = fn; },
  onSetup(fn) { setupHandler = fn; },
  onLog(fn) { logHandler = fn; },
};

function drawNav() {
  nav.innerHTML =
    `<div class="brand"><img class="mark" src="/icon.png" alt=""><span class="name">fefw recomp</span></div>` +
    Object.entries(PAGES)
      .map(([id, p]) => `<button class="nav-item ${id === current ? 'active' : ''}" data-page="${id}">${icon(p.icon)}<span class="label">${p.label}</span></button>`)
      .join('') +
    (available() || upd.status.state === 'done'
      ? `<button class="nav-item update-pill" data-page="about">${icon('circle-arrow-up')}<span class="label">${
          upd.status.state === 'done' ? 'Restart to update' : `Update to ${upd.check.latest.version}`
        }</span></button>`
      : '');
  nav.querySelectorAll('[data-page]').forEach((b) => b.addEventListener('click', () => go(b.dataset.page)));
  refreshIcons();
}

async function go(page, opts = {}) {
  current = page;
  statusHandler = null;
  setupHandler = null;
  logHandler = null;
  drawNav();
  view.scrollTop = 0;
  try {
    await PAGES[page].mod.render(view, { ...ctx, ...opts });
  } catch (e) {
    view.innerHTML = `<div class="page"><div class="banner danger">${icon('circle-x')}<div>${errText(e)}</div></div></div>`;
    toast(errText(e), 'error');
  }
  refreshIcons();
}

async function boot() {
  await listen('game-status', (ev) => statusHandler && statusHandler(ev.payload));
  await listen('setup-status', (ev) => setupHandler && setupHandler(ev.payload));
  await listen('setup-log', (ev) => logHandler && logHandler(ev.payload));
  await listen('update-status', (ev) => {
    upd.status = ev.payload;
    notify();
  });
  upd.listeners.add(drawNav);
  let cfg = { dump_dir: '' };
  try {
    cfg = await api('get_config');
  } catch (e) {
    toast(errText(e), 'error');
  }
  // first run: no game files chosen yet
  go(cfg.dump_dir ? 'play' : 'setup', { firstRun: !cfg.dump_dir });
  if (!cfg.skip_update_check) checkNow(); // quietly: a failure only shows on the About page
}

boot();
