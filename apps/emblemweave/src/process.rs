//! Centralized process utilities for suppressing console windows on Windows.

use std::process::Command;

/// Apply `CREATE_NO_WINDOW` on Windows to prevent console window flashing.
/// This should be used for all build subprocesses, dependency checks, and
/// background operations. GUI applications (explorer, notepad) and the game
/// runtime (controlled by `show_debug_console` setting) are exempt.
pub fn hidden(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    cmd
}

/// Convenience: create a new command with console suppression already applied.
pub fn new_hidden(program: impl AsRef<std::ffi::OsStr>) -> Command {
    let mut cmd = Command::new(program);
    hidden(&mut cmd);
    cmd
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    #[test]
    fn hidden_applies_creation_flags_on_windows() {
        let mut cmd = Command::new("echo");
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            // We can't directly inspect creation_flags, but we can verify the function runs
            hidden(&mut cmd);
        }
        #[cfg(not(windows))]
        {
            hidden(&mut cmd); // Should not panic
        }
    }

    #[test]
    fn new_hidden_creates_command() {
        let _cmd = new_hidden("echo");
    }
}