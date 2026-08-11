use colored::Colorize;
use std::path::PathBuf;
use std::process::{Command, Stdio};

/// Canonical absolute paths for the native binaries the pass-through wrappers
/// invoke. Using absolute paths guarantees these calls survive any `$PATH`
/// poisoning applied by the Kryonix Guard cli-lockdown module — even after
/// lockdown is enabled, the kryx binary keeps working because it never goes
/// through the user-visible wrappers.
const NIX_PATH: &str = "/run/current-system/sw/bin/nix";
const NH_PATH: &str = "/run/current-system/sw/bin/nh";

/// Ordered probing locations for `discover_real_bin`. The Kryonix Guard
/// cli-lockdown installs tiny shell wrappers (~400 bytes) that masquerade as
/// `nix`, `nh`, etc. We skip anything suspiciously small and prefer the real
/// Nix-store binary (several MB).
const BIN_PROBE_PATHS: &[&str] = &[
    "/run/current-system/sw/bin",
    "/run/wrappers/bin",
    "/usr/bin",
    "/usr/local/bin",
];

/// Discover the absolute path to a real native binary by name (e.g. "nix",
/// "nh", "nix-collect-garbage", "home-manager"). Probes a fixed list of
/// canonical locations first, then falls back to `$PATH` resolution.
///
/// Returns `None` if no executable matching `name` is found. The kryx
/// cli-lockdown bypass invariant (see `modules::discover_real_nix_dir`)
/// still applies: binaries discovered here are real store-backed binaries,
/// not the tiny cli-lockdown shell wrappers.
pub fn discover_real_bin(name: &str) -> Option<PathBuf> {
    // 1. Fixed probe paths (deterministic, lockdown-bypass)
    for dir in BIN_PROBE_PATHS {
        let candidate = PathBuf::from(dir).join(name);
        if let Ok(meta) = std::fs::metadata(&candidate)
            && meta.is_file()
            && meta.len() >= 1_000
        {
            return Some(candidate);
        }
    }

    // 2. $PATH fallback (last resort)
    if let Ok(path_env) = std::env::var("PATH") {
        for dir in path_env.split(':') {
            let candidate = PathBuf::from(dir).join(name);
            if let Ok(meta) = std::fs::metadata(&candidate)
                && meta.is_file()
                && meta.len() >= 1_000
            {
                return Some(candidate);
            }
        }
    }

    None
}

/// Gate table for destructive (binary, subcommand) pairs.
///
/// Returned boolean: `true` means the handler must require an explicit
/// `--confirm` flag (or `--yes`) before invoking the underlying binary.
/// Non-destructive commands return `false` and proceed transparently.
///
/// This is intentionally granular — not a blanket gate. Rationale (KCR-CLI-3
/// §10 Q3): uniform gates are friction without benefit. Per-pair gates
/// preserve user agency while protecting against foot-guns.
pub fn requires_confirm(binary: &str, subcmd: &str) -> bool {
    matches!(
        (binary, subcmd),
        // Tier 2.1 — `nixos-install` is destructive by design (writes to /mnt)
        ("nixos-install", _) |
        // Tier 2.4 — chroot into NixOS install is a classic foot-gun
        ("nixos-enter", _) |
        // Tier 1.2 — `boot` changes the default boot entry (recoverable
        // but serious — not a free action).
        ("nixos-rebuild", "boot")
    )
}

