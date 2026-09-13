use colored::Colorize;
use std::process::{Command, Stdio};

use crate::services::modules;

/// Marker prepended to every stash message created by `kryx update --force-sync`.
/// Lets `--cleanup-stash` identify and remove only kryx-generated stashes,
/// leaving any user-made stashes untouched. See `cleanup_auto_stashes`.
const STASH_MARKER: &str = "kryx-auto:";

/// Returns true if `repo_path` has working-tree changes OUTSIDE of `flake.lock`.
/// `flake.lock` is the file `kryx update` itself rewrites, so a dirty `flake.lock`
/// after `nix flake update` is the expected steady state — counting it as "dirty"
/// and stashing every run is what caused the 66-stash accumulation and the
/// "switch no-op" bug where Nix reused the cached store path because the
/// working tree was dirty.
fn has_changes_outside_lock(repo_path: &str) -> bool {
    // `git status --porcelain` lists every changed file, one per line. We strip
    // any line whose path resolves to a tracked `flake.lock` (handles both
    // ` M flake.lock` and `M  flake.lock` formats porcelain emits).
    let output = Command::new("git")
        .args(["-C", repo_path, "status", "--porcelain"])
        .output();

    match output {
        Ok(out) if out.status.success() => {
            let s = String::from_utf8_lossy(&out.stdout);
            s.lines().any(|line| {
                // porcelain format: XY <path> (XY are 2 status chars + space)
                let path = line.get(3..).unwrap_or("").trim();
                !path.is_empty() && path != "flake.lock"
            })
        }
        _ => {
            // If we cannot determine, fall back to "yes, dirty" so we don't
            // silently drop work. Safer to over-stash than to lose changes.
            true
        }
    }
}

/// Drop every stash whose message starts with `kryx-auto:`. User-created
/// stashes (no marker) are preserved. Returns the number of stashes removed.
fn cleanup_auto_stashes(repo_path: &str) -> Result<usize, String> {
    let list_output = Command::new("git")
        .args(["-C", repo_path, "stash", "list"])
        .output()
        .map_err(|e| format!("git stash list falhou em {}: {}", repo_path, e))?;

    if !list_output.status.success() {
        return Err(format!(
            "git stash list falhou em {} (exit {:?})",
            repo_path,
            list_output.status.code()
        ));
    }

    let list = String::from_utf8_lossy(&list_output.stdout);
    let mut removed = 0usize;

    // `stash list` prints lines like: "stash@{0}: kryx-auto: WIP on main: ..."
    // We want to drop by index. Iterate from the END so removals don't shift
    // the indices of the ones we haven't processed yet.
    let entries: Vec<(String, String)> = list
        .lines()
        .filter_map(|line| {
            // Format: "stash@{N}: <subject>"
            let (idx, subject) = line.split_once(": ")?;
            Some((idx.to_string(), subject.to_string()))
        })
        .filter(|(_, subject)| subject.starts_with(STASH_MARKER))
        .collect();

    for (idx, _) in entries.iter().rev() {
        let drop = Command::new("git")
            .args(["-C", repo_path, "stash", "drop", idx])
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()
            .map_err(|e| format!("git stash drop {} falhou em {}: {}", idx, repo_path, e))?;

        if drop.success() {
            removed += 1;
        } else {
            eprintln!(
                "{} Não foi possível remover stash {} em {} (exit {:?}); seguindo.",
                "[WARN]".yellow(),
                idx,
                repo_path,
                drop.code()
            );
        }
    }

    Ok(removed)
}

