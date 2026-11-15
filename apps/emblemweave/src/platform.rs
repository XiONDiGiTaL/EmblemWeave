//! What the rest of the launcher asks the operating system for, in one place.
//!
//! Every question the launcher needs an OS to answer goes through here, so the
//! Windows assumptions are visible in one file instead of scattered through
//! pages and job code (docs/PLATFORMS.md has the audit that led to it):
//!
//! * whether the game is running (a process list),
//! * whether the launcher may run dependency installers (winget),
//! * whether this platform can build the game at all (MSVC through
//!   `vcvars64.bat`, CMake, Ninja),
//! * the folders the launcher keeps its own data in,
//! * the name of the platform the interface shows in a diagnostic line.
//!
//! Nothing here is a promise that a non-Windows build plays the game: the
//! engine is Windows-only today, and docs/LINUX.md is the plan that has to be
//! built and tested before that changes. The point of this module is that a
//! build for another platform cannot accidentally run a winget line, write to
//! `%APPDATA%`, or think a game is running because it read a Windows process
//! list.

/// Whether this build targets Windows. The code below is `cfg`-selected, so
/// this is a compile-time fact, not a runtime probe.
pub const WINDOWS: bool = cfg!(windows);

/// The platform the interface names when it explains what it can do here.
pub fn name() -> &'static str {
    if WINDOWS {
        "Windows"
    } else if cfg!(target_os = "macos") {
        "macOS"
    } else if cfg!(target_os = "linux") {
        "Linux"
    } else {
        std::env::consts::OS
    }
}

/// Whether the game's process is running.
///
/// On Windows this reads the process list; elsewhere there is no list to read,
/// so the answer is "no", which is what a save editor needs before it writes.
pub fn game_running() -> bool {
    cfg!(windows) && crate::cheats::process::find(crate::cheats::GAME_EXE).is_some()
}

/// Whether the launcher can start a dependency installer itself.
///
/// The registry's install lines are `winget` commands (and one `py -m pip`
/// line) run from a `.cmd` script. Off Windows that is not a supported way to
/// install anything, and running it would fail with a shell error the user
/// cannot act on, so the button is refused with a sentence that says why.
pub fn install_supported() -> bool {
    WINDOWS
}

/// Why `install_supported` said no, for the message the interface shows.
pub fn install_unsupported_reason() -> String {
    format!(
        "installing build tools is a Windows step (winget and a .cmd script); on {} use your distribution's packages \
         (docs/LINUX.md lists the ones the engine needs)",
        name()
    )
}

/// Whether this platform can build the game.
///
/// The engine is compiled by MSVC's clang-cl through `vcvars64.bat` and links
/// the lifted libraries built the same way. That toolchain exists on Windows
/// only, so this is the single place that says so; `engine::build` asks here
/// before it starts a job that would fail 90 steps in.
pub fn build_supported() -> bool {
    WINDOWS
}

/// Why `build_supported` said no.
pub fn build_unsupported_reason() -> String {
    format!(
        "EmblemWeave builds the game with the Windows toolchain (Visual Studio's clang-cl through vcvars64.bat). \
         {} support is planned but has not been built and tested, so it is not advertised: docs/LINUX.md has the \
         implementation plan and the build requirements",
        name()
    )
}

/// The launcher's own data folder, the one that holds `play`, `work`, the logs
/// and the save folder.
///
/// Windows: `%APPDATA%\EmblemWeave`. Linux: `$XDG_CONFIG_HOME/EmblemWeave`, else
/// `$HOME/.config/EmblemWeave` (the same place a native build would keep it; the
/// name is the same so a future port can read a folder made by hand).
pub fn data_root_from(env: &dyn Fn(&str) -> Option<String>) -> Option<std::path::PathBuf> {
    if WINDOWS {
        return env("APPDATA").filter(|s| !s.is_empty()).map(|s| std::path::PathBuf::from(s).join("EmblemWeave"));
    }
    env("XDG_CONFIG_HOME")
        .filter(|s| !s.is_empty())
        .map(|s| std::path::PathBuf::from(s).join("EmblemWeave"))
        .or_else(|| env("HOME").filter(|s| !s.is_empty()).map(|s| std::path::PathBuf::from(s).join(".config").join("EmblemWeave")))
}

/// `data_root_from` against this process's environment.
pub fn data_root() -> Option<std::path::PathBuf> {
    data_root_from(&|k| std::env::var(k).ok())
}

/// The sentence a job shows when `data_root` found nothing: it names the variable per platform instead
/// of asking a Linux user for `%APPDATA%`.
pub const NO_DATA_DIR: &str = if cfg!(windows) {
    "APPDATA is not set"
} else {
    "neither XDG_CONFIG_HOME nor HOME is set"
};

#[cfg(test)]
mod tests {
    use super::*;

    fn env_of(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |k| pairs.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string())
    }

    #[test]
    fn the_data_folder_is_appdata_on_windows_and_xdg_elsewhere() {
        let win = data_root_from(&env_of(&[("APPDATA", "/home/x/.config")]));
        if WINDOWS {
            assert_eq!(win.unwrap(), std::path::PathBuf::from("/home/x/.config/EmblemWeave"));
        }
        let xdg = data_root_from(&env_of(&[("XDG_CONFIG_HOME", "/home/x/cfg"), ("HOME", "/home/x")]));
        let home = data_root_from(&env_of(&[("HOME", "/home/x")]));
        if !WINDOWS {
            assert_eq!(xdg.unwrap(), std::path::PathBuf::from("/home/x/cfg/EmblemWeave"));
            assert_eq!(home.unwrap(), std::path::PathBuf::from("/home/x/.config/EmblemWeave"));
        }
        assert!(data_root_from(&env_of(&[])).is_none());
    }

    #[test]
    fn the_install_and_build_paths_match_the_platform() {
        assert_eq!(install_supported(), WINDOWS);
        assert_eq!(build_supported(), WINDOWS);
        if !WINDOWS {
            assert!(install_unsupported_reason().contains(name()));
            assert!(build_unsupported_reason().contains("docs/LINUX.md"));
        }
    }

}