/// Spawn a native binary with the given args, inheriting stdio so the rich
/// terminal output of `nh` (diffs, progress bars, etc.) is preserved.
/// Returns an error string when the binary itself cannot be launched
/// (e.g. missing from the store) — command non-zero exits are propagated
/// to the parent process via `exit` and surface normally.
pub fn run_passthrough(
    binary_path: &str,
    args: &[String],
    subcommand_label: &str,
) -> Result<(), String> {
    let status = Command::new(binary_path)
        .args(args)
        // Bloqueia leitura de configs globais do git em paths inacessíveis
        // (e.g. /root/.gitconfig quando o kryx roda como root via sudo).
        // Necessário porque subprocessos nativos do nix (nh, nix) usam libgit2
        // que tenta ler configs do HOME, que pode ser /root (inacessível).
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
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

pub fn check(args: Vec<String>) -> Result<(), String> {
    let mut argv = vec![
        "--extra-experimental-features".to_string(),
        "nix-command flakes".to_string(),
        "flake".to_string(),
        "check".to_string(),
        "--keep-going".to_string(),
        "--impure".to_string(),
    ];
    argv.extend(args);
    run_passthrough(NIX_PATH, &argv, "check")
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

// ---- Phase A catch-alls (KCR-CLI-3) ----
// Each handler below resolves its target binary via `discover_real_bin` so
// it survives the Kryonix Guard cli-lockdown even though `name` is not
// hardcoded. Names follow the catch-all proposal from §9 of the KCR.
//
// All handlers pass through args verbatim and inherit stdio. Destructive
// operations (when surfaced in future Phase B/C) consult `requires_confirm`
// before invocation; Phase A only adds non-destructive wrappers, so no
// gate is checked here yet.

/// Phase A.1 — `kryx gc [args…]` → `nix-collect-garbage [args…]`
/// Alias for the legacy `nix-collect-garbage` wrapper, which internally
/// calls `nix store gc --delete-older-than`. Preserves muscle memory of
/// users migrating from `nh`/classic nix-env workflows.
pub fn gc(args: Vec<String>) -> Result<(), String> {
    let bin = discover_real_bin("nix-collect-garbage").ok_or_else(|| {
        "Falha: binário 'nix-collect-garbage' não encontrado no PATH nem em \
         /run/current-system/sw/bin. Verifique se NixOS está instalado \
         e se o cli-lockdown não removeu o pacote legacy."
            .to_string()
    })?;
    run_passthrough(bin.to_string_lossy().as_ref(), &args, "gc")
}

/// Phase A.2 — `kryx home-manager [args…]` → `home-manager [args…]`
/// Thin wrapper around home-manager CLI. Kryonix uses Home Manager
/// extensively (skill `kryonix-dev-repo-workflow` §38), so a first-class
/// catch-all saves users from switching to `nix run nix-community/home-manager`.
pub fn home_manager(args: Vec<String>) -> Result<(), String> {
    let bin = discover_real_bin("home-manager").ok_or_else(|| {
        "Falha: binário 'home-manager' não encontrado. Verifique se o \
         pacote home-manager está instalado no NixOS (/run/current-system/sw/bin \
         ou /nix/store)."
            .to_string()
    })?;
    run_passthrough(bin.to_string_lossy().as_ref(), &args, "home-manager")
}

/// Phase A.3 — `kryx copy-closure [args…]` → `nix-copy-closure [args…]`
/// Critical for deploy workflows (`kryx deploy`, `kryx factory-reset`).
/// Users regularly copy closures between hosts; muscle memory of
/// `nix-copy-closure --to <host>` survives via this wrapper.
pub fn copy_closure(args: Vec<String>) -> Result<(), String> {
    let bin = discover_real_bin("nix-copy-closure").ok_or_else(|| {
        "Falha: binário 'nix-copy-closure' não encontrado. Legacy NixOS \
         tool — se você está em instalação moderna sem ele, use \
         `kryx copy` (Phase B, planejado) ou `nix copy --to <uri>`."
            .to_string()
    })?;
    run_passthrough(bin.to_string_lossy().as_ref(), &args, "copy-closure")
}

/// Phase A.4 — `kryx nix-env [args…]` → `nix-env [args…]`
/// Legacy user-level package manager. Still widely used for ad-hoc
/// installs in `~/.nix-profile` without flakes. The cry for muscle memory
/// preservation: `kryx nix-env -iA nixos.pkgs.firefox` works as expected.
pub fn nix_env(args: Vec<String>) -> Result<(), String> {
    let bin = discover_real_bin("nix-env").ok_or_else(|| {
        "Falha: binário 'nix-env' não encontrado no PATH. Em sistemas \
         puramente flakes, nix-env pode estar desabilitado. Use \
         `nix profile install` ou `kryx search` para equivalentes modernos."
            .to_string()
    })?;
    run_passthrough(bin.to_string_lossy().as_ref(), &args, "nix-env")
}

/// Phase A.5 — `kryx nix-channel [args…]` → `nix-channel [args…]`
/// Legacy channels manager. Most useful for `nix-channel --rollback` after
/// a bad channel update. Pure pass-through.
pub fn nix_channel(args: Vec<String>) -> Result<(), String> {
    let bin = discover_real_bin("nix-channel").ok_or_else(|| {
        "Falha: binário 'nix-channel' não encontrado. Channels estão \
         deprecados em NixOS com flakes — considere migrar para \
         `nix flake update` + `kryx switch`."
            .to_string()
    })?;
    run_passthrough(bin.to_string_lossy().as_ref(), &args, "nix-channel")
}

// Suppress unused-import warning for `Colorize` until at least one handler
// emits colored output (planned for Phase B with structured errors). Kept
// here so future additions don't need to re-import.
#[allow(dead_code)]
fn _keep_colorize_import() {
    let _ = "[INFO]".cyan();
}
