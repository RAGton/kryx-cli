use colored::Colorize;
use std::process::{Command, Stdio};

/// Discover the directory containing a real (non-wrapped) `nix` binary.
fn discover_real_nix_dir() -> Option<String> {
    let entries = std::fs::read_dir("/nix/store").ok()?;
    let mut best: Option<(String, std::time::SystemTime)> = None;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.contains("-nix-2.") {
            continue;
        }
        let bin = entry.path().join("bin/nix");
        let meta = match std::fs::metadata(&bin) {
            Ok(m) => m,
            Err(_) => continue,
        };
        // Lockdown wrappers are ~400 bytes; real nix is several MB.
        if meta.len() < 1_000_000 {
            continue;
        }
        let mtime = meta.modified().ok()?;
        if best.as_ref().map_or(true, |(_, t)| mtime > *t) {
            best = Some((bin.to_string_lossy().to_string(), mtime));
        }
    }
    best.map(|(path, _)| {
        // Return the directory, not the binary path.
        std::path::Path::new(&path)
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or(path)
    })
}

pub fn run_switch(target: Option<String>) -> Result<(), String> {
    println!(
        "{} Iniciando operação atômica de switch...",
        "[INFO]".cyan()
    );

    // 1. Validate if the git tree is clean. The break-glass procedure for
    //    emergencies is to use the canonical absolute path to nixos-rebuild:
    //    sudo /run/current-system/sw/bin/nixos-rebuild switch --flake /etc/kryonixos#<host>
    //    That bypasses the CLI lockdown by design; no in-binary flag should
    //    ever reproduce that behavior, because the kryx binary itself would
    //    be the broken thing needing recovery.
    let git_status = Command::new("git")
        .arg("status")
        .arg("--porcelain")
        .output()
        .map_err(|e| format!("Falha ao executar 'git status': {}", e))?;

    if !git_status.stdout.is_empty() {
        return Err(format!(
            "{}\n{}",
            "A árvore do git não está limpa. Faça commit ou stash das suas alterações antes de executar o switch.",
            String::from_utf8_lossy(&git_status.stdout)
        ));
    }

    // 2. Identify the target hostname
    let hostname = target.unwrap_or_else(|| {
        std::fs::read_to_string("/etc/hostname")
            .unwrap_or_else(|_| "default".to_string())
            .trim()
            .to_string()
    });

    println!(
        "{} Flake target: /etc/kryonixos#{}",
        "[INFO]".cyan(),
        hostname
    );

    // 3. Discover the real nix binary (the cli-lockdown module installs
    //    tiny shell wrappers at /run/current-system/sw/bin/nix that nh
    //    cannot use, because nh internally calls `nix --version`).
    let real_nix_dir = match discover_real_nix_dir() {
        Some(d) => d,
        None => {
            return Err(
                "Could not locate a real nix binary in /nix/store. \
                 The Kryonix cli-lockdown may have removed it, which \
                 would break nh. Run the build outside of kryx switch \
                 using /run/current-system/sw/bin/nixos-rebuild."
                    .to_string(),
            );
        }
    };

    println!("{} Real nix path: {}", "[INFO]".cyan(), real_nix_dir);

    // 4. Run nh os switch via ABSOLUTE PATH so the call survives any
    //    future $PATH poisoning or wrapper substitution (e.g. cli-lockdown).
    //    We prepend the real nix binary to PATH so nh can find it.
    let nh_path = "/run/current-system/sw/bin/nh";
    println!("{} Executando nh os switch...", "[INFO]".cyan());

    let current_path = std::env::var("PATH").unwrap_or_default();
    let patched_path = format!("{}:{}", real_nix_dir, current_path);

    // kryx runs as root (via sudo). The nh binary refuses to run as root
    // and re-drops privileges itself — but only if invoked as a non-root user.
    // We use `sudo -u <user>` to re-drop, then `env` to inject our vars
    // (sudo's env_reset would otherwise strip GIT_CONFIG_*).
    // The nh binary reads SUDO_USER and escalates via the elevation-strategy
    // for the parts that need root (bootloader activation).
    let sudo_user = std::env::var("SUDO_USER").unwrap_or_else(|_| "rocha".to_string());
    let mut cmd = Command::new("/run/wrappers/bin/sudo");
    cmd.arg("-u")
        .arg(&sudo_user)
        .arg("env")
        .arg(format!("HOME=/home/{}", sudo_user))
        .arg(format!("PATH={}", patched_path))
        .arg("GIT_CONFIG_GLOBAL=/dev/null")
        .arg("GIT_CONFIG_SYSTEM=/dev/null")
        .arg("GIT_CONFIG_NOSYSTEM=1")
        .arg(nh_path)
        .arg("os")
        .arg("switch")
        .arg("--elevation-strategy")
        .arg("/run/wrappers/bin/sudo")
        .arg(format!("/etc/kryonixos#{}", hostname));

    cmd.stdout(Stdio::inherit())
        .stderr(Stdio::inherit());

    let status = cmd
        .status()
        .map_err(|e| format!("Falha ao invocar '{}': {}", nh_path, e))?;

    if status.success() {
        println!(
            "{} Switch do sistema concluído com sucesso!",
            "[PASS]".green()
        );
        Ok(())
    } else {
        Err(format!(
            "nh os switch abortado ou falhou com status: {}",
            status
        ))
    }
}