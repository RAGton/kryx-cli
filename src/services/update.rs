use colored::Colorize;
use std::process::{Command, Stdio};

pub fn run_update() -> Result<(), String> {
    println!(
        "{} Atualizando repositórios e locks de flake...",
        "[INFO]".cyan()
    );

    // git -C /etc/kryonix pull origin main --no-rebase
    println!("{} Sincronizando /etc/kryonix...", "[INFO]".cyan());
    let status_k = Command::new("git")
        .args([
            "-C",
            "/etc/kryonix",
            "pull",
            "origin",
            "main",
            "--no-rebase",
        ])
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| format!("Falha ao invocar git em /etc/kryonix: {}", e))?;

    if !status_k.success() {
        return Err("Falha ao atualizar /etc/kryonix".to_string());
    }

    // git -C /etc/kryonixos pull origin main --no-rebase
    println!("{} Sincronizando /etc/kryonixos...", "[INFO]".cyan());
    let status_kos = Command::new("git")
        .args([
            "-C",
            "/etc/kryonixos",
            "pull",
            "origin",
            "main",
            "--no-rebase",
        ])
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| format!("Falha ao invocar git em /etc/kryonixos: {}", e))?;

    if !status_kos.success() {
        return Err("Falha ao atualizar /etc/kryonixos".to_string());
    }

    // nix flake update --flake /etc/kryonixos
    println!(
        "{} Atualizando locks de flake em /etc/kryonixos...",
        "[INFO]".cyan()
    );
    let status_flake = Command::new("nix")
        .args(["flake", "update", "--flake", "/etc/kryonixos"])
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| format!("Falha ao invocar nix flake update: {}", e))?;

    if status_flake.success() {
        println!("{} Atualização concluída com sucesso!", "[PASS]".green());
        Ok(())
    } else {
        Err("Falha ao atualizar flake lock".to_string())
    }
}
