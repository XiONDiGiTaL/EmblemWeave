# Cloudflare fallback

This folder is the source for the static fallback at
<https://emblemweave.xiondigital.workers.dev>. It is a separate Cloudflare Worker
named `emblemweave`, not the GitHub Pages site.

| File | Purpose |
| --- | --- |
| `index.html` | The self-contained fallback page. No scripts, cookies or tracking. |
| `release.json` | The public release metadata served at `/release.json`. |
| `worker.js` | Serves known assets and returns a clean 404 for every other path. |

The Worker uses static assets with `html_handling: auto-trailing-slash` and
`not_found_handling: none`. Deploy the three files together. The Worker source is
uploaded as `worker.js`; `index.html` and `release.json` are the only public
assets.

## At a release

1. Set `index.html` and `release.json` to the published version and the exact
   Windows zip URL.
2. Commit and push the source.
3. Deploy the asset set to the `emblemweave` Worker.
4. Check `https://emblemweave.xiondigital.workers.dev/`,
   `/release.json`, and a deliberately missing path (expect 404).
5. Do not announce the release from the fallback before the GitHub release and
   the site are live.
