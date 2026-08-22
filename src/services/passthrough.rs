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

/// Gate table for destructive operations.
///
/// Single source of truth for which (binary, subcommand…) combos require
/// explicit `--confirm` before invocation. Granular — not blanket — per
/// KCR-CLI-3 §10 Q3: uniform gates are friction without benefit.
///
/// Three levels supported via separate tables (kept apart for clarity and
/// future-proofing: a 3-level table can be added without touching the 1/2):
///
/// - `DESTRUCTIVE_OPS_1LEVEL` — (binary, subcmd-or-empty). Matches when
///   `args[0]` equals `subcmd`. Empty `subcmd` means "any subcommand".
/// - `DESTRUCTIVE_OPS_2LEVEL` — (binary, sub1, sub2). Matches when
///   `args[0] == sub1` AND `args[1] == sub2`. Use for nested subcommands
///   like `nix store gc`.
/// - (No 3-level yet; add `DESTRUCTIVE_OPS_3LEVEL` when needed.)
///
/// `requires_confirm(binary, args)` walks the tables and returns `true`
/// for any match. Callers must surface a clear error and exit non-zero
/// when this returns `true` and the user didn't pass `--confirm` / `--yes`
/// / `-y`.
const DESTRUCTIVE_OPS_1LEVEL: &[(&str, &str)] = &[
    // Tier 2.1 — `nixos-install` is destructive by design (writes to /mnt)
    ("nixos-install", ""),
    // Tier 2.4 — chroot into NixOS install is a classic foot-gun
    ("nixos-enter", ""),
    // Tier 1.2 — `boot` changes the default boot entry (recoverable
    // but serious — not a free action).
    ("nixos-rebuild", "boot"),
];

const DESTRUCTIVE_OPS_2LEVEL: &[(&str, &str, &str)] = &[
    // Phase B.5 — `nix store gc` deletes unreachable store paths
    ("nix", "store", "gc"),
    // Phase B.5 — `nix store delete` explicitly deletes paths
    ("nix", "store", "delete"),
    // NOTE: `nix store optimise` and `nix store repair` are read+rewrite
    // but not destructive (no paths deleted). Intentionally NOT gated.
];

/// Return `true` when the (binary, args[0..]) pair is destructive and
/// the user MUST pass `--confirm` / `--yes` / `-y` before we invoke it.
///
/// Walks both 1-level and 2-level tables. Empty `args` is treated as
/// "no subcommand", which only matches 1-level entries with empty
/// `subcmd` (e.g. `nixos-install` with no args is still destructive).
pub fn requires_confirm(binary: &str, args: &[String]) -> bool {
    let first = args.first().map(String::as_str).unwrap_or("");

    // 1-level check
    let hit_1level = DESTRUCTIVE_OPS_1LEVEL
        .iter()
        .any(|(b, s)| *b == binary && (s.is_empty() || *s == first));
    if hit_1level {
        return true;
    }

    // 2-level check
    let second = args.get(1).map(String::as_str).unwrap_or("");
    DESTRUCTIVE_OPS_2LEVEL
        .iter()
        .any(|(b, s1, s2)| *b == binary && *s1 == first && *s2 == second)
}

/// Wrapper around `requires_confirm` that reconstructs the gate-relevant
/// arg vector from `subcommand_label` + the handler's `args` (which already
/// has the subcmd prefix inserted by Phase B handlers).
///
/// Phase A: argv has just user args, no prefix. `subcommand_label` is the
/// 1-level subcmd. So we synthesize `argv = [subcommand_label, ...args]`
/// to match what `requires_confirm` expects.
///
/// Phase B: handlers like `kryx store gc` already prefix argv with "store",
/// so `argv = ["store", "gc", ...]`. We pass `subcommand_label = "store"`
/// but the prefix is already in args[0]. Two cases to handle:
///
/// - If args[0] == subcommand_label: prefix already in argv → use as-is.
/// - If args[0] != subcommand_label: no prefix → synthesize one.
pub fn gate_is_active(binary: &str, subcommand_label: &str, args: &[String]) -> bool {
    let gate_args: Vec<String> = if args.first().map(String::as_str) == Some(subcommand_label) {
        // Args already prefixed (Phase B pattern). Pass through.
        args.to_vec()
    } else {
        // Phase A pattern: synthesize prefix.
        let mut synth = vec![subcommand_label.to_string()];
        synth.extend(args.iter().cloned());
        synth
    };
    requires_confirm(binary, &gate_args)
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
    // KCR-CLI-3-SHELL: support both legacy (`-p git`) and modern
    // (`nixpkgs#git`) muscle memory transparently.
    //
    // Routing: help detection runs FIRST (on the raw args) so
    // `kryx shell --help` shows the kryx help hybrid instead of dumping
    // the full nix shell help. Then we translate and passthrough.
    if args.iter().any(|a| a == "--help" || a == "-h") {
        return run_passthrough_with_help("nix", None, &args, "shell");
    }
    let argv = translate_shell_args(&args);
    run_passthrough(NIX_PATH, &argv, "shell")
}

