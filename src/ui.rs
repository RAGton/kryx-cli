// SPDX-License-Identifier: MIT
//
// ui.rs — visual layer for kryx
//
// Single source of truth for: banner ASCII, color palette, spinner/progress
// helpers, and the KryonixDashboard panel. Every long-running command
// (switch, update, gc, clean) wraps its work in a `KryxSpinner` so the
// terminal output is consistent across the whole CLI.
//
// Design contract:
//   - All visual output is via stdout EXCEPT diagnostics, which use
//     `ui::warn` / `ui::error` (stderr, with color, with [WARN]/[ERROR] tag).
//   - The banner is OPTIONAL and OFF by default; it shows on `--version`,
//     `kryx --help`, and at the top of `status` / `doctor` / `switch` / `update` / `gc`.
//   - TTY detection: spinners/progress bars only show when stderr is a TTY.
//     In non-TTY mode (CI, pipes, scripts) we fall back to plain text so
//     output stays parseable.

use colored::Colorize;
use indicatif::{ProgressBar, ProgressStyle};
use once_cell::sync::Lazy;
use std::io::IsTerminal;
use std::time::Duration;

/// ASCII banner for the kryx CLI. Rendered once at the top of major
/// commands. Uses Unicode box-drawing characters for crisp alignment
/// in monospace terminals.
pub const BANNER: &str = r"
   _  _______   ______  _   _ ___ __  __   ___  ____
  | |/ /  __ \ / __  \| \ | |_ _|\ \/ /  / _ \/ ___|
  | ' /| |__) | |  | ||  \| || |  \  /  | | | \___ \
  | . \|  _  /| |  | || . ` || |  /  \  | |_| |___) |
  |_|\_\_| \_\\____/ |_|\_|___|/_/\_\\___\___/____/
              Unified CLI · KryonixOS
";

/// The kryx version, sourced from the Cargo.toml of the running build.
/// Surfaced in `--version` and at the top of long-running commands.
pub static VERSION: Lazy<String> = Lazy::new(|| env!("CARGO_PKG_VERSION").to_string());

/// Indent prefix used for all visual blocks (banner, panels, tables).
/// 2 spaces keeps it visually grouped without screaming.
const INDENT: &str = "  ";

/// Render the full banner with current version. Caller controls where
/// to print it (stdout is the convention).
pub fn render_banner() -> String {
    let version_line = format!(
        "{}v{} · KryonixOS Unified CLI",
        INDENT,
        VERSION.as_str().cyan().bold()
    );
    // The banner already has a trailing newline; append the version line
    // on a fresh line so it visually anchors the art.
    format!("{}{}\n", BANNER, version_line)
}

/// Print the banner to stdout. Convenience wrapper used in main.rs and
/// in the long-running command entry points.
pub fn print_banner() {
    if !should_show_visuals() {
        return;
    }
    print!("{}", render_banner());
}

// ── Diagnostics: stderr with consistent tags ─────────────────────────

/// Print an informational message to stderr (so it doesn't pollute
/// structured stdout consumers in JSON mode). Use sparingly — most
/// output should be plain `println!` to stdout.
pub fn info(msg: &str) {
    eprintln!("{} {}", "[INFO]".cyan().bold(), msg);
}

/// Print a warning to stderr. Yellow tag.
pub fn warn(msg: &str) {
    eprintln!("{} {}", "[WARN]".yellow().bold(), msg);
}

/// Print an error to stderr. Red tag.
pub fn error(msg: &str) {
    eprintln!("{} {}", "[ERROR]".red().bold(), msg);
}

/// Print a success/pass line to stderr. Green tag.
pub fn pass(msg: &str) {
    eprintln!("{} {}", "[PASS]".green().bold(), msg);
}

/// Print a step (sub-phase) line to stderr. Cyan tag with a leading
/// arrow so it reads as "→ doing this now".
pub fn step(msg: &str) {
    eprintln!("{} {}", "→".cyan().bold(), msg);
}

// ── Spinner / progress ──────────────────────────────────────────────

/// Spinner / progress bar wrapper. Auto-disables in non-TTY environments
/// so piped output stays clean.
pub struct KryxSpinner {
    pb: Option<ProgressBar>,
}

