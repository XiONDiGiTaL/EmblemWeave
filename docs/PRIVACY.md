# Privacy

EmblemWeave is a local desktop launcher. It has no account system, no
telemetry, no analytics, no advertising identifier, and no automatic crash
upload.

## What stays on your PC

Settings, saves, mods, logs, decoded game modules, build output, and diagnostic
reports live in the launcher's data folder. In a release zip that folder is
`data` beside `EmblemWeave.exe`; an older profile install uses
`%APPDATA%\EmblemWeave`. The game's own log is in the run folder
(`data\play\fefw.log` or `%APPDATA%\EmblemWeave\play\fefw.log`).

The game folder you choose is only read. EmblemWeave does not upload it.

## Network connections

The launcher makes a connection only for an action you ask for:

| Destination | When | What is sent |
| --- | --- | --- |
| `api.github.com`, `github.com`, GitHub's release CDN | update check and update download | a release-list request or the zip; no account token and nothing about your game |
| `gamebanana.com` | browsing or installing a mod you chose | the page or file id you asked for |
| `git.ryujinx.app` | building the shader translator | a filtered clone at the pinned revision; no credentials |
| `pypi.org` through `uv` | the engine's Python build step | package downloads pinned in the engine's requirements |

No telemetry is attached to those requests. The site at
`xiondigital.net/EmblemWeave` is a static page and does not include analytics or
advertising scripts from this project.

## Discord Rich Presence

Rich Presence is optional; the toggle is on **About**. When it is on, the
launcher and the game send a small activity message to the Discord desktop
client on the same PC over Discord's local IPC pipe. It contains the public
EmblemWeave application id, whether the launcher or game is running, a start
time and the local process id. It does not contain your game path, saves,
account name, game data or a Discord token. The launcher does not contact
Discord's servers directly and does not use a bot or OAuth account.

Discord displays that activity to you and, depending on your Discord privacy
settings, to your friends. Discord handles it under Discord's own privacy
policy. EmblemWeave stores only the local on/off setting.

## Diagnostic reports and issues

**Troubleshooter > Create report bundle** writes a `.zip` on your PC. Nothing is
uploaded. The bundle can contain:

- the launcher version, platform, and storage mode,
- your launcher settings, including chosen folders and game paths,
- recent task logs and the game's `fefw.log`, `fefw_freeze.txt`,
  `fefw_fatal.txt`, or crash report,
- local Windows paths, which can include your Windows user name.

If you attach a bundle to a GitHub issue, GitHub stores it under its own terms
and it becomes publicly visible unless the repository is private. Review the
bundle before attaching it. Remove saves, game files, keys, tokens, or anything
you do not want to publish.

The issue description you write is sent to GitHub only when you press the button
that opens the new-issue page and then submit the form. EmblemWeave does not
post it for you.

## Children and personal data

The launcher does not ask for a name, email address, birth date, or account. The
optional profile name is stored locally and is not sent anywhere. Do not put
personal information into a public issue.

## Contact

Privacy questions can be opened as a GitHub issue, or raised privately through
the repository's **Security** tab when the matter is sensitive.
