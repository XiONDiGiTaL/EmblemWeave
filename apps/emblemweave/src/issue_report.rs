//! The Report An Issue model: categories, conditional fields, diagnostics,
//! attachments, validation, GitHub Markdown and the browser handoff.
//!
//! GitHub's public API has no attachment upload: the web editor's paperclip
//! talks to a private endpoint that needs a signed-in browser session, and
//! creating an issue with the REST API needs a token we must not ship. This
//! module therefore prepares a complete report and opens the prefilled
//! new-issue page; the files are copied to a handoff folder so the user can
//! drag them in. Nothing here submits an issue, and no state is called
//! "submitted" until the user says so.

use crate::config::Config;
use crate::{fmt_size, App};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg_attr(not(test), allow(dead_code))]
pub const REPO_URL: &str = "https://github.com/XiONDiGiTaL/EmblemWeave";
pub const NEW_ISSUE_URL: &str = "https://github.com/XiONDiGiTaL/EmblemWeave/issues/new";

/// GitHub's issue body is carried in the URL. Keep well below the limits of
/// browsers and proxies; the full report always exists in the handoff folder.
const MAX_URL_BODY: usize = 6_000;
const MAX_DIAGNOSTIC_CHARS: usize = 3_000;

pub const MAX_ATTACHMENTS: usize = 10;
pub const MAX_FILE_BYTES: u64 = 25 * 1024 * 1024;
pub const MAX_TOTAL_BYTES: u64 = 50 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    Bug,
    BuildFailed,
    Installation,
    Launcher,
    Crash,
    Performance,
    Compatibility,
    FeatureRequest,
    Enhancement,
    Inquiry,
    Documentation,
    Other,
}

impl Default for Category {
    fn default() -> Self {
        Category::Bug
    }
}