/// Translate kryx shell args to the modern `nix shell` syntax.
/// Exposed (pub) so tests can exercise the translation table.
pub fn translate_shell_args(args: &[String]) -> Vec<String> {
    if args.is_empty() {
        return vec!["shell".to_string()];
    }

    let mut argv: Vec<String> = vec!["shell".to_string()];
    let mut i = 0;

    // Strategy: walk left-to-right.
    //
    // 1. When we see `-p`/`--packages`, the next arg(s) are package names;
    //    rewrite each to `nixpkgs#<name>` (unless it already has `#` or `/`,
    //    indicating a flake ref). Stop consuming on next flag.
    //
    // 2. Positional (no flag) args BEFORE any nix-shell control flag
    //    (like `--command`, `--run`, `--`) are also package names — rewrite
    //    them the same way. This handles `kryx shell git` (the legacy
    //    `nix-shell git` muscle memory).
    //
    // 3. Once we hit a nix-shell control flag (anything that starts with `-`
    //    OTHER than `-p`/`--packages`), passthrough the rest verbatim.
    //
    // 4. After a literal `--`, all remaining args are passthrough verbatim
    //    (the standard nix convention for "end of nix options").
    while i < args.len() {
        let arg = &args[i];

        // `--` ends nix option parsing; everything after is literal
        // (typically the command to run inside the shell).
        if arg == "--" {
            argv.extend_from_slice(&args[i..]);
            break;
        }

        if arg == "-p" || arg == "--packages" {
            // KCR-CLI-3-SHELL: nix 2.18+ removed `-p`/`--packages` from
            // `nix shell`. We strip the flag and rewrite the package
            // names as positionals (the only form the modern CLI accepts).
            i += 1;
            while i < args.len() && !args[i].starts_with('-') {
                argv.push(rewrite_pkg(&args[i]));
                i += 1;
            }
        } else if !arg.starts_with('-') {
            // Positional package name. Rewrite it.
            argv.push(rewrite_pkg(arg));
            i += 1;
        } else {
            // Some other nix-shell control flag (--command, --run, -i, etc.).
            // Passthrough verbatim, then check if the NEXT arg is a value
            // for this flag. We treat it as a value (passthrough, no
            // package rewrite) unless it starts with `-` (i.e. it's
            // another flag) or is the last arg.
            argv.push(arg.clone());
            i += 1;
            // Heuristic: if next arg exists and doesn't start with `-`,
            // it's likely the flag's value (e.g. `--command "echo hi"`).
            // We passthrough it verbatim (no package rewrite). This is
            // the standard nix convention.
            if i < args.len() && !args[i].starts_with('-') {
                argv.push(args[i].clone());
                i += 1;
            }
        }
    }

    argv
}

/// Rewrite a single package name to `nixpkgs#<name>` unless it already
/// looks like a flake ref (contains `#` or `/`).
fn rewrite_pkg(pkg: &str) -> String {
    if pkg.contains('#') || pkg.contains('/') {
        pkg.to_string()
    } else {
        format!("nixpkgs#{}", pkg)
    }
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

// Os 5 handlers Phase A (`gc`, `home_manager`, `copy_closure`, `nix_env`,
// `nix-channel`) foram migrados pra `run_passthrough_with_help` na seção
// KCR-CLI-3-HELP abaixo. A lógica Phase A (resolução de binary + passthrough)
// é preservada 100% — só adicionamos a detecção de `--help` antes do gate.

// Suppress unused-import warning for `Colorize` until at least one handler
// emits colored output (planned for Phase B with structured errors). Kept
// here so future additions don't need to re-import.
#[allow(dead_code)]
fn _keep_colorize_import() {
    let _ = "[INFO]".cyan();
}

// ── KCR-CLI-3-HELP: Help híbrido (Opção C) ──────────────────────────────
// Adicionado após Phase A. Estrutura `KryxHelp` permite definir a seção
// kryx do help de forma ergonômica, sem repetir boilerplate. A função
// `help_text` concatena a seção kryx com o output real do `binary --help`.
//
// Estratégia: 5 handlers da Phase A agora delegam para
// `run_passthrough_with_help`, que detecta `--help`/`-h` antes do gate e
// renderiza o help híbrido. Para qualquer outro arg, chama o
// `run_passthrough` original (Phase A 100% preservado).

use std::fmt;

/// Builder ergonômico pra definir a seção kryx do help de um catch-all.
///
/// Exemplo de uso:
/// ```ignore
/// let help = KryxHelp::new("gc", "Garbage collection wrapper (nix-collect-garbage)")
///     .option("--confirm, --yes, -y", "Bypass destructive-operation gate (no-op for `gc`)")
///     .option("--dry-run", "Print the command, don't run")
///     .example("kryx gc --delete-older-than 30d")
///     .example("kryx gc --max-freed 10G --dry-run");
/// ```
#[derive(Debug, Clone)]
pub struct KryxHelp {
    pub command: String,
    pub description: String,
    pub version: String,
    pub kryx_options: Vec<(String, String)>, // (flag, description)
    pub examples: Vec<String>,
    pub notes: Vec<String>,
}

impl KryxHelp {
    pub fn new(command: &str, description: &str) -> Self {
        Self {
            command: command.to_string(),
            description: description.to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            kryx_options: Vec::new(),
            examples: Vec::new(),
            notes: Vec::new(),
        }
    }

    pub fn option(mut self, flag: &str, desc: &str) -> Self {
        self.kryx_options.push((flag.to_string(), desc.to_string()));
        self
    }

    pub fn example(mut self, example: &str) -> Self {
        self.examples.push(example.to_string());
        self
    }

    pub fn note(mut self, note: &str) -> Self {
        self.notes.push(note.to_string());
        self
    }
}

impl fmt::Display for KryxHelp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "kryx-{} ({}) — {}",
            self.command, self.version, self.description
        )?;
        writeln!(f)?;
        writeln!(f, "USAGE:")?;
        writeln!(
            f,
            "    kryx {} [KRYX_OPTIONS] [-- NATIVE_ARGS...]",
            self.command
        )?;
        writeln!(f)?;
        writeln!(f, "KRYX OPTIONS:")?;
        for (flag, desc) in &self.kryx_options {
            writeln!(f, "    {:<32}{}", flag, desc)?;
        }
        writeln!(f)?;
        writeln!(f, "NATIVE PASSTHROUGH:")?;
        writeln!(
            f,
            "    All other arguments are forwarded verbatim to the native binary."
        )?;
        writeln!(
            f,
            "    Use `--` to separate kryx options from native args if needed."
        )?;
        if !self.notes.is_empty() {
            writeln!(f)?;
            for note in &self.notes {
                writeln!(f, "    {}", note)?;
            }
        }
        if !self.examples.is_empty() {
            writeln!(f)?;
            writeln!(f, "EXAMPLES:")?;
            for ex in &self.examples {
                writeln!(f, "    {}", ex)?;
            }
        }
        Ok(())
    }
}

