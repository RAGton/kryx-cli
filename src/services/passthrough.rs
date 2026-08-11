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

// ── run_passthrough_with_help: wrapper Phase A + detection de --help ─────
//
// Esta é a evolução do `run_passthrough` original. Lógica:
//   1. Se args[0] é `--help` ou `-h` → renderiza help híbrido + exit 0
//   2. Caso contrário → chama `run_passthrough` original (Phase A 100% intacto)
//
// NOTA: O gate de operações destrutivas (KCR-CLI-3 §10) será adicionado em
// Phase B sobre esta mesma função. Phase A não usa gate — `--confirm` etc.
// são atualmente no-op na Phase A e ficam só no help como contrato futuro.
fn run_passthrough_with_help(
    binary_name: &str,
    binary_path_fallback: Option<&str>,
    args: &[String],
    subcommand_label: &str,
) -> Result<(), String> {
    // EDGE CASE: clap com `trailing_var_arg + allow_hyphen_values` consome
    // o token `--` internamente (não chega em `args`). Pra honrar convenção
    // Unix (`kryx <cmd> -- --help` = passthrough puro), checamos raw argv.
    // Workaround documentado em `kryx-cli/AGENTS.md` §HELP.
    let user_used_explicit_dashdash = std::env::args().any(|a| a == "--");
    if user_used_explicit_dashdash {
        // Usuário foi explícito: passa TUDO direto pro native, sem help híbrido.
        // Re-resolve o binário (mesma lógica do caminho normal abaixo).
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

    // 1. Help FIRST (antes do gate, antes da resolução de binary destrutivo)
    if let Some(first) = args.first()
        && (first == "--help" || first == "-h")
    {
        let help = help_for_binary(binary_name)
            .ok_or_else(|| format!("no help defined for binary: {}", binary_name))?;
        print!("{}", help_text(binary_name, help));
        std::process::exit(0);
    }

    // 2. Resolve o binário: usa `discover_real_bin` (lockdown-bypass), mas
    // aceita um path absoluto hardcoded como fallback pros wrappers nix/nh
    // (que vivem em /run/current-system/sw/bin e não estão no gate Phase A).
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

// ── Phase A handlers migrados pra run_passthrough_with_help ──────────────
//
// Mudança mínima: cada handler agora chama `run_passthrough_with_help`
// ao invés de `run_passthrough`. O `binary_path_fallback` é `None` porque
// a Phase A usa exclusivamente `discover_real_bin` (lockdown-bypass).

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
