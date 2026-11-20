# Publishes a release from one source: docs/changelog/<version>.md.
#
# That file is the prose. This script packages the Windows build (tools/package_release.ps1), writes the release notes
# from the changelog file plus the install steps and the hash it just measured, updates the changelog index, commits the
# changelog and the index row, pushes, and creates the GitHub release with the zip and its tag on that commit. The tag
# workflow builds the Linux translator, stamps the site and runs the live smoke check. Nothing is typed twice.
#
#   pwsh -File tools/release.ps1 -Version 0.1.15                  # build, package, write notes, publish
#   pwsh -File tools/release.ps1 -Version 0.1.15 -SkipBuild       # reuse target\release\EmblemWeave.exe
#   pwsh -File tools/release.ps1 -Version 0.1.15 -Local           # everything except the GitHub release
#
# A changelog file starts with its title and a one-line summary the index uses:
#
#   # EmblemWeave 0.1.15
#
#   > One line about the release.
#
#   ## What's new
#   ...
param(
    [Parameter(Mandatory = $true)][string]$Version,
    [switch]$SkipBuild,
    [switch]$Local
)
$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$changelog = Join-Path $repo "docs\changelog\$Version.md"
$index = Join-Path $repo 'docs\changelog\README.md'
Push-Location $repo
try {
    if (-not (Test-Path $changelog)) {
        throw "no changelog for ${Version}: write docs\changelog\$Version.md first, it is the release notes"
    }

    # 1. Read the summary and the body the notes are made of. A summary may wrap across several `> ` lines; the index
    # row needs it on one line, so join the blockquote paragraph instead of taking only its first line.
    $lines = [System.IO.File]::ReadAllLines($changelog)
    if (-not $lines[0].StartsWith('# ')) { throw "docs\changelog\${Version}.md must start with '# EmblemWeave $Version'" }
    $summaryLines = @()
    $bodyStart = 1
    for ($i = 1; $i -lt $lines.Count; $i++) {
        if ($lines[$i].Trim() -eq '') { continue }
        if ($lines[$i].StartsWith('> ')) {
            $summaryLines += $lines[$i].Substring(2).Trim()
            $bodyStart = $i + 1
            continue
        }
        break
    }
    $summary = ($summaryLines -join ' ').Trim()
    if (-not $summary) { throw "docs\changelog\${Version}.md needs a '> one line' summary under its title, for the index" }
    $body = ($lines[$bodyStart..($lines.Count - 1)] -join "`n").Trim()

    # 2. Package. The manifest it writes is what the notes quote.
    # A hashtable binds the named parameters; an array would be splatted positionally and '-Version' becomes the version.
    $pkgArgs = @{ Version = $Version }
    if ($SkipBuild) { $pkgArgs.SkipBuild = $true }
    & (Join-Path $PSScriptRoot 'package_release.ps1') @pkgArgs
    $pkgDir = Join-Path $repo 'target\pkg'
    $manifest = Get-Content (Join-Path $pkgDir 'manifest.json') -Raw | ConvertFrom-Json

    # 3. The notes: install steps, the hash just measured, then the changelog body. The Linux translator is built and
    # attached by the tag workflow on an Ubuntu runner; this local script packages the Windows zip only.
    $hashes = "``sha256`` of the zip: ``$($manifest.zipSha)``"
    $notes = @"
# EmblemWeave $Version

$summary

## Install

1. Download **$($manifest.zip)** below and unzip it into a folder of its own.
2. Run ``EmblemWeave.exe``.
3. On **Game Setup**, choose your dump. On **Overview**, press **Build the game** (once), then **Play**.

$hashes

$body
"@
    $notesPath = Join-Path $pkgDir "release-notes-$Version.md"
    [System.IO.File]::WriteAllText($notesPath, $notes + "`n")
    Write-Host "notes: $notesPath"

    # 4. The index row, newest first.
    $idx = [System.IO.File]::ReadAllText($index)
    $row = "| [$Version]($Version.md) | $(Get-Date -Format yyyy-MM-dd) | $summary |"
    $pattern = "(?m)^\| \[$([regex]::Escape($Version))\]\($([regex]::Escape($Version))\.md\) \|.*$"
    if ($idx -match $pattern) {
        $idx = [regex]::Replace($idx, $pattern, $row)
    } else {
        # After the "| --- |" separator of the table, so the newest release is the first row.
        $idx = [regex]::Replace($idx, "(?m)^(\| --- \| --- \| --- \|\r?\n)", "`$1$row`n", 1)
    }
    [System.IO.File]::WriteAllText($index, $idx)
    Write-Host "index: $index"

    # 5. Publish.
    if ($Local) {
        Write-Host "local only: not touching GitHub"
        return
    }

    # 5a. The tag has to point at a tree that already carries this release. Commit the changelog file and the index
    # row just written, push, and name that commit as the tag's target. 0.1.16 was tagged a commit behind its own
    # index row and had to be repointed by hand. Files left uncommitted are named after the commit, so a tag that
    # does not carry them is visible rather than a surprise.
    $relFiles = @($changelog, $index)
    $dirty = @(git status --porcelain -- $relFiles)
    if ($LASTEXITCODE -ne 0) { throw "git status failed ($LASTEXITCODE)" }
    if ($dirty.Count -gt 0) {
        git add -- $relFiles
        if ($LASTEXITCODE -ne 0) { throw "git add failed ($LASTEXITCODE)" }
        # A CRLF-only change is reported by git status and disappears after git add. Ask what is actually staged
        # before committing: an empty commit would make the script fail after the zip has already been built.
        $staged = @(git diff --cached --name-only -- $relFiles)
        if ($staged.Count -gt 0) {
            # Only these two paths: anything else already staged stays staged, for the next commit and out of this tag.
            git commit -q -m "Release $Version`n`nThe changelog and the index row tools/release.ps1 wrote for it." -- $relFiles
            if ($LASTEXITCODE -ne 0) { throw "git commit failed ($LASTEXITCODE)" }
            Write-Host 'committed the changelog and its index row'
        } else {
            Write-Host 'changelog and index row already in HEAD; nothing to commit'
        }
    }
    git push
    if ($LASTEXITCODE -ne 0) { throw "git push failed ($LASTEXITCODE): the tag would point at a tree without this release's notes" }
    $target = (git rev-parse HEAD).Trim()
    $left = @(git status --porcelain)
    if ($left.Count -gt 0) {
        Write-Warning "not in the tag, because they are not committed: $($left -join '; ')"
    }
    Write-Host "tag target: $target"

    $tag = "v$Version"
    $assets = @((Join-Path $pkgDir $manifest.zip))
    # Checked again here, immediately before upload: the same tool package_release.ps1 ran for the zip, so a stale or
    # hand-made asset cannot be published either, and the hashes in the notes are the files that go out.
    $py = Get-Command python -ErrorAction SilentlyContinue
    if (-not $py) { $py = Get-Command py -ErrorAction SilentlyContinue }
    if (-not $py) { throw 'python is needed to verify the release assets (tools/verify_archive.py)' }
    & $py.Source (@('tools/verify_archive.py') + $assets)
    if ($LASTEXITCODE -ne 0) { throw 'a release asset failed archive verification; nothing was uploaded' }
    $sha = (Get-FileHash $assets[0] -Algorithm SHA256).Hash.ToLower()
    if ($sha -ne $manifest.zipSha) { throw "$($manifest.zip) hashes to $sha, the manifest says $($manifest.zipSha)" }
    # Windows PowerShell 5.1 turns a native command's stderr into a terminating
    # error while $ErrorActionPreference is Stop. A missing release is the
    # normal first-publish case, so silence the probe and use its exit code.
    $oldPreference = $ErrorActionPreference
    $ErrorActionPreference = 'SilentlyContinue'
    gh release view $tag *> $null
    $releaseExists = $LASTEXITCODE -eq 0
    $ErrorActionPreference = $oldPreference
    if ($releaseExists) {
        # An existing release is edited rather than refused, so a corrected note can be republished.
        gh release edit $tag --title "EmblemWeave $Version" --notes-file $notesPath
        foreach ($a in $assets) {
            gh release delete-asset $tag ([System.IO.Path]::GetFileName($a)) -y *> $null
            gh release upload $tag $a
        }
    } else {
        gh release create $tag $assets --title "EmblemWeave $Version" --notes-file $notesPath --target $target
    }
    if ($LASTEXITCODE -ne 0) { throw "gh release failed ($LASTEXITCODE)" }
    gh release view $tag --json tagName,isDraft,isPrerelease,assets --jq '{tag: .tagName, draft: .isDraft, pre: .isPrerelease, assets: [.assets[] | {name, digest}]}'
} finally {
    Pop-Location
}