impl Category {
    pub const ALL: [Category; 12] = [
        Category::Bug,
        Category::BuildFailed,
        Category::Installation,
        Category::Launcher,
        Category::Crash,
        Category::Performance,
        Category::Compatibility,
        Category::FeatureRequest,
        Category::Enhancement,
        Category::Inquiry,
        Category::Documentation,
        Category::Other,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Category::Bug => "Bug",
            Category::BuildFailed => "Build Failed",
            Category::Installation => "Installation Problem",
            Category::Launcher => "Launcher Problem",
            Category::Crash => "Runtime / Game Crash",
            Category::Performance => "Performance Issue",
            Category::Compatibility => "Compatibility Issue",
            Category::FeatureRequest => "Feature Request",
            Category::Enhancement => "Enhancement",
            Category::Inquiry => "Inquiry / Question",
            Category::Documentation => "Documentation",
            Category::Other => "Other",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Category::Bug => "Something in the launcher or game does not work as expected.",
            Category::BuildFailed => "Building the game stops with an error or produces no executable.",
            Category::Installation => "Setting up EmblemWeave, its tools, or the game data failed.",
            Category::Launcher => "The launcher will not open, a page is broken, or a setting will not stick.",
            Category::Crash => "The game or runtime closes, freezes, or stops responding.",
            Category::Performance => "Frame rate, stutter, loading, or memory use is worse than it should be.",
            Category::Compatibility => "A GPU, driver, game version, or Windows setup does not work.",
            Category::FeatureRequest => "Ask for functionality that does not exist yet.",
            Category::Enhancement => "Improve something that already exists.",
            Category::Inquiry => "Ask how to do something, or for information you could not find.",
            Category::Documentation => "A guide is missing, wrong, unclear, or out of date.",
            Category::Other => "Anything that does not fit the categories above.",
        }
    }

    /// Only labels that exist in XiONDiGiTaL/EmblemWeave today. Checked through
    /// the GitHub API; the launcher must not invent labels.
    pub fn github_label(self) -> Option<&'static str> {
        match self {
            Category::Bug
            | Category::BuildFailed
            | Category::Installation
            | Category::Launcher
            | Category::Crash
            | Category::Performance
            | Category::Compatibility => Some("bug"),
            Category::FeatureRequest | Category::Enhancement => Some("enhancement"),
            Category::Inquiry => Some("question"),
            Category::Documentation => Some("documentation"),
            Category::Other => None,
        }
    }

    pub fn description_label(self) -> &'static str {
        match self {
            Category::BuildFailed => "What happened when the build failed?",
            Category::FeatureRequest => "What functionality are you requesting?",
            Category::Enhancement => "What existing feature needs improvement?",
            Category::Inquiry => "What are you trying to accomplish?",
            Category::Documentation => "What is unclear, missing, or wrong?",
            _ => "What happened?",
        }
    }

    pub fn description_hint(self) -> &'static str {
        match self {
            Category::BuildFailed => "Name the build step, what you expected, and what stopped it. The first error is the most useful line.",
            Category::FeatureRequest => "Describe the feature and the situation where you would use it.",
            Category::Enhancement => "Name the current behavior and what you would rather it did.",
            Category::Inquiry => "Explain the goal, not only the step that failed, so an answer can be specific.",
            Category::Documentation => "Link or name the page, and quote the part that misled you.",
            _ => "What were you doing, what did you expect, and what happened? Include the menu, scene, or step.",
        }
    }

    pub fn asks_for_frequency(self) -> bool {
        matches!(
            self,
            Category::Bug
                | Category::BuildFailed
                | Category::Launcher
                | Category::Crash
                | Category::Performance
                | Category::Compatibility
                | Category::Other
        )
    }

    pub fn asks_when_started(self) -> bool {
        matches!(
            self,
            Category::Bug | Category::Crash | Category::Performance | Category::Compatibility | Category::Other
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Frequency {
    #[default]
    NotSure,
    Always,
    Sometimes,
    Once,
}

impl Frequency {
    pub const ALL: [Frequency; 4] = [Frequency::NotSure, Frequency::Always, Frequency::Sometimes, Frequency::Once];

    pub fn label(self) -> &'static str {
        match self {
            Frequency::NotSure => "Not sure",
            Frequency::Always => "Always",
            Frequency::Sometimes => "Sometimes",
            Frequency::Once => "Once",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Field {
    Steps,
    Expected,
    Actual,
    Error,
    BuildOperation,
    Toolchain,
    BuildConfig,
    FirstError,
    BuildPreviously,
    Tried,
    GameVersion,
    Hardware,
    RequestedFeature,
    ProblemSolved,
    ExpectedWork,
    ExistingFeature,
    PreferredBehavior,
    WhyUseful,
    Goal,
    InfoNeeded,
    DocLocation,
}

impl Field {
    pub fn key(self) -> &'static str {
        match self {
            Field::Steps => "steps",
            Field::Expected => "expected",
            Field::Actual => "actual",
            Field::Error => "error",
            Field::BuildOperation => "build_operation",
            Field::Toolchain => "toolchain",
            Field::BuildConfig => "build_config",
            Field::FirstError => "first_error",
            Field::BuildPreviously => "build_previously",
            Field::Tried => "tried",
            Field::GameVersion => "game_version",
            Field::Hardware => "hardware",
            Field::RequestedFeature => "requested_feature",
            Field::ProblemSolved => "problem_solved",
            Field::ExpectedWork => "expected_work",
            Field::ExistingFeature => "existing_feature",
            Field::PreferredBehavior => "preferred_behavior",
            Field::WhyUseful => "why_useful",
            Field::Goal => "goal",
            Field::InfoNeeded => "info_needed",
            Field::DocLocation => "doc_location",
        }
    }
}

pub struct FieldSpec {
    pub field: Field,
    pub label: &'static str,
    pub hint: &'static str,
    pub rows: u8,
}

const fn spec(field: Field, label: &'static str, hint: &'static str, rows: u8) -> FieldSpec {
    FieldSpec { field, label, hint, rows }
}

pub fn fields(category: Category) -> Vec<FieldSpec> {
    match category {
        Category::Bug => vec![
            spec(Field::Steps, "Steps to reproduce", "One step per line. Start from opening the launcher.", 4),
            spec(Field::Expected, "Expected behavior", "What should have happened?", 2),
            spec(Field::Actual, "Actual behavior", "What happened instead? Include the exact error if there is one.", 3),
            spec(Field::Error, "Error message or code", "Paste it exactly. Screenshots can go in Attachments below.", 2),
            spec(Field::Tried, "What have you already tried?", "Rebuild, safe graphics, another driver, a new save: what changed the result?", 3),
        ],
        Category::BuildFailed => vec![
            spec(Field::BuildOperation, "Which build operation failed?", "Building the game, repairing it, preparing shaders, or another step.", 2),
            spec(Field::Toolchain, "Compiler / toolchain", "From Requirements or Diagnostics, if known.", 2),
            spec(Field::BuildConfig, "Build configuration", "Release/Debug, CPU variant, and any settings you changed.", 2),
            spec(Field::FirstError, "First meaningful error", "The first error line, not the cascade after it.", 4),
            spec(Field::BuildPreviously, "Did the build previously succeed?", "Yes, no, or not sure. If yes, what changed since then?", 2),
            spec(Field::Tried, "What have you already tried?", "Reinstalling tools, a clean rebuild, a different drive.", 3),
        ],
        Category::Installation => vec![
            spec(Field::Error, "Error message or code", "Paste the exact message from the installer or launcher.", 3),
            spec(Field::Tried, "What have you already tried?", "Reinstalling, a different folder, running as administrator.", 3),
        ],
        Category::Launcher => vec![
            spec(Field::Steps, "Steps to reproduce", "Which page or button, and what you did just before it happened.", 4),
            spec(Field::Expected, "Expected behavior", "What should the launcher have done?", 2),
            spec(Field::Actual, "Actual behavior", "What happened instead? Include any message.", 3),
            spec(Field::Error, "Error message or code", "Paste it exactly, if the launcher showed one.", 2),
            spec(Field::Tried, "What have you already tried?", "Restarting the launcher, resetting a setting, reopening the page.", 3),
        ],
        Category::Crash => vec![
            spec(Field::Steps, "Steps to reproduce", "What were you doing before the crash or freeze?", 4),
            spec(Field::Error, "Error message or code", "Paste the last lines or the code shown by the game.", 3),
            spec(Field::Tried, "What have you already tried?", "Safe graphics, a rebuild, another save or shader cache.", 3),
        ],
        Category::Performance => vec![
            spec(Field::Steps, "Where does it happen?", "Scene, menu, resolution, and what you were doing.", 4),
            spec(Field::Expected, "Expected performance", "The frame rate or loading time you expected, if you know.", 2),
            spec(Field::Actual, "Actual performance", "The frame rate, stutter pattern, or loading time you saw.", 3),
            spec(Field::Tried, "What have you already tried?", "Graphics settings, upscaler, frame limit, driver update.", 3),
        ],
        Category::Compatibility => vec![
            spec(Field::Steps, "What are you trying to run?", "Game version, GPU, driver, Windows version, or another setup.", 4),
            spec(Field::Actual, "What happens instead?", "The symptom, error, or unsupported-device message.", 3),
            spec(Field::Error, "Error message or code", "Paste the exact message if one appears.", 2),
            spec(Field::Tried, "What have you already tried?", "Driver versions, another GPU, a different game version.", 3),
        ],
        Category::FeatureRequest => vec![
            spec(Field::ProblemSolved, "What problem would it solve?", "Describe the need, not only the proposed control.", 3),
            spec(Field::ExpectedWork, "How would you expect it to work?", "A rough flow is enough.", 3),
        ],
        Category::Enhancement => vec![
            spec(Field::PreferredBehavior, "What behavior would you prefer?", "Describe the change you want.", 3),
            spec(Field::WhyUseful, "Why would the change be useful?", "Who benefits, and in which situation?", 3),
        ],
        Category::Inquiry => vec![
            spec(Field::Tried, "What have you already tried?", "Pages you read and steps you attempted.", 3),
            spec(Field::InfoNeeded, "What information do you need?", "Ask the specific question you want answered.", 3),
        ],
        Category::Documentation => vec![
            spec(Field::DocLocation, "Where is the documentation?", "Page title, link, or the file you were reading.", 2),
            spec(Field::Expected, "What did you expect it to say?", "The missing or corrected information.", 3),
            spec(Field::InfoNeeded, "What would have helped?", "The wording or example that would have unblocked you.", 3),
        ],
        Category::Other => vec![
            spec(Field::Tried, "What have you already tried?", "Anything a developer should know before replying.", 3),
        ],
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Details {
    pub steps: String,
    pub expected: String,
    pub actual: String,
    pub error: String,
    pub build_operation: String,
    pub toolchain: String,
    pub build_config: String,
    pub first_error: String,
    pub build_previously: String,
    pub tried: String,
    pub game_version: String,
    pub hardware: String,
    pub requested_feature: String,
    pub problem_solved: String,
    pub expected_work: String,
    pub existing_feature: String,
    pub preferred_behavior: String,
    pub why_useful: String,
    pub goal: String,
    pub info_needed: String,
    pub doc_location: String,
}

impl Details {
    pub fn get(&self, field: Field) -> &str {
        match field {
            Field::Steps => &self.steps,
            Field::Expected => &self.expected,
            Field::Actual => &self.actual,
            Field::Error => &self.error,
            Field::BuildOperation => &self.build_operation,
            Field::Toolchain => &self.toolchain,
            Field::BuildConfig => &self.build_config,
            Field::FirstError => &self.first_error,
            Field::BuildPreviously => &self.build_previously,
            Field::Tried => &self.tried,
            Field::GameVersion => &self.game_version,
            Field::Hardware => &self.hardware,
            Field::RequestedFeature => &self.requested_feature,
            Field::ProblemSolved => &self.problem_solved,
            Field::ExpectedWork => &self.expected_work,
            Field::ExistingFeature => &self.existing_feature,
            Field::PreferredBehavior => &self.preferred_behavior,
            Field::WhyUseful => &self.why_useful,
            Field::Goal => &self.goal,
            Field::InfoNeeded => &self.info_needed,
            Field::DocLocation => &self.doc_location,
        }
    }

    pub fn get_mut(&mut self, field: Field) -> &mut String {
        match field {
            Field::Steps => &mut self.steps,
            Field::Expected => &mut self.expected,
            Field::Actual => &mut self.actual,
            Field::Error => &mut self.error,
            Field::BuildOperation => &mut self.build_operation,
            Field::Toolchain => &mut self.toolchain,
            Field::BuildConfig => &mut self.build_config,
            Field::FirstError => &mut self.first_error,
            Field::BuildPreviously => &mut self.build_previously,
            Field::Tried => &mut self.tried,
            Field::GameVersion => &mut self.game_version,
            Field::Hardware => &mut self.hardware,
            Field::RequestedFeature => &mut self.requested_feature,
            Field::ProblemSolved => &mut self.problem_solved,
            Field::ExpectedWork => &mut self.expected_work,
            Field::ExistingFeature => &mut self.existing_feature,
            Field::PreferredBehavior => &mut self.preferred_behavior,
            Field::WhyUseful => &mut self.why_useful,
            Field::Goal => &mut self.goal,
            Field::InfoNeeded => &mut self.info_needed,
            Field::DocLocation => &mut self.doc_location,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttachmentKind {
    Log,
    Archive,
    Image,
    Text,
    Config,
    Diagnostic,
    Other,
}

impl AttachmentKind {
    pub fn label(self) -> &'static str {
        match self {
            AttachmentKind::Log => "log",
            AttachmentKind::Archive => "archive",
            AttachmentKind::Image => "image",
            AttachmentKind::Text => "text",
            AttachmentKind::Config => "config",
            AttachmentKind::Diagnostic => "diagnostic",
            AttachmentKind::Other => "file",
        }
    }

    fn for_path(path: &Path) -> AttachmentKind {
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
        match ext.as_str() {
            "log" => AttachmentKind::Log,
            "txt" => AttachmentKind::Text,
            "zip" | "7z" | "rar" | "gz" | "tar" => AttachmentKind::Archive,
            "png" | "jpg" | "jpeg" | "webp" | "gif" => AttachmentKind::Image,
            "ini" | "cfg" | "conf" | "json" | "toml" | "yaml" | "yml" => AttachmentKind::Config,
            "md" | "csv" => AttachmentKind::Text,
            "dmp" | "etl" | "wer" | "diag" => AttachmentKind::Diagnostic,
            _ => AttachmentKind::Other,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attachment {
    pub path: PathBuf,
    pub name: String,
    pub size: u64,
    pub kind: AttachmentKind,
}

impl Attachment {
    pub fn is_warning_name(&self) -> bool {
        let n = self.name.to_ascii_lowercase();
        ["token", "password", "passwd", "secret", "credential", ".env", ".key", ".pem", ".pfx"]
            .iter()
            .any(|m| n.contains(m))
    }
}

pub enum AddResult {
    Added(Attachment),
    Duplicate(String),
    Rejected(String),
}

/// Adds one file after checking its type, size, and whether the same file is
/// already listed. Nothing is read beyond its metadata and never executed.
pub fn add_attachment(draft: &mut Draft, path: &Path) -> AddResult {
    let bad = |why: String| AddResult::Rejected(why);
    let meta = match std::fs::metadata(path) {
        Ok(m) if m.is_file() => m,
        Ok(_) => return bad(format!("{} is not a file.", path.display())),
        Err(e) => return bad(format!("Could not read {}: {e}", path.display())),
    };
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    const BLOCKED: &[&str] = &[
        "exe", "dll", "sys", "msi", "bat", "cmd", "ps1", "com", "scr", "lnk", "pif", "reg", "kdbx", "pfx", "p12",
        "env", "key", "pem", "crt", "cer", "ovpn",
    ];
    let leaf = path.file_name().and_then(|n| n.to_str()).unwrap_or("").to_ascii_lowercase();
    const BLOCKED_NAMES: &[&str] = &[
        ".env", "id_rsa", "id_ed25519", "id_ecdsa", "id_dsa", ".git-credentials", ".npmrc", ".netrc", "credentials",
    ];
    if BLOCKED.contains(&ext.as_str()) || BLOCKED_NAMES.contains(&leaf.as_str()) || leaf.starts_with(".env.") {
        return bad(format!(
            "{} is a program or credential file. Attach a log, screenshot, archive, or text file instead.",
            path.display()
        ));
    }
    if meta.len() == 0 {
        return bad(format!("{} is empty (0 bytes).", path.display()));
    }
    if meta.len() > MAX_FILE_BYTES {
        return bad(format!(
            "{} is {}; the limit is {} per file.",
            path.display(),
            fmt_size(meta.len()),
            fmt_size(MAX_FILE_BYTES)
        ));
    }
    if draft.attachments.len() >= MAX_ATTACHMENTS {
        return bad(format!("A report can carry at most {MAX_ATTACHMENTS} files. Remove one first."));
    }
    let total: u64 = draft.attachments.iter().map(|a| a.size).sum();
    if total + meta.len() > MAX_TOTAL_BYTES {
        return bad(format!(
            "The attachments already total {}; adding this file would pass the {} limit.",
            fmt_size(total),
            fmt_size(MAX_TOTAL_BYTES)
        ));
    }
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string());
    let same_path = |a: &Attachment| match (a.path.canonicalize(), path.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a.path == path,
    };
    if let Some(existing) = draft.attachments.iter().find(|a| same_path(a) || (a.name == name && a.size == meta.len())) {
        return AddResult::Duplicate(format!("{} is already attached.", existing.name));
    }
    let attachment = Attachment { path: path.to_path_buf(), name, size: meta.len(), kind: AttachmentKind::for_path(path) };
    draft.attachments.push(attachment.clone());
    AddResult::Added(attachment)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Diagnostics {
    pub application: bool,
    pub system: bool,
    pub build: bool,
    pub runtime: bool,
    pub logs: bool,
}

impl Default for Diagnostics {
    fn default() -> Self {
        Diagnostics { application: true, system: true, build: true, runtime: true, logs: true }
    }
}

impl Diagnostics {
    pub fn any(&self) -> bool {
        self.application || self.system || self.build || self.runtime || self.logs
    }
}

pub fn remove_attachment(draft: &mut Draft, index: usize) -> Option<Attachment> {
    if index < draft.attachments.len() {
        Some(draft.attachments.remove(index))
    } else {
        None
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Draft {
    pub title: String,
    pub category: Option<Category>,
    pub description: String,
    pub details: Details,
    pub frequency: Frequency,
    pub started: String,
    pub diagnostics: Diagnostics,
    pub attachments: Vec<Attachment>,
    pub handoff_dir: Option<PathBuf>,
    pub confirmed_submitted: bool,
}

impl Default for Draft {
    fn default() -> Self {
        Draft {
            title: String::new(),
            category: None,
            description: String::new(),
            details: Details::default(),
            frequency: Frequency::NotSure,
            started: String::new(),
            diagnostics: Diagnostics::default(),
            attachments: Vec::new(),
            handoff_dir: None,
            confirmed_submitted: false,
        }
    }
}

impl Draft {
    pub fn category_label(&self) -> &'static str {
        self.category.map(Category::label).unwrap_or("Choose a category")
    }

    pub fn summary(&self) -> String {
        let files = self.attachments.len();
        let attachments = if files == 1 { "1 attachment".into() } else { format!("{files} attachments") };
        if self.title.trim().is_empty() && self.description.trim().is_empty() && self.category.is_none() {
            "Not started".into()
        } else {
            format!("{} · {}", self.category_label(), attachments)
        }
    }

    /// The note stored in the existing diagnostic ZIP, so the bundle and the
    /// issue describe the same problem.
    pub fn bundle_note(&self) -> String {
        let mut note = String::new();
        if let Some(c) = self.category {
            note.push_str(&format!("[{}] ", c.label()));
        }
        if !self.title.trim().is_empty() {
            note.push_str(self.title.trim());
            note.push_str("\n\n");
        }
        note.push_str(self.description.trim());
        if !self.details.steps.trim().is_empty() {
            note.push_str("\n\nSteps:\n");
            note.push_str(self.details.steps.trim());
        }
        note
    }
}

#[derive(Debug, Clone, Default)]
pub struct Validation {
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

impl Validation {
    pub fn ok(&self) -> bool {
        self.errors.is_empty()
    }
}

pub fn validate(draft: &Draft) -> Validation {
    let mut v = Validation::default();
    let title = draft.title.trim();
    if title.is_empty() {
        v.errors.push("Add a title that says what went wrong.".into());
    } else if title.chars().count() < 4 {
        v.errors.push("The title is too short to be useful.".into());
    } else if title.chars().count() > 160 {
        v.errors.push("The title is longer than 160 characters; shorten it to the main symptom.".into());
    } else if title.chars().count() < 12 {
        v.warnings.push("A more specific title makes the issue easier to find and triage.".into());
    }
    if draft.category.is_none() {
        v.errors.push("Choose an issue category.".into());
    }
    let description = draft.description.trim();
    if description.is_empty() {
        v.errors.push("Description: say what happened.".into());
    } else if description.chars().count() < 30 {
        v.warnings.push("The description is very short; include what you expected and what happened.".into());
    }
    if draft.category == Some(Category::Bug) && draft.details.steps.trim().is_empty() {
        v.warnings.push("Bug reports without steps to reproduce are much harder to act on.".into());
    }
    if draft.category == Some(Category::BuildFailed) {
        let has_log = draft.attachments.iter().any(|a| {
            matches!(a.kind, AttachmentKind::Log | AttachmentKind::Archive | AttachmentKind::Diagnostic)
        });
        if draft.details.first_error.trim().is_empty() && !has_log {
            v.warnings.push("Add the first build error or a build log so the failure can be diagnosed.".into());
        }
    }
    for a in &draft.attachments {
        if !a.path.is_file() {
            v.errors.push(format!("Attachment {} no longer exists; remove it or choose it again.", a.name));
        }
    }
    if !draft.diagnostics.any() {
        v.warnings.push("No diagnostic information is included; developers may have to ask for it.".into());
    }
    if draft.attachments.is_empty() {
        v.warnings.push("No files are attached. Logs or a screenshot often answer the first question.".into());
    }
    if draft.diagnostics.logs {
        v.warnings.push("The recent log can contain local paths or account names; review it before submitting.".into());
    }
    for hit in sensitive_hits(&user_text(draft)) {
        v.warnings.push(format!(
            "The report mentions \"{hit}\". Remove passwords, tokens, keys, and personal paths before submitting."
        ));
    }
    v
}

fn user_text(draft: &Draft) -> String {
    let mut text = format!("{}\n{}\n{}\n", draft.title, draft.description, draft.started);
    for f in [
        Field::Steps,
        Field::Expected,
        Field::Actual,
        Field::Error,
        Field::BuildOperation,
        Field::Toolchain,
        Field::BuildConfig,
        Field::FirstError,
        Field::BuildPreviously,
        Field::Tried,
        Field::GameVersion,
        Field::Hardware,
        Field::RequestedFeature,
        Field::ProblemSolved,
        Field::ExpectedWork,
        Field::ExistingFeature,
        Field::PreferredBehavior,
        Field::WhyUseful,
        Field::Goal,
        Field::InfoNeeded,
        Field::DocLocation,
    ] {
        text.push_str(draft.details.get(f));
        text.push('\n');
    }
    text
}

/// Cheap detection, deliberately biased to a warning rather than a block.
fn sensitive_hits(text: &str) -> Vec<&'static str> {
    let lower = text.to_ascii_lowercase();
    let mut hits = Vec::new();
    for needle in ["password", "passwd", "api key", "apikey", "access token", "bearer ", "authorization:", "-----begin"] {
        if lower.contains(needle) {
            hits.push(needle);
        }
    }
    if lower.contains("c:\\users\\") {
        hits.push("C:\\Users\\");
    }
    hits
}

#[derive(Debug, Clone, Default)]
pub struct Facts {
    pub application: String,
    pub system: String,
    pub game: String,
    pub build: String,
    pub runtime: String,
    pub logs: String,
}

impl Facts {
    /// Collects only what the user asked to include. The GPU list is not
    /// enumerated here: it is a synchronous Vulkan scan and the launcher must
    /// not do that while drawing a report. Hardware is a user field.
    pub fn collect(app: &App, sel: &Diagnostics) -> Facts {
        let mut f = Facts::default();
        if sel.application {
            f.application = format!(
                "{} ({}) · {} {}",
                env!("CARGO_PKG_VERSION"),
                env!("EW_GIT_REV"),
                crate::platform::name(),
                std::env::consts::ARCH
            );
        }
        if sel.system {
            let cpu = crate::cpu::probe().brand;
            f.system = if cpu.trim().is_empty() {
                format!("{} {} (processor name unavailable)", crate::platform::name(), std::env::consts::ARCH)
            } else {
                format!("{} {} · CPU: {cpu}", crate::platform::name(), std::env::consts::ARCH)
            };
        }
        if sel.build {
            let mut b = String::new();
            if let Some((name, version)) = &app.game.title {
                b.push_str(&format!("Game: {name} ({version})\n"));
            }
            if let Some(id) = app.game.title_id {
                b.push_str(&format!("Title ID: {id:016X}\n"));
            }
            b.push_str(&format!("Game data: {:?}\n", app.game.worst()));
            b.push_str(&format!("Built game ready: {}\n", if crate::engine::ready() { "yes" } else { "no" }));
            if let Some((built, now)) = crate::engine::stale_build() {
                b.push_str(&format!(
                    "Build folder made by {}, launcher is {}\n",
                    built.unwrap_or_else(|| "an older release".into()),
                    now
                ));
            }
            if let Some((report, _)) = &app.last_boot {
                b.push_str(&format!("Last runtime: {}\n", report.headline()));
            }
            f.build = b.trim_end().to_string();
        }
        if sel.runtime {
            let mut r = String::new();
            let g = &app.config.graphics;
            r.push_str(&format!(
                "Graphics: {} {} · vsync {} · frame limit {} · render scale {:.2} · upscaling {} · GPU {} · present guard {}
",
                g.resolution,
                g.display_mode,
                if g.vsync { "on" } else { "off" },
                if g.frame_limit == 0 { "unlimited".to_string() } else { g.frame_limit.to_string() },
                g.render_scale,
                g.upscaling,
                g.gpu,
                g.present_guard
            ));
            r.push_str(&format!(
                "Runtime limits: {} guest steps, {} MiB heap
",
                app.config.runtime.max_steps, app.config.runtime.heap_mib
            ));
            if let Some((report, ms)) = &app.last_boot {
                r.push_str(&format!("Boot test ({ms} ms): {}\n", report.headline()));
                for line in report.thread_states.iter().take(12) {
                    r.push_str(line);
                    r.push('\n');
                }
            } else {
                r.push_str("No boot test was run in this session.\n");
            }
            if let Some(report) = app.last_pipeline.as_ref().and_then(|p| p.report.as_ref()) {
                r.push_str(&format!(
                    "Recompiler: {} functions ({} complete), {:.2}% decode coverage, {} blocks\n",
                    report.lifted_functions, report.complete_functions, report.decode_coverage * 100.0, report.blocks_lifted
                ));
            }
            f.runtime = r.trim_end().to_string();
        }
        if sel.logs {
            f.logs = limit(&crate::crash::environment_text(), MAX_DIAGNOSTIC_CHARS);
        }
        f
    }
}

fn limit(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max).collect();
    out.push_str("\n… [truncated; the full log is in the log folder]");
    out
}

fn section(out: &mut String, title: &str, body: &str) {
    let body = body.trim();
    if body.is_empty() {
        return;
    }
    out.push_str(&format!("\n## {title}\n\n{body}\n"));
}

fn steps_markdown(text: &str) -> String {
    let mut out = String::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if line.starts_with('-') || line.starts_with('*') || line.chars().next().is_some_and(|c| c.is_ascii_digit()) {
            out.push_str(line);
        } else {
            out.push_str("- ");
            out.push_str(line);
        }
        out.push('\n');
    }
    out
}

pub fn markdown(draft: &Draft, facts: &Facts) -> String {
    let category = draft.category;
    let mut out = String::new();
    out.push_str(&format!("**Category:** {}\n", draft.category_label()));
    if draft.frequency != Frequency::NotSure && category.is_some_and(Category::asks_for_frequency) {
        out.push_str(&format!("**Frequency:** {}\n", draft.frequency.label()));
    }
    if !draft.started.trim().is_empty() && category.is_some_and(Category::asks_when_started) {
        out.push_str(&format!("**Started:** {}\n", draft.started.trim()));
    }
    section(&mut out, "Summary", &draft.description);

    if let Some(c) = category {
        match c {
            Category::Bug => {
                section(&mut out, "Steps to Reproduce", &steps_markdown(&draft.details.steps));
                section(&mut out, "Expected Behavior", &draft.details.expected);
                section(&mut out, "Actual Behavior", &draft.details.actual);
                if !draft.details.error.trim().is_empty() {
                    section(&mut out, "Error Message / Code", &format!("```text\n{}\n```", draft.details.error.trim()));
                }
                section(&mut out, "Already Tried", &steps_markdown(&draft.details.tried));
            }
            Category::BuildFailed => {
                section(&mut out, "Build Operation", &draft.details.build_operation);
                section(&mut out, "Toolchain", &draft.details.toolchain);
                section(&mut out, "Build Configuration", &draft.details.build_config);
                if !draft.details.first_error.trim().is_empty() {
                    section(&mut out, "First Meaningful Error", &format!("```text\n{}\n```", draft.details.first_error.trim()));
                }
                section(&mut out, "Previously Successful", &draft.details.build_previously);
                section(&mut out, "Already Tried", &steps_markdown(&draft.details.tried));
            }
            Category::Installation => {
                if !draft.details.error.trim().is_empty() {
                    section(&mut out, "Error Message / Code", &format!("```text\n{}\n```", draft.details.error.trim()));
                }
                section(&mut out, "Already Tried", &steps_markdown(&draft.details.tried));
            }
            Category::Launcher => {
                section(&mut out, "Steps to Reproduce", &steps_markdown(&draft.details.steps));
                section(&mut out, "Expected Behavior", &draft.details.expected);
                section(&mut out, "Actual Behavior", &draft.details.actual);
                if !draft.details.error.trim().is_empty() {
                    section(&mut out, "Error Message / Code", &format!("```text\n{}\n```", draft.details.error.trim()));
                }
                section(&mut out, "Already Tried", &steps_markdown(&draft.details.tried));
            }
            Category::Crash => {
                section(&mut out, "Steps to Reproduce", &steps_markdown(&draft.details.steps));
                if !draft.details.error.trim().is_empty() {
                    section(&mut out, "Error Message / Code", &format!("```text\n{}\n```", draft.details.error.trim()));
                }
                section(&mut out, "Already Tried", &steps_markdown(&draft.details.tried));
            }
            Category::Performance => {
                section(&mut out, "Where It Happens", &steps_markdown(&draft.details.steps));
                section(&mut out, "Expected Performance", &draft.details.expected);
                section(&mut out, "Actual Performance", &draft.details.actual);
                section(&mut out, "Already Tried", &steps_markdown(&draft.details.tried));
            }
            Category::Compatibility => {
                section(&mut out, "What I Am Trying To Run", &draft.details.steps);
                section(&mut out, "What Happens Instead", &draft.details.actual);
                if !draft.details.error.trim().is_empty() {
                    section(&mut out, "Error Message / Code", &format!("```text\n{}\n```", draft.details.error.trim()));
                }
                section(&mut out, "Already Tried", &steps_markdown(&draft.details.tried));
            }
            Category::FeatureRequest => {
                section(&mut out, "Problem This Solves", &draft.details.problem_solved);
                section(&mut out, "How It Could Work", &draft.details.expected_work);
            }
            Category::Enhancement => {
                section(&mut out, "Preferred Behavior", &draft.details.preferred_behavior);
                section(&mut out, "Why It Is Useful", &draft.details.why_useful);
            }
            Category::Inquiry => {
                section(&mut out, "What I Tried", &steps_markdown(&draft.details.tried));
                section(&mut out, "Information Needed", &draft.details.info_needed);
            }
            Category::Documentation => {
                section(&mut out, "Location", &draft.details.doc_location);
                section(&mut out, "What I Expected", &draft.details.expected);
                section(&mut out, "What Would Have Helped", &draft.details.info_needed);
            }
            Category::Other => {
                section(&mut out, "Already Tried", &steps_markdown(&draft.details.tried));
            }
        }
    }

    let mut environment = String::new();
    if !facts.application.is_empty() {
        environment.push_str(&format!("- EmblemWeave: {}\n", facts.application));
    }
    if !facts.system.is_empty() {
        environment.push_str(&format!("- System: {}\n", facts.system));
    }
    if !draft.details.game_version.trim().is_empty() {
        environment.push_str(&format!("- Game version: {}\n", draft.details.game_version.trim()));
    }
    if !facts.game.is_empty() {
        environment.push_str(&format!("- Game: {}\n", facts.game));
    }
    if !draft.details.hardware.trim().is_empty() {
        environment.push_str(&format!("- Hardware: {}\n", draft.details.hardware.trim()));
    }
    if !draft.details.build_config.trim().is_empty() {
        environment.push_str(&format!("- Build configuration: {}\n", draft.details.build_config.trim()));
    }
    section(&mut out, "Environment", &environment);

    let mut details = String::new();
    if draft.diagnostics.build && !facts.build.is_empty() {
        details.push_str(&format!("### Build\n\n{}\n\n", facts.build));
    }
    if draft.diagnostics.runtime && !facts.runtime.is_empty() {
        details.push_str(&format!("### Runtime\n\n{}\n\n", facts.runtime));
    }
    if draft.diagnostics.logs && !facts.logs.is_empty() {
        details.push_str(&format!("### Recent build log\n\n```text\n{}\n```\n\n", facts.logs));
    }
    if !details.trim().is_empty() {
        out.push_str("\n## Diagnostics\n\n<details>\n<summary>Collected by EmblemWeave</summary>\n\n");
        out.push_str(details.trim_end());
        out.push_str("\n\n</details>\n");
    }

    if !draft.attachments.is_empty() {
        let mut files = String::new();
        for a in &draft.attachments {
            files.push_str(&format!("- `{}` — {}, {}\n", a.name, fmt_size(a.size), a.kind.label()));
        }
        files.push_str(
            "\n> These files are not uploaded by EmblemWeave. Drag them into this issue before submitting.\n",
        );
        section(&mut out, "Attachments", &files);
    }
    out.trim_end().to_string() + "\n"
}

/// Everything the preview and the GitHub handoff need, built from one draft so
/// the title, chips, body and attachment list cannot disagree.
#[derive(Debug, Clone)]
pub struct Preview {
    pub title: String,
    pub category: &'static str,
    pub github_label: Option<&'static str>,
    pub markdown: String,
    pub attachments: Vec<String>,
}

pub fn preview(draft: &Draft, facts: &Facts) -> Preview {
    Preview {
        title: if draft.title.trim().is_empty() { "Untitled report".into() } else { draft.title.trim().to_string() },
        category: draft.category_label(),
        github_label: draft.category.and_then(Category::github_label),
        markdown: markdown(draft, facts),
        attachments: draft.attachments.iter().map(|a| a.name.clone()).collect(),
    }
}

/// A GitHub new-issue URL. The body is capped for transport; the full report is
/// written to the handoff folder whenever the URL is opened.
pub fn issue_url(draft: &Draft, facts: &Facts) -> (String, bool) {
    let category = draft.category.unwrap_or(Category::Other);
    let mut body = markdown(draft, facts);
    let mut truncated = false;
    if body.chars().count() > MAX_URL_BODY {
        body = body.chars().take(MAX_URL_BODY).collect();
        body.push_str("\n\n> The report was shortened for the URL. The complete Markdown is in the prepared report folder.\n");
        truncated = true;
    }
    let mut url = format!(
        "{NEW_ISSUE_URL}?title={}&body={}",
        encode(draft.title.trim()),
        encode(&body)
    );
    if let Some(label) = category.github_label() {
        url.push_str("&labels=");
        url.push_str(&encode(label));
    }
    (url, truncated)
}

fn encode(input: &str) -> String {
    let mut out = String::new();
    for b in input.as_bytes() {
        match *b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(*b as char),
            b' ' => out.push_str("%20"),
            b'\n' => out.push_str("%0A"),
            b'\r' => out.push_str("%0D"),
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

fn stamp() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0)
}

pub fn draft_path(dir: &Path) -> PathBuf {
    dir.join("issue-draft.json")
}

pub fn save_draft(dir: &Path, draft: &Draft) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    let path = draft_path(dir);
    let tmp = path.with_extension("json.tmp");
    let text = serde_json::to_string_pretty(draft).map_err(|e| format!("serialising the draft: {e}"))?;
    std::fs::write(&tmp, text).map_err(|e| format!("writing {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("replacing {}: {e}", path.display()))?;
    Ok(path)
}

pub fn load_draft(dir: &Path) -> Option<Draft> {
    let text = std::fs::read_to_string(draft_path(dir)).ok()?;
    serde_json::from_str(&text).ok()
}

/// Copies the report and every selected attachment into a timestamped folder,
/// and writes the instructions. Archives are copied, never extracted.
pub fn prepare_handoff(root: &Path, draft: &Draft, markdown: &str) -> Result<PathBuf, String> {
    let dir = root.join(format!("issue-report-{}", stamp()));
    std::fs::create_dir_all(&dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    std::fs::write(dir.join("issue.md"), markdown).map_err(|e| format!("writing issue.md: {e}"))?;
    let files = dir.join("attachments");
    if !draft.attachments.is_empty() {
        std::fs::create_dir_all(&files).map_err(|e| format!("creating {}: {e}", files.display()))?;
        for a in &draft.attachments {
            // A draft is local state, but it is still not trusted to choose a path
            // outside the handoff folder: only the leaf name is used.
            let safe = Path::new(&a.name)
                .file_name()
                .and_then(|n| n.to_str())
                .filter(|n| !n.is_empty() && *n != "." && *n != "..")
                .unwrap_or("attachment");
            // Two different folders can hold files with the same name; keep both
            // instead of letting the second copy overwrite the first.
            let mut dest = files.join(safe);
            if dest.exists() {
                let stem = Path::new(safe).file_stem().and_then(|s| s.to_str()).unwrap_or("file");
                let ext = Path::new(safe).extension().and_then(|s| s.to_str()).unwrap_or("");
                for n in 2..1000 {
                    let candidate = if ext.is_empty() {
                        files.join(format!("{stem}-{n}"))
                    } else {
                        files.join(format!("{stem}-{n}.{ext}"))
                    };
                    if !candidate.exists() {
                        dest = candidate;
                        break;
                    }
                }
            }
            std::fs::copy(&a.path, &dest)
                .map_err(|e| format!("copying {} to {}: {e}", a.path.display(), dest.display()))?;
        }
    }
    let mut readme = String::new();
    readme.push_str("EmblemWeave issue handoff\n=========================\n\n");
    readme.push_str("EmblemWeave has NOT submitted an issue. This folder only holds the prepared report.\n\n");
    readme.push_str("1. The GitHub new-issue page was opened in your browser.\n");
    readme.push_str("2. Paste issue.md into the description if the page did not receive it.\n");
    readme.push_str("3. Drag every file from the attachments folder into the issue.\n");
    readme.push_str("4. Review the report for passwords, tokens, and personal paths.\n");
    readme.push_str("5. Submit the issue on GitHub.\n\n");
    if draft.attachments.is_empty() {
        readme.push_str("No files were attached to this report.\n");
    }
    std::fs::write(dir.join("README.txt"), readme).map_err(|e| format!("writing README.txt: {e}"))?;
    Ok(dir)
}

/// Where draft metadata and handoff folders live.
pub fn data_dir() -> Option<PathBuf> {
    Config::dir()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filled(category: Category) -> Draft {
        let mut d = Draft::default();
        d.category = Some(category);
        d.title = "The game stops when the second chapter loads".into();
        d.description = "Starting chapter two ends the runtime with a GPU device lost message.".into();
        d
    }

    fn facts() -> Facts {
        Facts {
            application: "EmblemWeave TEST".into(),
            system: "Windows x86_64 · CPU: Test CPU".into(),
            game: String::new(),
            build: "Built game ready: yes".into(),
            runtime: "No boot test".into(),
            logs: "LOG MARKER".into(),
        }
    }

    #[test]
    fn every_category_has_a_unique_label_and_a_known_label_mapping() {
        let mut labels = std::collections::HashSet::new();
        for c in Category::ALL {
            assert!(labels.insert(c.label()), "duplicate category label {}", c.label());
            assert!(!c.description().is_empty());
            assert!(c.github_label().is_none_or(|l| ["bug", "enhancement", "question", "documentation"].contains(&l)));
        }
        assert_eq!(Category::Crash.github_label(), Some("bug"));
        assert_eq!(Category::FeatureRequest.github_label(), Some("enhancement"));
        assert_eq!(Category::Other.github_label(), None);
    }

    #[test]
    fn conditional_fields_follow_the_category() {
        let bug: Vec<&str> = fields(Category::Bug).iter().map(|f| f.field.key()).collect();
        assert!(bug.contains(&"steps") && bug.contains(&"expected") && bug.contains(&"actual"));
        assert!(!bug.contains(&"first_error"));
        let build: Vec<&str> = fields(Category::BuildFailed).iter().map(|f| f.field.key()).collect();
        assert!(build.contains(&"first_error") && build.contains(&"toolchain"));
        let request: Vec<&str> = fields(Category::FeatureRequest).iter().map(|f| f.field.key()).collect();
        assert!(request.contains(&"problem_solved") && !request.contains(&"steps"));
        let inquiry: Vec<&str> = fields(Category::Inquiry).iter().map(|f| f.field.key()).collect();
        assert!(inquiry.contains(&"info_needed"));
        assert!(!Category::FeatureRequest.asks_for_frequency());
    }

    #[test]
    fn validation_requires_title_category_and_description() {
        let d = Draft::default();
        let v = validate(&d);
        assert!(!v.ok());
        assert!(v.errors.iter().any(|e| e.contains("title")));
        assert!(v.errors.iter().any(|e| e.contains("category")));
        assert!(v.errors.iter().any(|e| e.contains("Description")));
        assert!(validate(&filled(Category::Bug)).ok());
    }

    #[test]
    fn useful_warnings_do_not_block_a_report() {
        let mut d = filled(Category::Bug);
        let v = validate(&d);
        assert!(v.ok());
        assert!(v.warnings.iter().any(|w| w.contains("steps to reproduce")));
        assert!(v.warnings.iter().any(|w| w.contains("files are attached")));

        let mut b = filled(Category::BuildFailed);
        b.details.first_error.clear();
        let v = validate(&b);
        assert!(v.ok());
        assert!(v.warnings.iter().any(|w| w.contains("build log") || w.contains("build error")));
    }

    #[test]
    fn a_whitespace_title_is_rejected() {
        let mut d = filled(Category::Bug);
        d.title = "    \n\t ".into();
        assert!(!validate(&d).ok());
    }

    #[test]
    fn sensitive_content_is_flagged_not_removed() {
        let mut d = filled(Category::Bug);
        d.details.error = "password: hunter2\npath C:\\Users\\someone\\game".into();
        let v = validate(&d);
        assert!(v.ok());
        assert!(v.warnings.iter().any(|w| w.contains("password")));
        assert!(v.warnings.iter().any(|w| w.contains("C:\\Users\\")));
        assert!(d.details.error.contains("hunter2"));
    }

    #[test]
    fn markdown_has_the_bug_sections_and_omits_empty_ones() {
        let mut d = filled(Category::Bug);
        d.details.steps = "Open the launcher\nStart a new game".into();
        d.details.expected = "The chapter loads".into();
        let text = markdown(&d, &facts());
        assert!(text.contains("## Steps to Reproduce"));
        assert!(text.contains("- Open the launcher"));
        assert!(text.contains("## Expected Behavior"));
        assert!(!text.contains("## Already Tried"));
        assert!(text.contains("## Environment"));
        assert!(text.contains("## Diagnostics"));
        assert!(text.contains("EmblemWeave TEST"));
    }

    #[test]
    fn feature_requests_do_not_get_bug_sections() {
        let mut d = filled(Category::FeatureRequest);
        d.details.problem_solved = "I cannot see my play time.".into();
        let text = markdown(&d, &facts());
        assert!(text.contains("## Problem This Solves"));
        assert!(!text.contains("## Steps to Reproduce"));
        assert!(!text.contains("## Expected Behavior"));
    }

    #[test]
    fn diagnostics_selection_controls_what_is_included() {
        let mut d = filled(Category::Bug);
        d.diagnostics.logs = false;
        d.diagnostics.build = false;
        d.diagnostics.runtime = false;
        let text = markdown(&d, &facts());
        assert!(!text.contains("## Diagnostics"));
        assert!(!text.contains("LOG MARKER"));
        d.diagnostics.build = true;
        let text = markdown(&d, &facts());
        assert!(text.contains("Built game ready"));
        assert!(!text.contains("LOG MARKER"));
    }

    #[test]
    fn urls_are_prefilled_and_use_only_existing_labels() {
        let d = filled(Category::Bug);
        let (url, truncated) = issue_url(&d, &facts());
        assert!(url.starts_with(NEW_ISSUE_URL));
        assert!(url.contains("title=The%20game%20stops"));
        assert!(url.contains("labels=bug"));
        assert!(url.contains("body=%2A%2ACategory"));
        assert!(!truncated);
        let mut other = filled(Category::Other);
        other.title = "Something else entirely".into();
        assert!(!issue_url(&other, &facts()).0.contains("labels="));
    }

    #[test]
    fn a_huge_report_is_shortened_for_the_url_but_kept_in_markdown() {
        let mut d = filled(Category::Bug);
        d.description = "x".repeat(MAX_URL_BODY + 500);
        let (url, truncated) = issue_url(&d, &facts());
        assert!(truncated);
        assert!(url.len() < MAX_URL_BODY * 4);
        assert!(markdown(&d, &facts()).len() > MAX_URL_BODY);
    }

    #[test]
    fn duplicate_and_unsafe_attachments_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("game.log");
        std::fs::write(&log, "hello").unwrap();
        let mut d = Draft::default();
        assert!(matches!(add_attachment(&mut d, &log), AddResult::Added(_)));
        assert!(matches!(add_attachment(&mut d, &log), AddResult::Duplicate(_)));
        assert_eq!(d.attachments.len(), 1);

        let exe = dir.path().join("tool.exe");
        std::fs::write(&exe, b"MZ").unwrap();
        assert!(matches!(add_attachment(&mut d, &exe), AddResult::Rejected(_)));

        let empty = dir.path().join("empty.txt");
        std::fs::write(&empty, "").unwrap();
        assert!(matches!(add_attachment(&mut d, &empty), AddResult::Rejected(_)));
        assert_eq!(d.attachments.len(), 1);
    }

    #[test]
    fn logs_images_and_archives_are_classified_and_copied_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let zip = dir.path().join("report.zip");
        std::fs::write(&zip, b"PK\x03\x04 not really a zip").unwrap();
        let image = dir.path().join("shot.png");
        std::fs::write(&image, b"\x89PNG\r\n").unwrap();
        let mut d = Draft::default();
        assert!(matches!(add_attachment(&mut d, &zip), AddResult::Added(_)));
        assert!(matches!(add_attachment(&mut d, &image), AddResult::Added(_)));
        assert_eq!(d.attachments[0].kind, AttachmentKind::Archive);
        assert_eq!(d.attachments[1].kind, AttachmentKind::Image);
        let out = prepare_handoff(dir.path(), &d, "body").unwrap();
        assert_eq!(std::fs::read(out.join("attachments/report.zip")).unwrap(), b"PK\x03\x04 not really a zip");
        // The archive is copied as one file; nothing is extracted beside it.
        assert!(!out.join("attachments/report").exists());
    }

    #[test]
    fn credential_files_are_refused_by_extension_and_name() {
        let dir = tempfile::tempdir().unwrap();
        let env = dir.path().join(".env");
        std::fs::write(&env, "TOKEN=secret").unwrap();
        let key = dir.path().join("signing.key");
        std::fs::write(&key, "secret").unwrap();
        let rsa = dir.path().join("id_rsa");
        std::fs::write(&rsa, "secret").unwrap();
        let mut d = Draft::default();
        for path in [&env, &key, &rsa] {
            match add_attachment(&mut d, path) {
                AddResult::Rejected(why) => assert!(why.contains("credential")),
                _ => panic!("{} must be refused", path.display()),
            }
        }
        assert!(d.attachments.is_empty());
    }

    #[test]
    fn the_attachment_count_cap_matches_github() {
        let dir = tempfile::tempdir().unwrap();
        let mut d = Draft::default();
        for i in 0..MAX_ATTACHMENTS {
            let path = dir.path().join(format!("evidence-{i}.log"));
            std::fs::write(&path, "line").unwrap();
            assert!(matches!(add_attachment(&mut d, &path), AddResult::Added(_)));
        }
        let extra = dir.path().join("one-too-many.log");
        std::fs::write(&extra, "line").unwrap();
        match add_attachment(&mut d, &extra) {
            AddResult::Rejected(why) => assert!(why.contains("at most")),
            _ => panic!("the cap must be enforced"),
        }
        assert_eq!(d.attachments.len(), MAX_ATTACHMENTS);
        assert_eq!(MAX_ATTACHMENTS, 10);
    }

    #[test]
    fn handoff_never_uses_an_attachment_name_as_a_path() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("source.log");
        std::fs::write(&src, "body").unwrap();
        let mut d = filled(Category::Bug);
        d.attachments.push(Attachment {
            path: src,
            name: "../escape.log".into(),
            size: 4,
            kind: AttachmentKind::Log,
        });
        let out = prepare_handoff(dir.path(), &d, "body").unwrap();
        assert!(out.join("attachments/escape.log").is_file());
        assert!(!dir.path().join("escape.log").exists());
    }

    #[test]
    fn oversized_attachments_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let big = dir.path().join("big.log");
        let file = std::fs::File::create(&big).unwrap();
        file.set_len(MAX_FILE_BYTES + 1).unwrap();
        let mut d = Draft::default();
        match add_attachment(&mut d, &big) {
            AddResult::Rejected(why) => assert!(why.contains("limit")),
            _ => panic!("an oversized file must be rejected"),
        }
    }

    #[test]
    fn drafts_survive_save_and_load() {
        let dir = tempfile::tempdir().unwrap();
        let mut d = filled(Category::Performance);
        d.attachments.push(Attachment {
            path: PathBuf::from("C:/logs/game.log"),
            name: "game.log".into(),
            size: 42,
            kind: AttachmentKind::Log,
        });
        d.diagnostics.logs = false;
        save_draft(dir.path(), &d).unwrap();
        let loaded = load_draft(dir.path()).expect("draft loads");
        assert_eq!(loaded.title, d.title);
        assert_eq!(loaded.category, d.category);
        assert_eq!(loaded.attachments.len(), 1);
        assert!(!loaded.diagnostics.logs);
    }

    #[test]
    fn handoff_copies_files_and_never_claims_submission() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("game.log");
        std::fs::write(&src, "log body").unwrap();
        let mut d = filled(Category::Bug);
        d.attachments.push(Attachment { path: src, name: "game.log".into(), size: 8, kind: AttachmentKind::Log });
        let out = prepare_handoff(dir.path(), &d, "## Summary\n\nbody\n").unwrap();
        assert!(out.join("issue.md").is_file());
        assert!(out.join("attachments/game.log").is_file());
        let readme = std::fs::read_to_string(out.join("README.txt")).unwrap();
        assert!(readme.contains("NOT submitted"));
        assert!(!readme.to_ascii_lowercase().contains("successfully submitted"));
    }

    #[test]
    fn preview_agrees_with_the_report_and_lists_attachments() {
        let mut d = filled(Category::Bug);
        d.attachments.push(Attachment {
            path: PathBuf::from("C:/logs/game.log"),
            name: "game.log".into(),
            size: 10,
            kind: AttachmentKind::Log,
        });
        let p = preview(&d, &facts());
        assert_eq!(p.title, d.title);
        assert_eq!(p.category, "Bug");
        assert_eq!(p.github_label, Some("bug"));
        assert_eq!(p.attachments, vec!["game.log".to_string()]);
        assert!(p.markdown.contains("## Summary"));
    }

    #[test]
    fn attachments_can_be_removed_without_touching_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("one.log");
        std::fs::write(&log, "one").unwrap();
        let mut d = filled(Category::Bug);
        assert!(matches!(add_attachment(&mut d, &log), AddResult::Added(_)));
        let removed = remove_attachment(&mut d, 0).expect("removed");
        assert_eq!(removed.name, "one.log");
        assert!(d.attachments.is_empty());
        assert_eq!(d.title, filled(Category::Bug).title);
        assert!(remove_attachment(&mut d, 0).is_none());
    }

    #[test]
    fn handoff_failure_surfaces_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("not-a-folder");
        std::fs::write(&file, "x").unwrap();
        let d = filled(Category::Bug);
        assert!(prepare_handoff(&file, &d, "body").is_err());
    }

    #[test]
    fn a_missing_attachment_is_a_validation_error() {
        let mut d = filled(Category::Bug);
        d.attachments.push(Attachment {
            path: PathBuf::from("C:/definitely/missing.log"),
            name: "missing.log".into(),
            size: 10,
            kind: AttachmentKind::Log,
        });
        let v = validate(&d);
        assert!(!v.ok());
        assert!(v.errors.iter().any(|e| e.contains("no longer exists")));
    }
}
