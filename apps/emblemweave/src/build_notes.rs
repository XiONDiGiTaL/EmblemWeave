//! A short, read-only digest of the release notes between the build in the game folder and the launcher that is running.
//!
//! The notes are the shipped `docs/changelog/<version>.md` files. Only the summary and the first few points of each
//! release are kept: enough for the Overview's "Compare changes" popup to say what a rebuild would bring without
//! turning into a second copy of the release ledger. Nothing here decides whether a rebuild is needed; that is the
//! launcher version stamp in `fefw.ini`.

use std::path::{Path, PathBuf};

/// The most releases one comparison shows. An old install can be dozens of versions behind; the newest notes are the
/// ones that describe the code it would rebuild.
const MOST_RELEASES: usize = 8;

/// One release's summary and the first few substantive points.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildNote {
    pub version: String,
    pub summary: String,
    pub points: Vec<String>,
}

/// The notes a rebuild would compile in, plus how many older releases were left out of the popup.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BuildChanges {
    pub notes: Vec<BuildNote>,
    /// Releases between the build and the oldest note shown. Their engine changes are included in the rebuild too;
    /// the popup says so rather than pretending eight releases are the whole story.
    pub older: usize,
}

/// The shipped notes for the running launcher, between `from` (the build's version) and the current one.
pub fn current(from: Option<&str>) -> BuildChanges {
    let to = env!("CARGO_PKG_VERSION");
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            roots.push(dir.join("docs").join("changelog"));
        }
    }
    // Development runs start from `target\debug`, where the packaged docs are not beside the executable.
    roots.push(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("docs")
            .join("changelog"),
    );
    for dir in roots {
        if dir.is_dir() {
            return between(&dir, from, to);
        }
    }
    BuildChanges::default()
}

/// Read the notes in `changelog` for versions after `from` and at or before `to`.
pub fn between(changelog: &Path, from: Option<&str>, to: &str) -> BuildChanges {
    let Some(to_parts) = parse_version(to) else {
        return BuildChanges::default();
    };
    let from_parts = from.and_then(parse_version);
    let mut releases: Vec<(Vec<u32>, String, PathBuf)> = std::fs::read_dir(changelog)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            if !path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
            {
                return None;
            }
            let version = path.file_stem()?.to_string_lossy().into_owned();
            let parts = parse_version(&version)?;
            Some((parts, version, path))
        })
        .collect();
    releases.sort_by(|a, b| a.0.cmp(&b.0));

    let mut selected: Vec<(Vec<u32>, String, PathBuf)> = releases
        .into_iter()
        .filter(|(parts, _, _)| {
            if *parts > to_parts {
                return false;
            }
            match &from_parts {
                // A build from before the version stamp existed predates every file here. The current release's notes
                // are the only ones that can be shown honestly; the popup does not claim to reconstruct all history.
                None => parts == &to_parts,
                Some(from) => parts > from,
            }
        })
        .collect();
    if from_parts.is_none() && selected.is_empty() {
        // `to` has no changelog file: show the newest notes that do exist.
        if let Some(latest) = releases_last(&changelog, to_parts) {
            selected.push(latest);
        }
    }
    let older = selected.len().saturating_sub(MOST_RELEASES);
    if older > 0 {
        selected.drain(..older);
    }
    BuildChanges {
        notes: selected
            .into_iter()
            .filter_map(|(_, version, path)| {
                let text = std::fs::read_to_string(path).ok()?;
                Some(parse_note(&version, &text))
            })
            .collect(),
        older,
    }
}

/// The newest changelog whose version is at or below `to`; used only when there is no file for `to` itself.
fn releases_last(changelog: &Path, to: Vec<u32>) -> Option<(Vec<u32>, String, PathBuf)> {
    std::fs::read_dir(changelog)
        .ok()?
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            if !path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
            {
                return None;
            }
            let version = path.file_stem()?.to_string_lossy().into_owned();
            let parts = parse_version(&version)?;
            (parts <= to).then_some((parts, version, path))
        })
        .max_by(|a, b| a.0.cmp(&b.0))
}

fn parse_version(text: &str) -> Option<Vec<u32>> {
    let parts: Vec<u32> = text
        .trim()
        .trim_start_matches('v')
        .split('.')
        .map(|part| part.parse::<u32>().ok())
        .collect::<Option<Vec<_>>>()?;
    (parts.len() >= 2).then_some(parts)
}

/// A release note trimmed to its summary and a handful of points. Verification tables, deployment notes and known
/// issues are skipped: they describe how a release was checked, not what rebuilding changes.
fn parse_note(version: &str, text: &str) -> BuildNote {
    let mut summary = String::new();
    let mut points: Vec<String> = Vec::new();
    let mut include = true;
    for line in text.lines() {
        let line = line.trim();
        if let Some(heading) = line.strip_prefix("## ") {
            let heading = heading.to_ascii_lowercase();
            include = !(heading.contains("verification")
                || heading.contains("deployment")
                || heading.contains("known issue")
                || heading.contains("testing")
                || heading.contains("checks"));
            continue;
        }
        if !include {
            continue;
        }
        if summary.is_empty() && line.starts_with('>') {
            summary = shorten(&clean(line.trim_start_matches('>')), 300);
        } else if (line.starts_with("- ") || line.starts_with("* ")) && points.len() < 5 {
            let point = shorten(&clean(&line[2..]), 260);
            if !point.is_empty() && !points.contains(&point) {
                points.push(point);
            }
        }
    }
    BuildNote {
        version: version.to_string(),
        summary,
        points,
    }
}

