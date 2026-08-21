// SPDX-License-Identifier: MIT
//
// cleanup.rs — automatic garbage collection + temp cleanup
//
// Two responsibilities, both called from kryx switch / update / clean:
//
// 1. `run_auto_gc(older_than)` — wraps `nix-collect-garbage -d` with a
//    retention window. Default 2d. Idempotent, safe to call after every
//    successful build. Skips silently if `--no-gc` is passed.
//
// 2. `run_cleanup_pass()` — removes transient junk left by build
//    processes: `.hm-bak-*` files (Home Manager backup dirs from
//    `xdg.configFile.<name>.backup = true`), `result` symlinks from
//    `nix build`, `/tmp/kryx-*` tmpfiles, and the target/ build cache
//    inside the kryx-cli repo (gated by env var KRYX_PRUNE_TARGET).
//
// Both functions are *opt-out*, never opt-in: the goal is that running
// `kryx switch` leaves the system cleaner than it found it.

use crate::ui;
use colored::Colorize;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

/// Default GC retention window. 2 days matches the canonical NixOS
/// recommendation for desktop/laptop users: short enough to free disk
/// after a typical upgrade cycle, long enough to allow rollback via
/// `nixos-rebuild --rollback` to any generation in the last 48h.
pub const DEFAULT_GC_KEEP: &str = "2d";

/// Walk through the GC + cleanup pass. Returns a summary that the caller
/// can print via `ui::pass(...)` for a satisfying "system cleaned"
/// confirmation.
#[derive(Debug, Default)]
pub struct CleanupReport {
    pub gc_runs: u32,
    pub gc_bytes_freed: u64,
    pub hm_baks_removed: u32,
    pub result_symlinks_removed: u32,
    pub tmp_files_removed: u32,
    pub errors: Vec<String>,
}

impl CleanupReport {
    pub fn is_empty(&self) -> bool {
        self.gc_runs == 0
            && self.hm_baks_removed == 0
            && self.result_symlinks_removed == 0
            && self.tmp_files_removed == 0
            && self.errors.is_empty()
    }

    /// Compact one-line summary, used in the post-cleanup log line.
    pub fn summary(&self) -> String {
        if self.is_empty() {
            return "nothing to clean".to_string();
        }
        let mut parts = Vec::new();
        if self.gc_runs > 0 {
            parts.push(format!("gc×{}", self.gc_runs));
        }
        if self.gc_bytes_freed > 0 {
            parts.push(format!("{} freed", human_bytes(self.gc_bytes_freed)));
        }
        if self.hm_baks_removed > 0 {
            parts.push(format!("{} hm-bak", self.hm_baks_removed));
        }
        if self.result_symlinks_removed > 0 {
            parts.push(format!("{} result", self.result_symlinks_removed));
        }
        if self.tmp_files_removed > 0 {
            parts.push(format!("{} tmp", self.tmp_files_removed));
        }
        if !self.errors.is_empty() {
            parts.push(format!("{} errors", self.errors.len()));
        }
        parts.join(" · ")
    }
}

/// Run the full cleanup pass: auto-gc + .hm-bak-*, result*, /tmp/kryx-*.
/// Caller passes `older_than` (e.g. "2d", "7d", "1h") and an optional
/// `dry_run` flag. The `no_gc` flag short-circuits the GC step.
///
/// This is the *entry point* used by kryx switch / update / clean.
pub fn run_full_cleanup(older_than: &str, no_gc: bool, dry_run: bool) -> CleanupReport {
    let mut report = CleanupReport::default();

    ui::step("Cleanup pass starting (auto-gc + temp files)");

    // 1. Auto GC (unless suppressed)
    if no_gc {
        ui::info("Skipping auto-gc (--no-gc passed)");
    } else {
        match run_auto_gc(older_than, dry_run) {
            Ok(freed) => {
                report.gc_runs += 1;
                report.gc_bytes_freed = freed;
            }
            Err(e) => {
                report.errors.push(format!("gc: {}", e));
                ui::warn(&format!("auto-gc failed: {}", e));
            }
        }
    }

    // 2. .hm-bak-* in HOME
    match remove_hm_backups(dry_run) {
        Ok(n) => report.hm_baks_removed = n,
        Err(e) => {
            report.errors.push(format!("hm-bak: {}", e));
        }
    }

    // 3. result symlinks in CWD
    match remove_result_symlinks(dry_run) {
        Ok(n) => report.result_symlinks_removed = n,
        Err(e) => {
            report.errors.push(format!("result: {}", e));
        }
    }

    // 4. /tmp/kryx-*
    match remove_kryx_tmpfiles(dry_run) {
        Ok(n) => report.tmp_files_removed = n,
        Err(e) => {
            report.errors.push(format!("tmp: {}", e));
        }
    }

    report
}