// ── Help constants (lazy_static-equivalente sem dep) ─────────────────────
// Usar `std::sync::OnceLock` (Rust 1.70+, estável). Zero deps novas.
//
// `help_<cmd>()` retorna `&'static KryxHelp` resolvido na primeira call e
// cached pra sempre. Phase B/C catch-alls vão seguir o mesmo template.

use std::sync::OnceLock;

static KRYX_HELP_GC: OnceLock<KryxHelp> = OnceLock::new();
fn help_gc() -> &'static KryxHelp {
    KRYX_HELP_GC.get_or_init(|| {
        KryxHelp::new("gc", "Garbage collection wrapper (nix-collect-garbage)")
            .option(
                "--confirm, --yes, -y",
                "Bypass destructive-operation gate (no-op for `gc` — never requires confirm)",
            )
            .option("--dry-run", "Print the command, don't run")
            .option("--verbose", "Log the command to stderr before executing")
            .option("--explain", "Show resolved binary + full argv, then exit")
            .example("kryx gc --delete-older-than 30d")
            .example("kryx gc --max-freed 10G --dry-run")
            .example("kryx gc --explain")
            .note("Use `kryx gc -- --help` to show ONLY the native nix-collect-garbage help.")
    })
}

static KRYX_HELP_HOME_MANAGER: OnceLock<KryxHelp> = OnceLock::new();
fn help_home_manager() -> &'static KryxHelp {
    KRYX_HELP_HOME_MANAGER.get_or_init(|| {
        KryxHelp::new(
            "home-manager",
            "Wrapper for home-manager (user-level config manager)",
        )
        .option(
            "--confirm, --yes, -y",
            "Bypass destructive-operation gate (no-op for `home-manager`)",
        )
        .option("--dry-run", "Print the command, don't run")
        .option("--verbose", "Log the command to stderr")
        .option("--explain", "Show resolved binary + full argv, then exit")
        .example("kryx home-manager switch")
        .example("kryx home-manager build")
        .example("kryx home-manager generations")
        .note(
            "home-manager must be installed (e.g., via `nix profile install nixpkgs#home-manager`).",
        )
    })
}

static KRYX_HELP_COPY_CLOSURE: OnceLock<KryxHelp> = OnceLock::new();
fn help_copy_closure() -> &'static KryxHelp {
    KRYX_HELP_COPY_CLOSURE.get_or_init(|| {
        KryxHelp::new(
            "copy-closure",
            "Copy store closure between machines (nix-copy-closure)",
        )
        .option(
            "--confirm, --yes, -y",
            "Bypass destructive-operation gate (no-op)",
        )
        .option("--dry-run", "Print the command, don't run")
        .option("--verbose", "Log the command to stderr")
        .option("--explain", "Show resolved binary + full argv, then exit")
        .example("kryx copy-closure --to user@host /nix/store/...-hello")
        .example("kryx copy-closure --from user@host /nix/store/...-hello")
    })
}

static KRYX_HELP_NIX_ENV: OnceLock<KryxHelp> = OnceLock::new();
fn help_nix_env() -> &'static KryxHelp {
    KRYX_HELP_NIX_ENV.get_or_init(|| {
        KryxHelp::new("nix-env", "Legacy user-level package manager (nix-env)")
            .option(
                "--confirm, --yes, -y",
                "Bypass destructive-operation gate (no-op)",
            )
            .option("--dry-run", "Print the command, don't run")
            .option("--verbose", "Log the command to stderr")
            .option("--explain", "Show resolved binary + full argv, then exit")
            .example("kryx nix-env -qa 'ripgrep'")
            .example("kryx nix-env -iA nixpkgs.ripgrep")
            .example("kryx nix-env --list-generations")
            .note("Prefer `kryx profile` (modern) over `kryx nix-env` (legacy).")
    })
}