fn clean(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '`' | '*' => {}
            '[' => {
                // `[label](url)` keeps the label and drops the address.
                for c in chars.by_ref() {
                    if c == ']' {
                        break;
                    }
                    out.push(c);
                }
                if chars.peek() == Some(&'(') {
                    for c in chars.by_ref() {
                        if c == ')' {
                            break;
                        }
                    }
                }
            }
            c if c.is_whitespace() => {
                if !out.ends_with(' ') {
                    out.push(' ');
                }
            }
            c => out.push(c),
        }
    }
    out.trim().to_string()
}

fn shorten(text: &str, limit: usize) -> String {
    let mut chars = text.chars();
    let head: String = chars.by_ref().take(limit).collect();
    if chars.next().is_none() {
        return head;
    }
    let cut = head.rfind(' ').unwrap_or(head.len());
    format!("{}…", head[..cut].trim_end())
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOTE_42: &str = r#"# EmblemWeave 0.1.42

> Cinematic playback and save fixes.

## Critical Fixes

- **Issue #77: Crash returning to title (implemented).** A long explanation follows.
- **Autosave guard.** A damaged autosave is moved to backups.

## Verification (0.1.42 release pass)

| Check | Result |
| --- | --- |
- this verification bullet is not a change

## Known Issues

- #41: crashes past Chapter 12.
"#;

    const NOTE_43: &str = r#"# EmblemWeave 0.1.43

> Build reliability.

## Build reliability

- **The final step reuses the launcher's toolchain.** See [the notes](https://example.invalid).
- **Regression coverage.** A script checks the environment.
"#;

    #[test]
    fn a_comparison_lists_only_the_releases_after_the_build() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("0.1.41.md"),
            "# EmblemWeave 0.1.41\n\n- old\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("0.1.42.md"), NOTE_42).unwrap();
        std::fs::write(dir.path().join("0.1.43.md"), NOTE_43).unwrap();

        let changes = between(dir.path(), Some("0.1.41"), "0.1.43");
        assert_eq!(changes.older, 0);
        assert_eq!(
            changes
                .notes
                .iter()
                .map(|n| n.version.as_str())
                .collect::<Vec<_>>(),
            ["0.1.42", "0.1.43"]
        );
        assert!(changes.notes[0].summary.contains("Cinematic"));
        assert!(changes.notes[0]
            .points
            .iter()
            .any(|p| p.contains("Crash returning")));
        assert!(
            !changes.notes[0]
                .points
                .iter()
                .any(|p| p.contains("verification")),
            "check tables are not changes a rebuild makes"
        );
        assert!(
            !changes.notes[0].points.iter().any(|p| p.contains("#41")),
            "known issues are not changes a rebuild makes"
        );
    }

    #[test]
    fn a_build_without_a_version_stamp_shows_only_the_current_release() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("0.1.42.md"), NOTE_42).unwrap();
        std::fs::write(dir.path().join("0.1.43.md"), NOTE_43).unwrap();
        let changes = between(dir.path(), None, "0.1.43");
        assert_eq!(changes.notes.len(), 1);
        assert_eq!(changes.notes[0].version, "0.1.43");
    }

    #[test]
    fn a_long_gap_keeps_the_newest_notes_and_counts_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        for patch in 30..=43 {
            let name = format!("0.1.{patch}.md");
            std::fs::write(
                dir.path().join(name),
                format!("# EmblemWeave 0.1.{patch}\n\n> notes\n"),
            )
            .unwrap();
        }
        let changes = between(dir.path(), Some("0.1.30"), "0.1.43");
        assert_eq!(changes.notes.len(), MOST_RELEASES);
        assert_eq!(changes.older, 5);
        assert_eq!(changes.notes.last().unwrap().version, "0.1.43");
    }

    #[test]
    fn the_running_launcher_finds_its_own_shipped_notes() {
        let changes = current(Some("0.1.42"));
        assert!(
            changes
                .notes
                .iter()
                .any(|n| n.version == env!("CARGO_PKG_VERSION")),
            "the changelog beside the source should name the running version"
        );
    }

    #[test]
    fn markdown_links_keep_their_label_and_lose_the_address() {
        assert_eq!(
            clean("See [the notes](https://example.invalid)."),
            "See the notes."
        );
        assert_eq!(clean("**Bold** and `code`"), "Bold and code");
        assert_eq!(shorten("a short line", 40), "a short line");
        assert!(shorten("one two three four", 10).ends_with('…'));
    }
}
