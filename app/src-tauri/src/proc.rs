//! Spawning helper processes without putting a window on screen.
//!
//! ⚠️ On Windows a GUI application that starts a console program gets a
//! CONSOLE WINDOW for it — on Windows 11 that is a full Windows Terminal
//! window, titled `C:\WINDOWS\System32\OpenSSH\ssh.exe`, which sits in front
//! of the app for as long as the child runs. BHTerminal starts ssh for the
//! file browser, for port forwards and for compressing on the server, so
//! without this every one of those put a black window on the user's desktop.
//!
//! The terminal panes are unaffected: those go through a pseudoconsole
//! (ConPTY), which has no window of its own.

use std::process::Command;

/// `Command::new`, minus the console window on Windows.
pub fn command(program: &str) -> Command {
    // `mut` is only needed on Windows, where the flag is set below.
    #[allow(unused_mut)]
    let mut command = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        /// CREATE_NO_WINDOW — the child runs with no console of its own. Its
        /// stdio is piped in every case here, so nothing is lost by it.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}