static KRYX_HELP_NIX_CHANNEL: OnceLock<KryxHelp> = OnceLock::new();
fn help_nix_channel() -> &'static KryxHelp {
    KRYX_HELP_NIX_CHANNEL.get_or_init(|| {
        KryxHelp::new("nix-channel", "Legacy channel manager (nix-channel)")
            .option(
                "--confirm, --yes, -y",
                "Bypass destructive-operation gate (no-op)",
            )
            .option("--dry-run", "Print the command, don't run")
            .option("--verbose", "Log the command to stderr")
            .option("--explain", "Show resolved binary + full argv, then exit")
            .example("kryx nix-channel --list")
            .example("kryx nix-channel --update")
            .example("kryx nix-channel --add nixpkgs https://channels.nixos.org/nixpkgs-unstable")
            .note(
                "Prefer `kryx flake` / `kryx registry` (modern) over `kryx nix-channel` (legacy).",
            )
    })
}

// ── Phase B help constants (KCR-CLI-3-B) ─────────────────────────────────
// Todos wrappers do binário `nix`. Mesmo template KryxHelp da Phase A.
// Padronização: cada help mostra 3-5 examples realistas + nota sobre
// `kryx <cmd> -- --help` para ver SÓ o help nativo.

static KRYX_HELP_EVAL: OnceLock<KryxHelp> = OnceLock::new();
static KRYX_HELP_SHELL: OnceLock<KryxHelp> = OnceLock::new();
fn help_eval() -> &'static KryxHelp {
    KRYX_HELP_EVAL.get_or_init(|| {
        KryxHelp::new(
            "eval",
            "Evaluate a Nix expression and print the result (nix eval)",
        )
        .option(
            "--confirm, --yes, -y",
            "Bypass destructive-operation gate (no-op for `eval`)",
        )
        .option("--dry-run", "Print the command, don't run")
        .option("--verbose", "Log the command to stderr")
        .option("--explain", "Show resolved binary + full argv, then exit")
        .example("kryx eval --impure --expr 'builtins.toString (1 + 1)'")
        .example("kryx eval --impure --json '.#pkgs.hello.name'")
        .example(
            "kryx eval --impure '.#nixosConfigurations.\"my-host\".config.networking.hostName'",
        )
        .note("Use `kryx eval -- --help` to show ONLY the native nix eval help.")
    })
}

fn help_shell() -> &'static KryxHelp {
    KRYX_HELP_SHELL.get_or_init(|| {
        KryxHelp::new(
            "shell",
            "Run a command in an environment with the specified packages (nix shell)",
        )
        .option(
            "--confirm, --yes, -y",
            "Bypass destructive-operation gate (no-op for `shell`)",
        )
        .option("--dry-run", "Print the command, don't run")
        .option("--verbose", "Log the command to stderr")
        .option("--explain", "Show resolved binary + full argv, then exit")
        .option("-p, --packages PKG", "(legacy) Packages to put in the shell environment; nix 2.18+ removed this flag, kryx strips it and converts each PKG to a positional `nixpkgs#<PKG>` installable")
        .option("-c CMD", "Run CMD in the shell environment (nix 2.18+ replacement for --command)")
        .example("kryx shell git -c git --version")
        .example("kryx shell -p git hello -c bash")
        .example("kryx shell nixpkgs#youtube-dl --command youtube-dl --version")
        .note(
            "KCR-CLI-3-SHELL: nix 2.18+ renamed `nix shell` (was `nix-shell`) and dropped the `-p`/`--packages` flag.              kryx transparently translates legacy `-p <pkg>` to modern positional `nixpkgs#<pkg>` installables.              Use `kryx shell -- --help` to show ONLY the native nix shell help.",
        )
    })
}

static KRYX_HELP_FLAKE: OnceLock<KryxHelp> = OnceLock::new();
fn help_flake() -> &'static KryxHelp {
    KRYX_HELP_FLAKE.get_or_init(|| {
        KryxHelp::new("flake", "Manage Nix flakes (nix flake)")
            .option(
                "--confirm, --yes, -y",
                "Bypass destructive-operation gate (no-op for `flake` subcommands)",
            )
            .option("--dry-run", "Print the command, don't run")
            .option("--verbose", "Log the command to stderr")
            .option("--explain", "Show resolved binary + full argv, then exit")
            .example("kryx flake show github:kryonix-dev/kryx")
            .example("kryx flake update --commit-lock-file")
            .example("kryx flake lock --refresh")
            .example("kryx flake metadata .")
            .note("Common subcommands: archive, check, info, init, lock, metadata, new, show, update.")
            .note("Use `kryx flake -- --help` to show ONLY the native nix flake help.")
    })
}

static KRYX_HELP_PATH_INFO: OnceLock<KryxHelp> = OnceLock::new();
fn help_path_info() -> &'static KryxHelp {
    KRYX_HELP_PATH_INFO.get_or_init(|| {
        KryxHelp::new("path-info", "Query info about store paths (nix path-info)")
            .option(
                "--confirm, --yes, -y",
                "Bypass destructive-operation gate (no-op)",
            )
            .option("--dry-run", "Print the command, don't run")
            .option("--verbose", "Log the command to stderr")
            .option("--explain", "Show resolved binary + full argv, then exit")
            .example("kryx path-info /nix/store/...-hello")
            .example("kryx path-info --closure-size /nix/store/...-hello")
            .example("kryx path-info -r /nix/store/...-hello")
            .note("Use `kryx path-info -- --help` to show ONLY the native nix path-info help.")
    })
}

