use std::process::{Command, Stdio};

/// Canonical absolute paths for the native binaries the pass-through wrappers
/// invoke. Using absolute paths guarantees these calls survive any `$PATH`
/// poisoning applied by the Kryonix Guard cli-lockdown module — even after
/// lockdown is enabled, the kryx binary keeps working because it never goes
/// through the user-visible wrappers.
const NIX_PATH: &str = "/run/current-system/sw/bin/nix";
const NH_PATH: &str = "/run/current-system/sw/bin/nh";

/// Spawn a native binary with the given args, inheriting stdio so the rich
/// terminal output of `nh` (diffs, progress bars, etc.) is preserved.
/// Returns an error string when the binary itself cannot be launched
/// (e.g. missing from the store) — command non-zero exits are propagated
/// to the parent process via `exit` and surface normally.
pub fn run_passthrough(binary_path: &str, args: &[String], subcommand_label: &str) -> Result<(), String> {
    let status = Command::new(binary_path)
        .args(args)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| {
            format!(
                "Falha ao invocar '{}' para 'kryx {}': {}",
                binary_path, subcommand_label, e
            )
        })?;

    if status.success() {
        Ok(())
    } else {
        // Mirror the child exit code so scripts and pipelines behave naturally.
        std::process::exit(status.code().unwrap_or(1));
    }
}

// ---- nix pass-through wrappers ----

pub fn shell(args: Vec<String>) -> Result<(), String> {
    let mut argv = vec!["shell".to_string()];
    argv.extend(args);
    run_passthrough(NIX_PATH, &argv, "shell")
}

pub fn build(args: Vec<String>) -> Result<(), String> {
    let mut argv = vec!["build".to_string()];
    argv.extend(args);
    run_passthrough(NIX_PATH, &argv, "build")
}

pub fn run(args: Vec<String>) -> Result<(), String> {
    let mut argv = vec!["run".to_string()];
    argv.extend(args);
    run_passthrough(NIX_PATH, &argv, "run")
}

pub fn develop(args: Vec<String>) -> Result<(), String> {
    let mut argv = vec!["develop".to_string()];
    argv.extend(args);
    run_passthrough(NIX_PATH, &argv, "develop")
}

pub fn repl(args: Vec<String>) -> Result<(), String> {
    let mut argv = vec!["repl".to_string()];
    argv.extend(args);
    run_passthrough(NIX_PATH, &argv, "repl")
}

pub fn fmt(args: Vec<String>) -> Result<(), String> {
    let mut argv = vec!["fmt".to_string()];
    argv.extend(args);
    run_passthrough(NIX_PATH, &argv, "fmt")
}

// ---- nh pass-through wrappers ----

pub fn search(args: Vec<String>) -> Result<(), String> {
    let mut argv = vec!["search".to_string()];
    argv.extend(args);
    run_passthrough(NH_PATH, &argv, "search")
}

pub fn clean(args: Vec<String>) -> Result<(), String> {
    let mut argv = vec!["clean".to_string()];
    argv.extend(args);
    run_passthrough(NH_PATH, &argv, "clean")
}