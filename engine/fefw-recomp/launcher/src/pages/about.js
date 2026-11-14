import { getVersion } from '@tauri-apps/api/app';
import { api, esc, icon, refreshIcons, toast, errText } from '../util.js';
import { upd, checkNow, available } from '../update.js';

const mib = (b) => (b / (1024 * 1024)).toFixed(0);

/** The release notes from their "Changes" part on (the install steps above it are for new players), as light HTML. */
function notesHtml(md) {
  const lines = md.split('\n');
  const from = lines.findIndex((l) => /^(#+\s*)?(\*\*)?(changes|what's new|new)\b/i.test(l.trim()));
  const inline = (s) =>
    esc(s)
      .replace(/\*\*(.+?)\*\*/g, '<strong>$1</strong>')
      .replace(/`([^`]+)`/g, '<code>$1</code>')
      .replace(/\[([^\]]+)\]\([^)]+\)/g, '$1');
  let html = '';
  let list = false;
  for (const raw of lines.slice(from >= 0 ? from : 0)) {
    const l = raw.trim();
    const item = l.match(/^[-*]\s+(.*)$/);
    if (item && !list) (html += '<ul>'), (list = true);
    if (!item && list) (html += '</ul>'), (list = false);
    if (item) html += `<li>${inline(item[1])}</li>`;
    else if (/^#+\s/.test(l)) html += `<p><strong>${inline(l.replace(/^#+\s*/, ''))}</strong></p>`;
    else if (l) html += `<p>${inline(l)}</p>`;
  }
  return html + (list ? '</ul>' : '');
}

function updateCard(cfg) {
  const c = upd.check;
  const s = upd.status;
  const running = s.state === 'running';
  let body;
  if (c && !c.package) {
    body = `<p class="dim">This launcher runs in a checkout of the repository: update it with git.</p>`;
  } else if (s.state === 'done') {
    const rebuild =
      s.rebuild === true
        ? `<div class="banner warn">${icon('hammer')}<div><strong>Set up has to run again</strong> after the restart: this version changes the game build${s.rebuild_reason ? ` (${esc(s.rebuild_reason)})` : ''}. Your saves and settings are kept.</div></div>`
        : s.rebuild === false
          ? `<div class="banner ok">${icon('circle-check')}<div>You do not need to run Set up again. Your saves and settings are kept.</div></div>`
          : '';
    body = `<div class="banner ok">${icon('circle-check')}<div>${esc(s.message)}</div></div>${rebuild}
      <div class="row"><button class="btn primary" id="restart">${icon('rotate-cw')}Restart launcher</button></div>`;
  } else if (running) {
    const pct = s.total ? Math.floor((100 * s.done) / s.total) : null;
    body = `<p class="dim">${esc(s.message)}</p><progress max="100" ${pct === null ? '' : `value="${pct}"`}></progress>
      <div class="row"><button class="btn quiet" id="cancel" ${s.phase === 'download' ? '' : 'disabled'}>Cancel</button></div>`;
  } else if (available()) {
    const r = c.latest;
    body = `<p><strong>Version ${esc(r.version)} is available</strong> <span class="faint">(you have ${esc(c.current)}, download ${mib(r.size)} MiB)</span></p>
      ${r.notes ? `<div class="notes">${notesHtml(r.notes)}</div>` : ''}
      ${r.page ? `<p class="faint">Full notes: ${esc(r.page)}</p>` : ''}
      ${s.state === 'error' ? `<div class="banner danger">${icon('circle-x')}<div>${esc(s.message)}</div></div>` : ''}
      <p class="faint">The launcher downloads and checks the new version, then asks to restart. Your saves, settings and built game stay.</p>
      <div class="row"><button class="btn primary" id="update">${icon('download')}Update to ${esc(r.version)}</button></div>`;
  } else {
    const line = upd.checking
      ? `${icon('loader-circle', 'spin')} Checking for updates`
      : upd.error
        ? `<span class="chip danger">${icon('circle-alert')}${esc(upd.error)}</span>`
        : c
          ? `You have the latest version (${esc(c.current)}).`
          : 'Not checked yet.';
    body = `<p class="dim">${line}</p>`;
  }
  return `
    <div class="card" id="updates">
      <h2>${icon('circle-arrow-up')}Updates</h2>
      <div class="stack">${body}</div>
      <div class="row wrap update-foot">
        <button class="btn" id="check" ${upd.checking || running ? 'disabled' : ''}>${icon('refresh-cw')}Check now</button>
        <label class="switch"><input type="checkbox" id="auto" ${cfg.skip_update_check ? '' : 'checked'}> Check when the launcher starts</label>
      </div>
    </div>`;
}

export async function render(el) {
  let version = '';
  try {
    version = await getVersion();
  } catch {
    version = 'dev';
  }
  let cfg = await api('get_config');
  upd.status = await api('update_status');

  function draw() {
    el.innerHTML = `
    <div class="page">
      <div class="hero">
        <h1>About</h1>
        <p class="dim">Launcher for fefw-recomp, a static recompilation of Fire Emblem: Fortune's Weave (Nintendo Switch 2) for Windows.</p>
      </div>
      ${updateCard(cfg)}
      <div class="card">
        <h2>${icon('info')}fefw recomp launcher</h2>
        <dl class="kv">
          <dt>Version</dt><dd>${version}</dd>
          <dt>Licence</dt><dd>GPL-3.0-or-later</dd>
          <dt>Built with</dt><dd>Tauri 2, Lucide icons (ISC)</dd>
          <dt>Credits</dt><dd>See README.md: nxrecomp, pokopia-recomp and the other projects this builds on.</dd>
        </dl>
      </div>
      <div class="banner warn">
        ${icon('triangle-alert')}
        <div><strong>No game files included.</strong> You need your own legally dumped copy of the game. This project contains no game code or data and is not affiliated with or endorsed by Nintendo or Intelligent Systems. Fire Emblem and Nintendo Switch 2 are trademarks of Nintendo; they are used here only to say which game this works with.</div>
      </div>
    </div>`;
    refreshIcons();
    el.querySelector('#check')?.addEventListener('click', () => checkNow());
    el.querySelector('#auto')?.addEventListener('change', async (ev) => {
      cfg = { ...cfg, skip_update_check: !ev.target.checked };
      await api('save_config', { config: cfg }).catch((e) => toast(errText(e), 'error'));
    });
    el.querySelector('#update')?.addEventListener('click', () =>
      api('start_update', { release: upd.check.latest }).catch((e) => toast(errText(e), 'error')),
    );
    el.querySelector('#cancel')?.addEventListener('click', () => api('cancel_update'));
    el.querySelector('#restart')?.addEventListener('click', () => api('restart_launcher').catch((e) => toast(errText(e), 'error')));
  }

  // redraw on update events while this page is shown
  const listener = () => {
    if (!el.querySelector('#updates')) {
      upd.listeners.delete(listener);
      return;
    }
    const prog = el.querySelector('#updates progress');
    const s = upd.status;
    if (prog && s.state === 'running' && s.phase === 'download' && s.total) {
      prog.value = Math.floor((100 * s.done) / s.total); // no full redraw for every download tick
      prog.previousElementSibling.textContent = s.message;
      return;
    }
    draw();
  };
  upd.listeners.add(listener);
  draw();
}