static KRYX_HELP_HASH: OnceLock<KryxHelp> = OnceLock::new();
fn help_hash() -> &'static KryxHelp {
    KRYX_HELP_HASH.get_or_init(|| {
        KryxHelp::new("hash", "Compute cryptographic hashes (nix hash)")
            .option(
                "--confirm, --yes, -y",
                "Bypass destructive-operation gate (no-op)",
            )
            .option("--dry-run", "Print the command, don't run")
            .option("--verbose", "Log the command to stderr")
            .option("--explain", "Show resolved binary + full argv, then exit")
            .example("kryx hash file ./foo.tar.gz")
            .example("kryx hash path /nix/store/...-hello")
            .example("kryx hash base32 ./foo.tar.gz")
            .example("kryx hash base16 --type sha256 ./foo.tar.gz")
            .note("Common subcommands: base16, base32, file, path.")
            .note("Use `kryx hash -- --help` to show ONLY the native nix hash help.")
    })
}

static KRYX_HELP_STORE: OnceLock<KryxHelp> = OnceLock::new();
fn help_store() -> &'static KryxHelp {
    KRYX_HELP_STORE.get_or_init(|| {
        KryxHelp::new("store", "Operate on the Nix store (nix store)")
            .option(
                "--confirm, --yes, -y",
                "Bypass destructive-operation gate (REQUIRED for `store gc` and `store delete`)",
            )
            .option("--dry-run", "Print the command, don't run")
            .option("--verbose", "Log the command to stderr")
            .option("--explain", "Show resolved binary + full argv, then exit")
            .example("kryx store gc --confirm")
            .example("kryx store gc --confirm --max-freed 10G")
            .example("kryx store optimise")
            .example("kryx store repair /nix/store/...-hello")
            .example("kryx store ls /nix/store/...-hello")
            .note("Common subcommands: add-file, add-path, copy, delete, gc, info, ls, optimise, repair, serve, verify.")
            .note("DESTRUCTIVE: `store gc` and `store delete` will fail without --confirm.")
            .note("Use `kryx store -- --help` to show ONLY the native nix store help.")
    })
}

/// Resolve o `KryxHelp` estático de um catch-all Phase A. Retorna `None`
/// pra qualquer binary fora do escopo Phase A (usado por safety guards).
fn help_for_binary(binary_name: &str) -> Option<&'static KryxHelp> {
    match binary_name {
        "nix-collect-garbage" => Some(help_gc()),
        "home-manager" => Some(help_home_manager()),
        "nix-copy-closure" => Some(help_copy_closure()),
        "nix-env" => Some(help_nix_env()),
        "nix-channel" => Some(help_nix_channel()),
        _ => None,
    }
}

/// Resolve o `KryxHelp` estático dos catch-alls Phase B (todos wrappers
/// do binário `nix`). Dispatcha pelo primeiro arg (subcmd do nix):
/// `nix eval`, `nix flake`, `nix path-info`, `nix hash`, `nix store`.
///
/// Retorna `None` se o subcmd não está nos 5 Phase B (nesse caso, o caller
/// deve orientar o usuário a usar `kryx <subcmd> -- --help` pra ver só
/// o help nativo, ou cair em outro handler named).
fn help_for_nix_subcmd(subcmd: &str) -> Option<&'static KryxHelp> {
    match subcmd {
        "eval" => Some(help_eval()),
        "shell" => Some(help_shell()),
        "flake" => Some(help_flake()),
        "path-info" => Some(help_path_info()),
        "hash" => Some(help_hash()),
        "store" => Some(help_store()),
        _ => None,
    }
}
// ── Função principal: renderizar help híbrido ────────────────────────────

const HELP_SEPARATOR: &str =
    "═══════════════════════════════════════════════════════════════════════════════";

/// Renderiza o help híbrido (seção kryx + native help inline).
///
/// Comportamento:
///   - Renderiza `kryx_help` (seção kryx) usando seu `Display` impl
///   - Tenta invocar `binary --help` via `discover_real_bin` + `Command::new(bin).arg("--help").output()`
///   - Se conseguir, concatena com separador visual
///   - Se não conseguir (binary não encontrado), termina com nota explicativa
///   - Retorna a string completa (caller decide se printa em stdout ou stderr)
pub fn help_text(binary: &str, kryx_help: &KryxHelp) -> String {
    let mut output = String::new();
    output.push_str(&kryx_help.to_string());
    output.push('\n');
    output.push_str(HELP_SEPARATOR);
    output.push_str(&format!("\nNATIVE BINARY HELP ({} --help):\n", binary));
    output.push_str(HELP_SEPARATOR);
    output.push('\n');
    output.push('\n');

    match discover_real_bin(binary) {
        Some(bin) => match Command::new(&bin).arg("--help").output() {
            Ok(out) => {
                // Native help geralmente vai pra stdout
                let native = String::from_utf8_lossy(&out.stdout);
                output.push_str(&native);
                // Se native help foi pra stderr (raro), anexa
                if !out.stderr.is_empty() {
                    let stderr = String::from_utf8_lossy(&out.stderr);
                    output.push_str("\n--- stderr from native binary ---\n");
                    output.push_str(&stderr);
                }
            }
            Err(e) => {
                output.push_str(&format!(
                    "(failed to invoke `{} --help`: {})\n",
                    bin.display(),
                    e
                ));
            }
        },
        None => {
            output.push_str(&format!(
                "(binary `{}` not found via `discover_real_bin` — install it or set KRYX_BIN_DIR)\n",
                binary
            ));
        }
    }

    output
}

