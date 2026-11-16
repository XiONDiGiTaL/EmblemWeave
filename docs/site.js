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
    lightbox() {
      tone(659, 0, 0.06, 'sine', 0.02);
      tone(988, 0.055, 0.12, 'triangle', 0.018);
    },
  };

  // A hover is not a user gesture a browser accepts for audio. Once the player
  // clicks or presses a key anywhere, resume the context so later hovers play.
  const unlockAudio = () => audio();
  document.addEventListener('pointerdown', unlockAudio, { once: true });
  document.addEventListener('keydown', unlockAudio, { once: true });

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

  // Every element that asks for a sound. Cards ask for the hover sound only,
  // since clicking them has no action of its own.
  document.querySelectorAll('[data-sfx]').forEach((el) => {
    el.addEventListener('pointerenter', () => !reduced && sfx[el.dataset.sfx]?.());
    el.addEventListener('click', () => sfx.click());
  });
  document.querySelectorAll('[data-sfx-hover]').forEach((el) => {
    el.addEventListener('pointerenter', () => !reduced && sfx[el.dataset.sfxHover]?.());
  });
  document.getElementById('logo')?.addEventListener('pointerenter', () => !reduced && sfx.fanfare(), { once: true });

  /* ---------- screenshot viewer ----------
     A click opens the original file in a native dialog, not the gallery
     thumbnail. The anchors keep their href, so the image still opens when
     JavaScript is unavailable. */

  const lightbox = document.getElementById('lightbox');
  const lightboxImg = document.getElementById('lightbox-img');
  const lightboxCaption = document.getElementById('lightbox-caption');
  const shots = Array.from(document.querySelectorAll('.shot'));
  let shotIndex = 0;
  let returnFocus = null;

  function paintShot(index) {
    if (!lightbox || !lightboxImg || !shots.length) return;
    shotIndex = (index + shots.length) % shots.length;
    const shot = shots[shotIndex];
    const image = shot.querySelector('img');
    const caption = shot.closest('figure')?.querySelector('figcaption');
    lightboxImg.src = shot.getAttribute('href');
    lightboxImg.alt = image?.alt || '';
    if (lightboxCaption) lightboxCaption.textContent = caption?.textContent?.trim() || image?.alt || '';
  }

  shots.forEach((shot, index) => {
    shot.addEventListener('click', (event) => {
      if (!lightbox || typeof lightbox.showModal !== 'function') return;
      event.preventDefault();
      returnFocus = document.activeElement;
      paintShot(index);
      lightbox.showModal();
      if (!reduced) sfx.lightbox();
    });
  });

  lightbox?.querySelector('[data-close]')?.addEventListener('click', () => lightbox.close());
  lightbox?.addEventListener('click', (event) => {
    if (event.target === lightbox) lightbox.close();
  });
  lightbox?.addEventListener('close', () => {
    if (returnFocus && typeof returnFocus.focus === 'function') returnFocus.focus();
  });
  document.addEventListener('keydown', (event) => {
    if (!lightbox?.open) return;
    if (event.key === 'ArrowRight') {
      event.preventDefault();
      paintShot(shotIndex + 1);
      sfx.click();
    } else if (event.key === 'ArrowLeft') {
      event.preventDefault();
      paintShot(shotIndex - 1);
      sfx.click();
    }
  });

  /* ---------- version and download ----------
     release.json is stamped by the release workflow; the GitHub API is the
     fallback. The badge, the download and the stylesheet query all move
     together, so a release never needs a badge edit and a cached site.css
     cannot outlive the release that changed it. */

  // Compare release numbers without treating the cache revision as a release.
  const releaseRank = (value) => {
    const p = String(value).split(/[.-]/).map((n) => parseInt(n, 10) || 0);
    return (p[0] || 0) * 1000000 + (p[1] || 0) * 1000 + (p[2] || 0);
  };

  function paintRelease(rel, fromApi) {
    if (!rel) return;
    const version = String(rel.version || rel.tag_name || '').replace(/^v/, '');
    const tag = String(rel.tag || rel.tag_name || (version ? 'v' + version : ''));
    if (!version) return;
    const chip = document.getElementById('version');
    if (chip) chip.textContent = 'v' + version;
    const badge = document.getElementById('badge-release');
    if (badge) badge.textContent = 'release ' + version;
    document.documentElement.dataset.release = version;

    // The stylesheet URL carries the release, so a new release cannot be served
    // with an old cached stylesheet. Stamped HTML already has it; this covers a
    // page deployed before release.json caught up.
    const css = document.getElementById('site-css');
    if (css) {
      const current = css.getAttribute('href') || '';
      const currentVersion = (current.match(/[?&]v=([^&]+)/) || [])[1] || '';
      // Keep a cache revision such as 0.1.34-2: it belongs to this release but
      // the page has not necessarily been stamped again for it. Do not let an
      // older GitHub release downgrade the stylesheet while that revision is
      // what carries the current styles.
      if (!currentVersion || releaseRank(version) > releaseRank(currentVersion)) {
        css.setAttribute('href', 'site.css?v=' + encodeURIComponent(version));
      }
    }

    const download = document.getElementById('download');
    if (download) {
      const assets = Array.isArray(rel.assets) ? rel.assets : [];
      const zip = assets.find((a) => /(windows|win)/i.test(a.name || '') && /\.zip$/i.test(a.name || '')) ||
        assets.find((a) => /\.zip$/i.test(a.name || ''));
      if (zip && zip.browser_download_url) download.href = zip.browser_download_url;
      else if (rel.download) download.href = rel.download;
      else if (tag) download.href = 'https://github.com/XiONDiGiTaL/EmblemWeave/releases/tag/' + encodeURIComponent(tag);
      if (fromApi && rel.published_at) download.title = 'Published ' + rel.published_at.slice(0, 10);
    }
  }

  fetch('release.json', { cache: 'no-store' })
    .then((r) => (r.ok ? r.json() : null))
    .then((local) => {
      paintRelease(local, false);
      return fetch('https://api.github.com/repos/XiONDiGiTaL/EmblemWeave/releases/latest', { headers: { Accept: 'application/vnd.github+json' } })
        .then((r) => (r.ok ? r.json() : null))
        .then((api) => {
          if (api && api.tag_name) paintRelease(api, true);
        });
    })
    .catch(() => { /* offline or rate-limited: the stamped version stays */ });

  /* ---------- mirrors ----------
     mirrors.json lists the copies of the repository. Each one is checked with a
     HEAD request (no CORS on GitHub or GitLab homes, so the request uses
     no-cors and only proves the host answered). The point is the list itself:
     if GitHub is gone, this page is gone too, and a mirror is where to look. */

  const list = document.getElementById('mirror-list');

  // The HTML carries the mirrors that must remain visible when release.json
  // or this script fails. A fetch that fails leaves those links in place.
  function keepStaticMirrors() {
    if (!list) return;
    list.querySelectorAll('.state').forEach((state) => {
      state.textContent = 'not checked';
      state.classList.remove('up', 'down');
    });
  }

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
      if (!data || !Array.isArray(data.mirrors) || !data.mirrors.length) {
        keepStaticMirrors();
        return;
      }
      list.textContent = '';
      for (const m of data.mirrors) {
        const state = addMirror(m.name, m.url, m.note);
        check(state, m.url);
      }
    })
    .catch(keepStaticMirrors);
})();
