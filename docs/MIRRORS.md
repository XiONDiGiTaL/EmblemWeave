# Mirrors and backups

The project lives in more than one place on purpose. Recompilation projects have been removed from hosting before, and
a landing page that only exists in the repository disappears with it. This page records where each copy is, what it
holds, and how to update it.

| Copy | What it holds | Updated by |
| --- | --- | --- |
| [GitHub](https://github.com/XiONDiGiTaL/EmblemWeave) | source, releases, issues, the Pages site | `git push` |
| [GitLab](https://gitlab.com/XiONDiGiTaL/emblemweave) | source and changelog | `git push gitlab main` (token required once) |
| [Fallback page](https://emblemweave.xiondigital.workers.dev) | one static page: what the project is, where the mirrors are | `fallback/index.html`, redeployed to Cloudflare |
| Cloudflare R2 `emblemweave-backup` | bucket for source and site snapshots | `wrangler r2 object put`, or the Cloudflare tools |
| GitGud | source, once an account exists | `git push gitgud main` |

The Pages site is served from this repository's `docs/` folder at
[xiondigital.net/EmblemWeave](https://xiondigital.net/EmblemWeave/), and `docs/mirrors.json` is the list the page reads.
When a mirror is added or removed, edit that file: the page follows it, so the copy in the repository and the copy on the
site cannot drift apart.

## GitLab

The project is created and empty. The first push needs a GitLab personal access token with `write_repository`:

```sh
git push https://oauth2:<token>@gitlab.com/XiONDiGiTaL/emblemweave.git main
```

Use the token once, then set the remote back to the plain URL (`git remote set-url gitlab https://gitlab.com/XiONDiGiTaL/emblemweave.git`)
so it does not stay in `.git/config`. Do not commit a token, and do not put one in a script.

## GitGud

An account exists but no repository has been created from here. Once it exists, add it as a remote and push the same
way. Nothing in the build depends on it; it is a second place to find the source.

## Fallback page

`fallback/index.html` is a single self-contained file (no scripts, no fonts, no images). It is deployed as a Worker with
static assets named `emblemweave`, which serves it at `emblemweave.xiondigital.workers.dev`. To change the text, edit the
file, commit it, and deploy it again through the Cloudflare integration; the Worker reads from the asset bundle, not from
the repository.

The page is deliberately small: it is what is left if GitHub, the domain and Pages are all unavailable, so it must not
depend on any of them.

## Keeping a local copy

```sh
git clone --mirror https://github.com/XiONDiGiTaL/EmblemWeave.git emblemweave.git
```

A mirror clone holds every branch and tag. Put it on a drive that is not the machine you build on, and refresh it with
`git remote update` occasionally. The zip on the release page is a build, not the source.