/// Pull a git repository. Three modes, in order of decreasing force:
///   * `--force-sync`: stash EVERYTHING (including flake.lock), pull, warn user
///   * default: stash only NON-LOCK changes; flake.lock is the expected outcome
///     of `nix flake update` and must NOT trigger a stash
///   * `--no-stash`: refuse to touch anything; fail loudly if pull would conflict
fn git_pull_with_flags(
    repo_path: &str,
    ff_only: bool,
    force_sync: bool,
    no_stash: bool,
) -> Result<(), String> {
    let mut args = vec!["-C", repo_path, "pull", "origin", "main"];

    if force_sync {
        // Legacy behaviour: stash -u (including untracked), regardless of what
        // changed. Kept for backwards compatibility.
        let stash_status = Command::new("git")
            .args([
                "-C",
                repo_path,
                "stash",
                "push",
                "-u",
                "-m",
                &format!("{} force-sync", STASH_MARKER),
            ])
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()
            .map_err(|e| format!("git stash push falhou em {}: {}", repo_path, e))?;

        if !stash_status.success() {
            return Err(format!(
                "git stash push falhou em {} (exit {}). Abortando.",
                repo_path, stash_status
            ));
        }

        println!(
            "{} Stash criado em {}. As alterações serão perdidas se o switch falhar.",
            "[WARN]".yellow(),
            repo_path
        );
        args.push("--ff-only");
    } else if no_stash {
        // Refuse to even try if there are non-lock changes. User must commit
        // or `git restore` them first.
        if has_changes_outside_lock(repo_path) {
            return Err(format!(
                "{} tem alterações locais fora de flake.lock e --no-stash foi passado. \
                 Faça commit, git restore, ou rode sem --no-stash.",
                repo_path
            ));
        }
        args.push("--ff-only");
    } else if has_changes_outside_lock(repo_path) {
        // Default mode: only stash if there are REAL changes. Previously this
        // path always stashed on --force-sync; now we treat --force-sync as
        // opt-in and the default behaviour as "stash only what would block".
        // We mark the stash with `kryx-auto:` so `--cleanup-stash` can find it.
        let stash_status = Command::new("git")
            .args([
                "-C",
                repo_path,
                "stash",
                "push",
                "-u",
                "-m",
                &format!("{} local changes before pull", STASH_MARKER),
            ])
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()
            .map_err(|e| format!("git stash push falhou em {}: {}", repo_path, e))?;

        if !stash_status.success() {
            return Err(format!(
                "git stash push falhou em {} (exit {}). Abortando.",
                repo_path, stash_status
            ));
        }

        println!(
            "{} Stash automático criado em {} (marcado {}). Use --cleanup-stash para remover.",
            "[INFO]".cyan(),
            repo_path,
            STASH_MARKER
        );
        args.push("--ff-only");
    } else if ff_only {
        args.push("--ff-only");
    } else {
        args.push("--no-rebase");
    }

    let status = Command::new("git")
        .args(&args)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| format!("Falha ao invocar git em {}: {}", repo_path, e))?;

    if !status.success() {
        if force_sync || no_stash {
            return Err(format!(
                "git pull falhou em {} (exit {}). Suas alterações estão no stash — recupere com: git -C {} stash pop",
                repo_path, status, repo_path
            ));
        } else {
            return Err(format!(
                "git pull falhou em {} (exit {}). Use 'kryx update --force-sync' para fazer stash das alterações locais.",
                repo_path, status
            ));
        }
    }

    Ok(())
}

pub fn run_update(force_sync: bool, no_stash: bool, cleanup_stash: bool) -> Result<(), String> {
    println!(
        "{} Atualizando repositórios e locks de flake...",
        "[INFO]".cyan()
    );

    // Optional pre-cleanup: drop any kryx-auto:* stashes left behind by
    // previous runs. This is the user-facing knob that fixes the
    // "switch reports success but nothing changed" symptom caused by 66+
    // accumulated stashes dirtying the tree.
    if cleanup_stash {
        for repo in &["/etc/kryonix", "/etc/kryonixos"] {
            match cleanup_auto_stashes(repo) {
                Ok(n) if n > 0 => println!(
                    "{} {}: {} stash(es) automático(s) removido(s)",
                    "[INFO]".cyan(),
                    repo,
                    n
                ),
                Ok(_) => println!(
                    "{} {}: nenhum stash automático para limpar",
                    "[INFO]".cyan(),
                    repo
                ),
                Err(e) => eprintln!("{} {}", "[WARN]".yellow(), e),
            }
        }
    }

    // git pull /etc/kryonix
    println!("{} Sincronizando /etc/kryonix...", "[INFO]".cyan());
    git_pull_with_flags("/etc/kryonix", !force_sync, force_sync, no_stash)?;

    // git pull /etc/kryonixos
    println!("{} Sincronizando /etc/kryonixos...", "[INFO]".cyan());
    git_pull_with_flags("/etc/kryonixos", !force_sync, force_sync, no_stash)?;

    // nix flake update --flake /etc/kryonixos
    println!(
        "{} Atualizando locks de flake em /etc/kryonixos...",
        "[INFO]".cyan()
    );

    // Discover the real nix binary under /nix/store; the cli-lockdown module
    // installs shell wrappers at /run/current-system/sw/bin/nix that mask
    // `nix` as "[Kryonix Guard] bloqueado". Searching for the >1 MB binary
    // and prepending its directory to PATH keeps `kryx update` working
    // after lockdown is enabled. Mirrors the pattern in modules::run_switch.
    let real_nix_dir = modules::discover_real_nix_dir().ok_or_else(|| {
        "Could not locate a real nix binary in /nix/store. \
         The Kryonix cli-lockdown may have removed it, which \
         would break `kryx update`. Run the build outside of \
         `kryx` using /run/current-system/sw/bin/nixos-rebuild."
            .to_string()
    })?;
    println!("{} Real nix path: {}", "[INFO]".cyan(), real_nix_dir);

    let sudo_user = std::env::var("SUDO_USER").unwrap_or_else(|_| "rocha".to_string());
    let current_path = std::env::var("PATH").unwrap_or_default();
    let patched_path = format!("{}:{}", real_nix_dir, current_path);

    let mut nix_cmd = Command::new("nix");
    nix_cmd
        .args(["flake", "update", "--flake", "/etc/kryonixos"])
        .env("PATH", patched_path)
        .env("HOME", format!("/home/{}", sudo_user))
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    let status_flake = nix_cmd
        .status()
        .map_err(|e| format!("Falha ao invocar nix flake update: {}", e))?;

    if status_flake.success() {
        println!("{} Atualização concluída com sucesso!", "[PASS]".green());
        Ok(())
    } else {
        Err("Falha ao atualizar flake lock".to_string())
    }
}
