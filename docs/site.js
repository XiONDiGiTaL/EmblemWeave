/* EmblemWeave site: arcade sound effects (synthesised, no asset files),
   the current release, and a live check of the source mirrors. */

(() => {
  'use strict';

  const reduced = window.matchMedia('(prefers-reduced-motion: reduce)').matches;

  /* ---------- sound ----------
     Small WebAudio blips: nothing to download, no tracking, and off with one
     button (the choice is remembered). A browser only lets audio start after
     a gesture, so the context is made on the first pointer or key press. */

  const SOUND_KEY = 'ew.sound';
  let enabled = localStorage.getItem(SOUND_KEY) !== 'off';
  let ctx = null;

  function audio() {
    if (!enabled) return null;
    if (!ctx) {
      const C = window.AudioContext || window.webkitAudioContext;
      if (!C) return null;
      ctx = new C();
    }
    if (ctx.state === 'suspended') ctx.resume();
    return ctx;
  }

  function tone(freq, when, dur, type = 'sine', gain = 0.045, glide = 0) {
    const a = audio();
    if (!a) return;
    const t0 = a.currentTime + when;
    const osc = a.createOscillator();
    const g = a.createGain();
    osc.type = type;
    osc.frequency.setValueAtTime(freq, t0);
    if (glide) osc.frequency.exponentialRampToValueAtTime(Math.max(40, freq + glide), t0 + dur);
    g.gain.setValueAtTime(0.0001, t0);
    g.gain.exponentialRampToValueAtTime(gain, t0 + 0.008);
    g.gain.exponentialRampToValueAtTime(0.0001, t0 + dur);
    osc.connect(g).connect(a.destination);
    osc.start(t0);
    osc.stop(t0 + dur + 0.02);
  }

  const sfx = {
    hover() { tone(880, 0, 0.05, 'sine', 0.018); },
    click() { tone(520, 0, 0.06, 'square', 0.025); tone(780, 0.05, 0.07, 'sine', 0.02); },
    coin() {
      tone(988, 0, 0.07, 'square', 0.03);
      tone(1319, 0.07, 0.18, 'square', 0.026);
      tone(1976, 0.12, 0.22, 'sine', 0.012);
    },
    fanfare() {
      [523, 659, 784, 1047].forEach((f, i) => tone(f, i * 0.07, 0.16, 'triangle', 0.02));
    },
  };

  const soundButton = document.getElementById('sound');
  function paintSound() {
    soundButton.setAttribute('aria-pressed', String(enabled));
    soundButton.title = enabled ? 'Sound on' : 'Sound off';
  }
  paintSound();
  soundButton.addEventListener('click', () => {
    enabled = !enabled;
    localStorage.setItem(SOUND_KEY, enabled ? 'on' : 'off');
    if (enabled) sfx.click();
    paintSound();
  });

  // Every element that asks for a sound.
  document.querySelectorAll('[data-sfx]').forEach((el) => {
    el.addEventListener('pointerenter', () => !reduced && sfx[el.dataset.sfx]?.());
    el.addEventListener('click', () => sfx.click());
  });
  document.getElementById('logo')?.addEventListener('pointerenter', () => !reduced && sfx.fanfare(), { once: true });

  /* ---------- version ---------- */

  fetch('https://api.github.com/repos/XiONDiGiTaL/EmblemWeave/releases/latest', { headers: { Accept: 'application/vnd.github+json' } })
    .then((r) => (r.ok ? r.json() : null))
    .then((rel) => {
      if (!rel || !rel.tag_name) return;
      const v = String(rel.tag_name);
      const chip = document.getElementById('version');
      if (chip) chip.textContent = v;
      const badge = document.getElementById('badge-release');
      if (badge) badge.textContent = 'release ' + v.replace(/^v/, '');
    })
    .catch(() => { /* offline or rate-limited: the static version stays */ });

  /* ---------- mirrors ----------
     mirrors.json lists the copies of the repository. Each one is checked with a
     HEAD request (no CORS on GitHub or GitLab homes, so the request uses
     no-cors and only proves the host answered). The point is the list itself:
     if GitHub is gone, this page is gone too, and a mirror is where to look. */

  const list = document.getElementById('mirror-list');

  function addMirror(name, url, note) {
    const li = document.createElement('li');
    const a = document.createElement('a');
    a.href = url;
    a.textContent = name;
    a.rel = 'noopener';
    const why = document.createElement('span');
    why.className = 'why';
    why.textContent = note || '';
    const state = document.createElement('span');
    state.className = 'state';
    state.textContent = 'checking';
    li.append(a, why, state);
    list.append(li);
    return state;
  }

  function check(state, url) {
    const ctrl = new AbortController();
    const t = setTimeout(() => ctrl.abort(), 6000);
    fetch(url, { mode: 'no-cors', signal: ctrl.signal, cache: 'no-store' })
      .then(() => {
        state.textContent = 'reachable';
        state.classList.add('up');
      })
      .catch(() => {
        state.textContent = 'no answer';
        state.classList.add('down');
      })
      .finally(() => clearTimeout(t));
  }

  fetch('mirrors.json', { cache: 'no-store' })
    .then((r) => (r.ok ? r.json() : null))
    .then((data) => {
      list.textContent = '';
      if (!data || !Array.isArray(data.mirrors) || !data.mirrors.length) {
        addMirror('GitHub', 'https://github.com/XiONDiGiTaL/EmblemWeave', 'the primary copy').textContent = 'unknown';
        return;
      }
      for (const m of data.mirrors) {
        const state = addMirror(m.name, m.url, m.note);
        check(state, m.url);
      }
    })
    .catch(() => {
      list.textContent = '';
      addMirror('GitHub', 'https://github.com/XiONDiGiTaL/EmblemWeave', 'the primary copy').textContent = 'unknown';
    });
})();