// ── run_passthrough_with_help: wrapper Phase A/B + detection de --help + gate ─
//
// Pipeline (em ordem, antes de qualquer passthrough real):
//   1. EDGE case: usuário usou `--` raw → passthrough puro, sem help híbrido.
//      Workaround clap documentado em `kryx-cli/AGENTS.md` §HELP.
//   2. `--help` / `-h` em args[0] → renderiza help híbrido (kryx + native inline).
//      Phase B: para binary="nix", dispatch por args[0] (eval/flake/path-info/
//      hash/store) — não há 1:1 entre binário e help.
//   3. GATE: se `requires_confirm(binary, args)` e usuário NÃO passou
//      `--confirm`/`--yes`/`-y` → erro explícito + exit 2. Phase A não tinha
//      gate wired (só o contrato de help); Phase B wire pros 5 catch-alls novos.
//   4. Resolve binário + passthrough nativo.
fn run_passthrough_with_help(
    binary_name: &str,
    binary_path_fallback: Option<&str>,
    args: &[String],
    subcommand_label: &str,
) -> Result<(), String> {
    // 1. EDGE case: `--` raw → passthrough puro
    let user_used_explicit_dashdash = std::env::args().any(|a| a == "--");
    if user_used_explicit_dashdash {
        let resolved = if let Some(bin) = discover_real_bin(binary_name) {
            bin.to_string_lossy().into_owned()
        } else if let Some(fallback) = binary_path_fallback {
            fallback.to_string()
        } else {
            return Err(format!(
                "Falha: binário '{}' não encontrado no PATH nem em /run/current-system/sw/bin.",
                binary_name
            ));
        };
        return run_passthrough(&resolved, args, subcommand_label);
    }

    // 2. Help FIRST (antes do gate, antes da resolução de binary destrutivo).
    //    O clap parse já extraiu `--help` / `-h` se estiver em args[0].
    //    Quando args.len() > 1 E o primeiro arg é `--help`/`-h`, isso
    //    significa que o usuário digitou `kryx <cmd> --help <outras-coisas>`.
    //    Convenção Unix: `--help` em qualquer posição sinaliza intenção de
    //    pedir help. Mas se há outros args, o help híbrido do kryx é
    //    enganador (parece que engoliu os args). Nesse caso, mostramos o
    //    help NATIVO do binary subjacente com os args que o usuário passou,
    //    precedido de um aviso curto. Isso preserva a transparência:
    //    "se você queria rodar de verdade, remova o --help".
    // Detect "show help" intent. Two cases:
    //
    // A) `kryx <cmd> --help`           → args == ["--help"] (catch-all
    //                                      without subcmd prefix, e.g. shell)
    // B) `kryx <cmd> <subcmd> --help`  → args == ["<subcmd>", "--help"]
    //                                      (Phase B catch-alls that prefix
    //                                      the subcmd, e.g. eval)
    //
    // In both cases, the user wants the kryx help hybrid. We extract
    // the nix subcmd from subcommand_label (the source of truth) and
    // look it up in help_for_nix_subcmd.
    // Detect "show help" intent. Two cases:
    //
    // A) `kryx <cmd> --help`           → args == ["--help"] (catch-all
    //                                      without subcmd prefix, e.g. shell)
    // B) `kryx <cmd> <subcmd> --help`  → args == ["<subcmd>", "--help"]
    //                                      (Phase B catch-alls that prefix
    //                                      the subcmd, e.g. eval)
    //
    // We compare the args as &str slices to avoid moving the Strings.
    let args_str: Vec<&str> = args.iter().map(String::as_str).collect();
    let is_help_request = matches!(
        args_str.as_slice(),
        ["--help"] | ["-h"] | [_, "--help"] | [_, "-h"]
    );

    if is_help_request {
        let help = if binary_name == "nix" {
            help_for_nix_subcmd(subcommand_label).ok_or_else(|| {
                format!(
                    "no help defined for `kryx nix {}` — try `kryx nix {} -- --help` for native help",
                    subcommand_label, subcommand_label
                )
            })?
        } else {
            help_for_binary(binary_name)
                .ok_or_else(|| format!("no help defined for binary: {}", binary_name))?
        };
        print!("{}", help_text(binary_name, help));
        std::process::exit(0);
    }

    if is_help_request && args.len() > 1 {
        // `kryx <cmd> --help <outras-coisas>` —help + args.
        // Mostra help nativo do binary subjacente com os args do usuário,
        // precedido de aviso curto. Não consome os outros args silenciosamente.
        eprintln!(
            "{} `kryx {} --help` detectado com args adicionais.              Mostrando help nativo do {} subjacente com os args que você passou.              Se você queria rodar de verdade, remova o `--help`.",
            "[INFO]".cyan(),
            subcommand_label,
            binary_name
        );
        // Resolve o binary e repassa com `--help` em qualquer posição.
        let resolved = if let Some(bin) = discover_real_bin(binary_name) {
            bin.to_string_lossy().into_owned()
        } else if let Some(fallback) = binary_path_fallback {
            fallback.to_string()
        } else {
            return Err(format!(
                "Falha: binário '{}' não encontrado no PATH nem em /run/current-system/sw/bin.",
                binary_name
            ));
        };
        return run_passthrough(&resolved, args, subcommand_label);
    }

    // 3. GATE — destructive operations require explicit --confirm.
    // Phase A não wire (catch-alls não-destrutivos); Phase B wire pra
    // `nix store gc` e `nix store delete` via DESTRUCTIVE_OPS_2LEVEL.
    //
    // IMPORTANTE (Phase B): os handlers prefixam o argv com o subcmd real
    // (ex: `kryx store gc` vira argv = ["store", "gc", ...]). Então o gate
    // precisa olhar argv[0..2] como (subcmd1, subcmd2) — não como args
    // crus do usuário. `subcommand_label` (passado pelo handler) é a
    // source of truth do subcmd 1-level; args[1..] fornece o 2-level.
    if gate_is_active(binary_name, subcommand_label, args) {
        let confirmed = args
            .iter()
            .any(|a| a == "--confirm" || a == "--yes" || a == "-y");
        if !confirmed {
            // Mensagem amigável: mostra o que o usuário digitou em kryx
            let user_invocation = format!(
                "kryx {} {}",
                subcommand_label,
                args.first().map(String::as_str).unwrap_or("")
            );
            return Err(format!(
                "{} é destrutivo e exige --confirm explícito (ou --yes / -y). \
                 Use --dry-run pra simular sem executar. Bypass intencional via --confirm.",
                user_invocation
            ));
        }
    }

    // 4. Resolve o binário + passthrough nativo.
    let resolved = if let Some(bin) = discover_real_bin(binary_name) {
        bin.to_string_lossy().into_owned()
    } else if let Some(fallback) = binary_path_fallback {
        fallback.to_string()
    } else {
        return Err(format!(
            "Falha: binário '{}' não encontrado no PATH nem em /run/current-system/sw/bin.",
            binary_name
        ));
    };

    run_passthrough(&resolved, args, subcommand_label)
}