// ── Step 1: auto-gc ──────────────────────────────────────────────────

/// Invoke `nix-collect-garbage -d` with a retention window. Returns
/// the number of bytes freed (best-effort: parsed from stdout; if the
/// binary doesn't report, returns 0 and the caller just sees a
/// "ran successfully" line).
pub fn run_auto_gc(older_than: &str, dry_run: bool) -> Result<u64, String> {
    if dry_run {
        ui::info(&format!(
            "DRY-RUN: would run nix-collect-garbage -d --delete-older-than {}",
            older_than
        ));
        return Ok(0);
    }

    let spinner = ui::indeterminate_progress(&format!(
        "Running nix-collect-garbage (delete-older-than {})...",
        older_than.cyan()
    ));

    // Discover the real nix-collect-garbage binary (cli-lockdown
    // installs a small wrapper that won't run as root, so we go
    // straight to the store-backed binary).
    let ncg = crate::services::passthrough::discover_real_bin("nix-collect-garbage").ok_or_else(
        || {
            "Could not locate a real nix-collect-garbage binary. \
             The Kryonix cli-lockdown may have removed it; \
             use the canonical path /run/current-system/sw/bin/nix-collect-garbage"
                .to_string()
        },
    )?;

    let output = Command::new(&ncg)
        .arg("-d")
        .arg("--delete-older-than")
        .arg(older_than)
        .output()
        .map_err(|e| format!("failed to spawn nix-collect-garbage: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        spinner.finish_error(&format!("nix-collect-garbage exited {}", output.status));
        return Err(format!("exit {}: {}", output.status, stderr.trim()));
    }

    // Best-effort parse of "X bytes freed" from stdout. nix-collect-garbage
    // doesn't always print this; fall back to 0 if not found.
    let stdout = String::from_utf8_lossy(&output.stdout);
    let bytes_freed = parse_bytes_freed(&stdout);

    let summary = if bytes_freed > 0 {
        format!("freed {}", human_bytes(bytes_freed))
    } else {
        "done".to_string()
    };
    spinner.finish_success(&format!("nix-collect-garbage {}", summary));
    Ok(bytes_freed)
}

/// Best-effort extraction of the freed-bytes figure from nix-collect-garbage
/// output. nix-collect-garbage prints lines like:
///   "XXX.YYY MiB freed"
///   "XXX.YYY KiB freed"
/// or sometimes nothing at all. We try the regex; on miss, return 0.
fn parse_bytes_freed(stdout: &str) -> u64 {
    for line in stdout.lines() {
        let lower = line.to_ascii_lowercase();
        if let Some(idx) = lower.find("freed") {
            let head = &lower[..idx].trim();
            // head looks like "xxx.y kib" or "xxx.y mib"
            let mut parts = head.split_whitespace().rev();
            let unit = parts.next().unwrap_or("");
            let num_str = parts.next().unwrap_or("0");
            if let Ok(n) = num_str.parse::<f64>() {
                let mult: u64 = match unit {
                    "b" => 1,
                    "kib" | "kb" => 1024,
                    "mib" | "mb" => 1024 * 1024,
                    "gib" | "gb" => 1024 * 1024 * 1024,
                    _ => 1,
                };
                return (n * mult as f64) as u64;
            }
        }
    }
    0
}

// ── Step 2: .hm-bak-* ────────────────────────────────────────────────

/// Remove Home Manager backup directories left by `hm-switch` when
/// `xdg.configFile.<name>.force = true` triggers a backup. These
/// accumulate in $HOME over time and can grow to GBs.
pub fn remove_hm_backups(dry_run: bool) -> Result<u32, String> {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    let home_path = PathBuf::from(&home);
    if !home_path.is_dir() {
        return Ok(0);
    }

    let entries = fs::read_dir(&home_path).map_err(|e| format!("cannot read HOME dir: {}", e))?;

    let mut removed = 0u32;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        // Match the Home Manager backup naming convention: starts with
        // "." and contains ".hm-bak-". Common forms: ".config.hm-bak-..."
        if name.starts_with(".") && name.contains(".hm-bak-") {
            let path = entry.path();
            if dry_run {
                ui::info(&format!("DRY-RUN: would remove {}", path.display()));
            } else {
                let res = if path.is_dir() {
                    fs::remove_dir_all(&path)
                } else {
                    fs::remove_file(&path)
                };
                match res {
                    Ok(()) => removed += 1,
                    Err(e) => ui::warn(&format!("failed to remove {}: {}", path.display(), e)),
                }
            }
        }
    }
    if removed > 0 {
        ui::step(&format!("Removed {} .hm-bak-* dirs in $HOME", removed));
    }
    Ok(removed)
}

