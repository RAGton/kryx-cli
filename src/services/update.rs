use colored::Colorize;
use std::process::{Command, Stdio};

use crate::services::modules;

/// Pull a git repository with optional fast-forward only or force-sync (stash).
fn git_pull_with_flags(repo_path: &str, ff_only: bool, force_sync: bool) -> Result<(), String> {
    let mut args = vec!["-C", repo_path, "pull", "origin", "main"];

    if ff_only {
        args.push("--ff-only");
    } else if force_sync {
        // Stage everything (including untracked) and stash
        let stash_status = Command::new("git")
            .args(["-C", repo_path, "stash", "-u"])
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()
            .map_err(|e| format!("git stash -u falhou em {}: {}", repo_path, e))?;

        if !stash_status.success() {
            return Err(format!(
                "git stash -u falhou em {} (exit {}). Abortando.",
                repo_path, stash_status
            ));
        }

        println!(
            "{} Stash criado em {}. As alterações serão perdidas se o switch falhar.",
            "[WARN]".yellow(),
            repo_path
        );
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
        if force_sync {
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

pub fn run_update(force_sync: bool) -> Result<(), String> {
    println!(
        "{} Atualizando repositórios e locks de flake...",
        "[INFO]".cyan()
    );

    // git pull /etc/kryonix
    println!("{} Sincronizando /etc/kryonix...", "[INFO]".cyan());
    git_pull_with_flags("/etc/kryonix", !force_sync, force_sync)?;

    // git pull /etc/kryonixos
    println!("{} Sincronizando /etc/kryonixos...", "[INFO]".cyan());
    git_pull_with_flags("/etc/kryonixos", !force_sync, force_sync)?;

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
