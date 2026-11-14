import { api, esc, icon, refreshIcons, toast, errText } from '../util.js';

const hex = (c) => '0x' + (c >>> 0).toString(16).toUpperCase().padStart(8, '0');

function statusChip(s) {
  if (s.state === 'running') return `<span class="chip warn">${icon('loader-circle', 'spin')}Running</span>`;
  if (s.state === 'exited') {
    const code = s.code === null || s.code === undefined ? 'no exit code' : `code ${s.code} (${hex(s.code)})`;
    return s.crashed
      ? `<span class="chip danger">${icon('circle-alert')}Stopped abnormally, ${esc(code)}</span>`
      : `<span class="chip ok">${icon('circle-check')}Exited, ${esc(code)}</span>`;
  }
  return '<span class="chip">Ready</span>';
}

function reportHtml(s) {
  if (s.state !== 'exited') return '';
  const files = s.files.filter((f) => s.crashed || f.name === 'fefw.log');
  return `
    <div class="card">
      <h2>${icon(s.crashed ? 'circle-alert' : 'file-text')}${s.crashed ? 'The game did not end cleanly' : 'Last run'}</h2>
      <p class="dim">${esc(s.message)}</p>
      ${files.length ? `<div class="row wrap" style="margin-top: var(--space-4)">${files
        .map((f) => `<button class="btn" data-open="${esc(f.name)}">${icon('file-text')}${esc(f.name)}${f.fresh && f.name !== 'fefw.log' ? ' (new)' : ''}</button>`)
        .join('')}</div>` : ''}
    </div>`;
}

export async function render(el, ctx) {
  const [cfg, ov] = await Promise.all([api('get_config'), api('get_overview')]);
  const setup = ov.repo_ok ? await api('setup_status') : null;
  const report = cfg.dump_dir && ov.repo_ok ? await api('validate_dump', { path: cfg.dump_dir }) : null;
  const chosen = report && report.chosen;
  let status = await api('game_status');

  function draw() {
    const running = status.state === 'running';
    const problems = [];
    if (!ov.repo_ok) problems.push('The repository was not found. Set its folder in Settings.');
    else if (!cfg.dump_dir) problems.push('No game files chosen yet.');
    else if (!chosen) problems.push(`The game files at ${cfg.dump_dir} are not usable: ${report.errors.join(' ')}`);
    else if (!setup.complete) problems.push(setup.progress.state === 'running' ? 'Setup is still running.' : 'Setup is not finished yet: open Setup and press Set up.');
    const ready = problems.length === 0;
    const left = setup ? setup.steps.filter((x) => x.state !== 'done').map((x) => x.title) : [];

    el.innerHTML = `
      <div class="page">
        <div class="hero">
          <h1>Play</h1>
          <p class="dim">A native PC build of the game, recompiled on this PC from your own dump.</p>
        </div>
        ${problems.map((p) => `<div class="banner danger">${icon('circle-x')}<div>${esc(p)}</div></div>`).join('')}
        ${problems.length && ov.repo_ok ? `<div><button class="btn primary" id="to-setup">${icon(chosen ? 'hammer' : 'folder-open')}${chosen ? 'Open Setup' : 'Choose game files'}</button></div>` : ''}

        <div class="row wrap">
          <button class="btn primary big" id="play" ${ready && !running ? '' : 'disabled'}>${icon('play')}${running ? 'Running' : 'Play'}</button>
          ${statusChip(status)}
        </div>
        ${reportHtml(status)}

        <div class="card">
          <h2>${icon('hard-drive')}Install</h2>
          <dl class="kv">
            <dt>Game files</dt>
            <dd>${chosen ? `${chosen.kind === 'update' ? 'Update ' + esc(chosen.version) : chosen.kind === 'base' ? 'Base game' : 'Folder'}<div class="path">${esc(chosen.path)}</div>` : '<span class="faint">not set</span>'}</dd>
            <dt>DLC</dt>
            <dd>${report && report.dlc && report.dlc.length ? `${report.dlc.length} add-on${report.dlc.length === 1 ? '' : 's'} from your dump<div class="path">${esc(report.dlc_dir)}</div>` : '<span class="faint">none in your dump (dlc/&lt;title id&gt;/romfs)</span>'}</dd>
            <dt>Setup</dt>
            <dd>${setup && setup.complete ? 'complete' : `<span class="faint">${left.length ? 'to do: ' + esc(left.join(', ')) : 'not started'}</span>`}</dd>
            <dt>fefw.exe</dt>
            <dd>${ov.exe_exists ? 'built' : '<span class="faint">not built yet</span>'}<div class="path">${esc(ov.exe)}</div></dd>
            <dt>Run folder</dt>
            <dd><span class="faint">fefw.ini, controls.ini, fefw.log and save/</span><div class="path">${esc(ov.game_dir)}</div></dd>
            <dt>Mods</dt>
            <dd><span class="faint">RomFS mods in mods/&lt;name&gt;/romfs, the layout emulators use. A name starting with - or _ turns a mod off; when two mods change the same file, the name sorted last wins. ExeFS (code) patches do not apply.</span></dd>
          </dl>
          <div class="row" style="margin-top: var(--space-4)">
            <button class="btn" id="open-dir" ${ov.repo_ok ? '' : 'disabled'}>${icon('folder-open')}Open run folder</button>
            <button class="btn" id="open-mods" ${ov.repo_ok ? '' : 'disabled'}>${icon('package')}Open mods folder</button>
          </div>
        </div>
        <p class="faint">In the game: F1 opens the runtime's settings overlay. Settings here edit fefw.ini and controls.ini and apply at the next start.</p>
      </div>`;
    refreshIcons();
    el.querySelector('#play')?.addEventListener('click', () => api('play').catch((e) => toast(errText(e), 'error')));
    el.querySelector('#to-setup')?.addEventListener('click', () => ctx.go('setup'));
    el.querySelector('#open-dir')?.addEventListener('click', () => api('open_game_dir').catch((e) => toast(errText(e), 'error')));
    el.querySelector('#open-mods')?.addEventListener('click', () => api('open_mods_dir').catch((e) => toast(errText(e), 'error')));
    el.querySelectorAll('[data-open]').forEach((b) =>
      b.addEventListener('click', () => api('open_game_file', { name: b.dataset.open }).catch((e) => toast(errText(e), 'error'))),
    );
  }

  ctx.onStatus((s) => {
    status = s;
    draw();
  });
  draw();
}