// ── Step 3: result symlinks ──────────────────────────────────────────

/// Remove `result*` symlinks from CWD. These are created by `nix build`
/// and are technically GC roots; removing them lets the next gc free
/// the underlying store paths.
pub fn remove_result_symlinks(dry_run: bool) -> Result<u32, String> {
    let cwd = std::env::current_dir().map_err(|e| format!("cannot read CWD: {}", e))?;
    let entries = fs::read_dir(&cwd).map_err(|e| format!("cannot read CWD: {}", e))?;

    let mut removed = 0u32;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        // Match the canonical `nix build` symlink and common
        // variations (e.g. "result-2" from repeated builds).
        if name == "result" || (name.starts_with("result-") && name.len() <= 12) {
            let path = entry.path();
            let meta = match fs::symlink_metadata(&path) {
                Ok(m) => m,
                Err(_) => continue,
            };
            // Only operate on symlinks; "result" as a real dir/file
            // is suspicious but leave it alone.
            if meta.file_type().is_symlink() {
                if dry_run {
                    ui::info(&format!("DRY-RUN: would remove symlink {}", path.display()));
                } else {
                    match fs::remove_file(&path) {
                        Ok(()) => removed += 1,
                        Err(e) => ui::warn(&format!("failed to remove {}: {}", path.display(), e)),
                    }
                }
            }
        }
    }
    if removed > 0 {
        ui::step(&format!("Removed {} result* symlinks from CWD", removed));
    }
    Ok(removed)
}

// ── Step 4: /tmp/kryx-* ─────────────────────────────────────────────

/// Remove transient tmpfiles created by kryx itself (and named with
/// the `kryx-` prefix to avoid clobbering anything else). Examples:
/// `kryx-doctor-XXXX.json`, `kryx-build-XXXX.log`.
pub fn remove_kryx_tmpfiles(dry_run: bool) -> Result<u32, String> {
    let tmp = std::env::temp_dir();
    if !tmp.is_dir() {
        return Ok(0);
    }
    let entries = match fs::read_dir(&tmp) {
        Ok(e) => e,
        Err(_) => return Ok(0),
    };

    let mut removed = 0u32;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        // Conservative: only kryx-prefixed files. Never touch other
        // tools' tmpfiles even if they look stale.
        if name.starts_with("kryx-") {
            let path = entry.path();
            if dry_run {
                ui::info(&format!("DRY-RUN: would remove {}", path.display()));
            } else {
                let res = if path.is_dir() {
                    fs::remove_dir_all(&path)
                } else {
                    fs::remove_file(&path)
                };
                match res {
                    Ok(()) => removed += 1,
                    Err(e) => ui::warn(&format!("failed to remove {}: {}", path.display(), e)),
                }
            }
        }
    }
    if removed > 0 {
        ui::step(&format!("Removed {} /tmp/kryx-* tmpfiles", removed));
    }
    Ok(removed)
}