impl KryxSpinner {
    /// Start a new spinner with the given message. If stderr is not a
    /// TTY, returns a "quiet" spinner that just prints the message
    /// once and silently finishes.
    pub fn new(message: &str) -> Self {
        let pb = if should_show_visuals() {
            let pb = ProgressBar::new_spinner();
            pb.set_style(
                ProgressStyle::with_template("{spinner:.cyan.bold} {msg}")
                    .expect("template parse")
                    .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]),
            );
            pb.set_message(message.to_string());
            pb.enable_steady_tick(Duration::from_millis(80));
            Some(pb)
        } else {
            // Non-TTY: still surface the message once so user sees
            // something is happening, but no spinner.
            eprintln!("{} {}", "→".cyan().bold(), message);
            None
        };
        Self { pb }
    }

    /// Update the spinner message while spinning. Safe to call
    /// repeatedly; the renderer updates the bar in place.
    pub fn set_message(&self, msg: &str) {
        if let Some(pb) = &self.pb {
            pb.set_message(msg.to_string());
        }
    }

    /// Finish the spinner with a success message (green checkmark).
    pub fn finish_success(&self, msg: &str) {
        if let Some(pb) = &self.pb {
            pb.finish_with_message(format!("{} {}", "✓".green().bold(), msg));
        } else {
            eprintln!("{} {}", "✓".green().bold(), msg);
        }
    }

    /// Finish the spinner with a failure message (red X).
    pub fn finish_error(&self, msg: &str) {
        if let Some(pb) = &self.pb {
            pb.finish_with_message(format!("{} {}", "✗".red().bold(), msg));
        } else {
            eprintln!("{} {}", "✗".red().bold(), msg);
        }
    }

    /// Finish the spinner and clear the line (no message printed).
    pub fn finish_and_clear(&self) {
        if let Some(pb) = &self.pb {
            pb.finish_and_clear();
        }
    }
}

impl Drop for KryxSpinner {
    /// Defensive: if the spinner is dropped without an explicit finish,
    /// finish it cleanly so we don't leave a stuck cursor.
    fn drop(&mut self) {
        if let Some(pb) = self.pb.take()
            && !pb.is_finished()
        {
            pb.finish_and_clear();
        }
    }
}

/// Indeterminate progress bar (no fixed total). For commands that
/// don't know how long they'll take but want a visible indicator.
pub fn indeterminate_progress(msg: &str) -> KryxSpinner {
    KryxSpinner::new(msg)
}

// ── TTY detection ───────────────────────────────────────────────────

/// Returns true if the current stderr is a TTY (and therefore it's
/// safe to render colors, spinners, and progress bars). Returns
/// false in piped/CI contexts so output stays parseable.
pub fn should_show_visuals() -> bool {
    std::io::stderr().is_terminal()
}

// ── Boxed section dividers (used in long outputs) ───────────────────

/// Render a horizontal divider for visual section breaks.
pub fn divider() -> String {
    "─".repeat(72)
}

/// Render a section header with an underline divider.
pub fn section_header(title: &str) -> String {
    format!("{}\n{}{}", title.bold().underline(), divider().dimmed(), "")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn banner_is_non_empty_and_has_version() {
        let b = render_banner();
        assert!(!b.is_empty());
        assert!(b.contains("KryonixOS"));
        assert!(b.contains(&VERSION.as_str().to_string()));
    }

    #[test]
    fn section_header_contains_title() {
        let h = section_header("Phase A");
        assert!(h.contains("Phase A"));
        assert!(h.contains("─"));
    }

    #[test]
    fn spinner_finishes_cleanly() {
        let s = KryxSpinner::new("test");
        s.finish_success("done");
        // No panic, no leftover cursor.
    }

    #[test]
    fn spinner_quiet_fallback_does_not_panic() {
        // When stderr is not a TTY (test env), the spinner returns
        // the quiet variant. Both branches must be safe to call.
        let s = KryxSpinner::new("test-quiet");
        s.set_message("updated");
        s.finish_success("ok");
    }
}