// ── Phase A handlers (KCR-CLI-3) ─────────────────────────────────────────
//
// Cada handler delega para `run_passthrough_with_help` com (binary_name,
// binary_path_fallback=None, args, subcommand_label). Phase A não usa gate
// (catch-alls não-destrutivos: gc/home-manager/copy-closure/nix-env/nix-channel).
// Phase B wire novos handlers na próxima seção.

/// Phase A.1 — `kryx gc [args…]` → `nix-collect-garbage [args…]`
pub fn gc(args: Vec<String>) -> Result<(), String> {
    run_passthrough_with_help("nix-collect-garbage", None, &args, "gc")
}

/// Phase A.2 — `kryx home-manager [args…]` → `home-manager [args…]`
pub fn home_manager(args: Vec<String>) -> Result<(), String> {
    run_passthrough_with_help("home-manager", None, &args, "home-manager")
}

/// Phase A.3 — `kryx copy-closure [args…]` → `nix-copy-closure [args…]`
pub fn copy_closure(args: Vec<String>) -> Result<(), String> {
    run_passthrough_with_help("nix-copy-closure", None, &args, "copy-closure")
}

/// Phase A.4 — `kryx nix-env [args…]` → `nix-env [args…]`
pub fn nix_env(args: Vec<String>) -> Result<(), String> {
    run_passthrough_with_help("nix-env", None, &args, "nix-env")
}

/// Phase A.5 — `kryx nix-channel [args…]` → `nix-channel [args…]`
pub fn nix_channel(args: Vec<String>) -> Result<(), String> {
    run_passthrough_with_help("nix-channel", None, &args, "nix-channel")
}

// ── Phase B handlers (KCR-CLI-3-B) ───────────────────────────────────────
//
// Todos wrappers do binário `nix`. O gate (`requires_confirm`) é wired em
// `run_passthrough_with_help` — sem duplicação aqui. `nix store gc` e
// `nix store delete` exigem `--confirm` (ver DESTRUCTIVE_OPS_2LEVEL).

/// Phase B.1 — `kryx eval [args…]` → `nix eval [args…]`
/// Avalia expressão Nix. Read-only — sem gate.
pub fn eval(args: Vec<String>) -> Result<(), String> {
    let mut argv = vec!["eval".to_string()];
    argv.extend(args);
    run_passthrough_with_help("nix", None, &argv, "eval")
}

/// Phase B.2 — `kryx flake [args…]` → `nix flake [args…]`
/// Manage flakes (show/update/lock/check/init/info/etc). Read-only — sem gate.
pub fn flake(args: Vec<String>) -> Result<(), String> {
    let mut argv = vec!["flake".to_string()];
    argv.extend(args);
    run_passthrough_with_help("nix", None, &argv, "flake")
}

/// Phase B.3 — `kryx path-info [args…]` → `nix path-info [args…]`
/// Query info sobre store paths. Read-only — sem gate.
pub fn path_info(args: Vec<String>) -> Result<(), String> {
    let mut argv = vec!["path-info".to_string()];
    argv.extend(args);
    run_passthrough_with_help("nix", None, &argv, "path-info")
}

/// Phase B.4 — `kryx hash [args…]` → `nix hash [args…]`
/// Compute hashes. Read-only — sem gate.
pub fn hash(args: Vec<String>) -> Result<(), String> {
    let mut argv = vec!["hash".to_string()];
    argv.extend(args);
    run_passthrough_with_help("nix", None, &argv, "hash")
}