// ── Step 5: target/ build cache (opt-in) ─────────────────────────────

/// Remove the `target/` build cache of the kryx-cli repo. Off by
/// default; only triggered when KRYX_PRUNE_TARGET=1 is set. Useful
/// for the cleanup command when developing on kryx-cli itself.
pub fn prune_target_cache(workspace: &std::path::Path, dry_run: bool) -> Result<u64, String> {
    let target = workspace.join("target");
    if !target.is_dir() {
        return Ok(0);
    }
    let size = dir_size(&target).unwrap_or(0);
    if dry_run {
        ui::info(&format!(
            "DRY-RUN: would remove {} ({} bytes)",
            target.display(),
            size
        ));
        return Ok(size);
    }
    ui::info(&format!(
        "Pruning {} ({} bytes)",
        target.display(),
        human_bytes(size)
    ));
    fs::remove_dir_all(&target).map_err(|e| format!("remove_dir_all: {}", e))?;
    Ok(size)
}

// ── Utilities ────────────────────────────────────────────────────────

/// Walk a directory and return its total size in bytes. Used to print
/// a human-readable "freed N MB" line for the target/ cache prune.
fn dir_size(path: &std::path::Path) -> std::io::Result<u64> {
    let mut total = 0u64;
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let meta = entry.metadata()?;
        if meta.is_dir() {
            total += dir_size(&entry.path())?;
        } else {
            total += meta.len();
        }
    }
    Ok(total)
}

/// Convert a byte count to a human-readable string. Uses binary units
/// (KiB/MiB/GiB) which is the convention for disk space reporting.
pub fn human_bytes(n: u64) -> String {
    const UNITS: &[&str] = &["B", "KiB", "MiB", "GiB", "TiB"];
    let mut size = n as f64;
    let mut idx = 0;
    while size >= 1024.0 && idx < UNITS.len() - 1 {
        size /= 1024.0;
        idx += 1;
    }
    if idx == 0 {
        format!("{} {}", n, UNITS[0])
    } else {
        format!("{:.1} {}", size, UNITS[idx])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn human_bytes_formats_correctly() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(1024), "1.0 KiB");
        assert_eq!(human_bytes(1024 * 1024), "1.0 MiB");
        assert_eq!(human_bytes(1024 * 1024 * 1024), "1.0 GiB");
    }

    #[test]
    fn parse_bytes_freed_handles_garbage() {
        assert_eq!(parse_bytes_freed("nothing here"), 0);
        assert_eq!(parse_bytes_freed("123.45 MiB freed"), 129446707);
        assert_eq!(parse_bytes_freed("  7.5 KiB freed  \n"), 7680);
    }

    #[test]
    fn cleanup_report_summary_empty() {
        let r = CleanupReport::default();
        assert!(r.is_empty());
        assert_eq!(r.summary(), "nothing to clean");
    }

    #[test]
    fn cleanup_report_summary_includes_counts() {
        let r = CleanupReport {
            gc_runs: 1,
            gc_bytes_freed: 1024 * 1024 * 50,
            hm_baks_removed: 3,
            result_symlinks_removed: 1,
            tmp_files_removed: 2,
            errors: vec![],
        };
        let s = r.summary();
        assert!(s.contains("gc×1"));
        assert!(s.contains("50.0 MiB"));
        assert!(s.contains("3 hm-bak"));
        assert!(s.contains("1 result"));
        assert!(s.contains("2 tmp"));
        assert!(!s.contains("error"));
    }
}