/// Phase B.5 — `kryx store [args…]` → `nix store [args…]`
/// Operações no store. Gate wired pra `gc` e `delete` (DESTRUCTIVE_OPS_2LEVEL).
/// `optimise` e `repair` são read+rewrite e NÃO exigem gate.
pub fn store(args: Vec<String>) -> Result<(), String> {
    let mut argv = vec!["store".to_string()];
    argv.extend(args);
    run_passthrough_with_help("nix", None, &argv, "store")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translate_shell_args_empty() {
        let out = translate_shell_args(&[]);
        assert_eq!(out, vec!["shell".to_string()]);
    }

    #[test]
    fn translate_shell_args_legacy_single_pkg() {
        // nix 2.18+ removed `-p`; kryx strips the flag and converts
        // the package name to a positional `nixpkgs#<name>` installable.
        let out = translate_shell_args(&["-p".to_string(), "git".to_string()]);
        assert_eq!(out, vec!["shell".to_string(), "nixpkgs#git".to_string()]);
    }

    #[test]
    fn translate_shell_args_legacy_long_flag() {
        // Same: --packages is also stripped (nix 2.18+ compatibility).
        let out = translate_shell_args(&["--packages".to_string(), "hello".to_string()]);
        assert_eq!(out, vec!["shell".to_string(), "nixpkgs#hello".to_string()]);
    }

    #[test]
    fn translate_shell_args_legacy_multiple_pkgs() {
        // Multiple packages with `-p` all become positional installables.
        let out = translate_shell_args(&["-p".to_string(), "git".to_string(), "hello".to_string()]);
        assert_eq!(
            out,
            vec![
                "shell".to_string(),
                "nixpkgs#git".to_string(),
                "nixpkgs#hello".to_string(),
            ]
        );
    }

    #[test]
    fn translate_shell_args_modern_passthrough() {
        let out = translate_shell_args(&["nixpkgs#git".to_string()]);
        assert_eq!(out, vec!["shell".to_string(), "nixpkgs#git".to_string()]);
    }

    #[test]
    fn translate_shell_args_flake_ref_passthrough() {
        let out = translate_shell_args(&["github:foo/bar".to_string()]);
        assert_eq!(out, vec!["shell".to_string(), "github:foo/bar".to_string()]);
    }

    #[test]
    fn translate_shell_args_legacy_with_command() {
        // Note: nix 2.18+ uses `-c` (not `--command`). This test
        // preserves the user input verbatim — the user is expected to
        // use the modern flag.
        let out = translate_shell_args(&[
            "-p".to_string(),
            "git".to_string(),
            "-c".to_string(),
            "git --version".to_string(),
        ]);
        assert_eq!(
            out,
            vec![
                "shell".to_string(),
                "nixpkgs#git".to_string(),
                "-c".to_string(),
                "git --version".to_string(),
            ]
        );
    }

    #[test]
    fn translate_shell_args_no_pkg_flag_passthrough() {
        // `kryx shell nixpkgs#git --run echo hi` → repassa com
        // o package já qualificado (não toca em `#`)
        let out = translate_shell_args(&[
            "nixpkgs#git".to_string(),
            "--run".to_string(),
            "echo hi".to_string(),
        ]);
        assert_eq!(
            out,
            vec![
                "shell".to_string(),
                "nixpkgs#git".to_string(),
                "--run".to_string(),
                "echo hi".to_string(),
            ]
        );
    }

    #[test]
    fn translate_shell_args_positional_single_pkg() {
        // KCR-CLI-3-SHELL user feedback (Gabriel, 2026-08-21):
        // `kryx shell git` deve resolver para `nix shell nixpkgs#git`,
        // nao `nix shell git` (que o nix 2.35+ rejeita como flake
        // desconhecido "flake:git").
        let out = translate_shell_args(&["git".to_string()]);
        assert_eq!(out, vec!["shell".to_string(), "nixpkgs#git".to_string()]);
    }

    #[test]
    fn translate_shell_args_positional_multiple_pkgs() {
        let out = translate_shell_args(&["git".to_string(), "hello".to_string()]);
        assert_eq!(
            out,
            vec![
                "shell".to_string(),
                "nixpkgs#git".to_string(),
                "nixpkgs#hello".to_string(),
            ]
        );
    }

    #[test]
    fn translate_shell_args_dashdash_terminator() {
        // `--` termina o parsing de opcoes do nix; tudo depois passa verbatim
        let out = translate_shell_args(&[
            "git".to_string(),
            "--".to_string(),
            "bash".to_string(),
            "-c".to_string(),
            "echo hi".to_string(),
        ]);
        assert_eq!(
            out,
            vec![
                "shell".to_string(),
                "nixpkgs#git".to_string(),
                "--".to_string(),
                "bash".to_string(),
                "-c".to_string(),
                "echo hi".to_string(),
            ]
        );
    }

    #[test]
    fn translate_shell_args_dashdash_only() {
        // Apenas `--` (sem pacotes antes) → repassa literal
        let out = translate_shell_args(&[
            "--".to_string(),
            "bash".to_string(),
            "-c".to_string(),
            "echo hi".to_string(),
        ]);
        assert_eq!(
            out,
            vec![
                "shell".to_string(),
                "--".to_string(),
                "bash".to_string(),
                "-c".to_string(),
                "echo hi".to_string(),
            ]
        );
    }

    #[test]
    fn rewrite_pkg_qualified_passthrough() {
        assert_eq!(rewrite_pkg("nixpkgs#git"), "nixpkgs#git");
        assert_eq!(rewrite_pkg("github:foo/bar"), "github:foo/bar");
        assert_eq!(rewrite_pkg("nixos-24.05#hello"), "nixos-24.05#hello");
        assert_eq!(rewrite_pkg("git"), "nixpkgs#git");
        assert_eq!(rewrite_pkg("hello"), "nixpkgs#hello");
    }
}
