// MIGRATION STATUS:
// - run_switch: NATIVO (Rust)

use colored::Colorize;
use std::fs;
use std::process::{Command, Stdio};

pub fn run_switch(target: Option<String>) -> Result<(), String> {
    println!(
        "{} Iniciando operação atômica de switch...",
        "[INFO]".cyan()
    );

    // 1. Validate if the git tree is clean
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
        fs::read_to_string("/etc/hostname")
            .unwrap_or_else(|_| "default".to_string())
            .trim()
            .to_string()
    });

    println!(
        "{} Flake target: /etc/kryonixos#{}",
        "[INFO]".cyan(),
        hostname
    );

    // 3. Run nh os switch (replaces nixos-rebuild)
    println!("{} Executando nh os switch...", "[INFO]".cyan());

    let status = Command::new("nh")
        .arg("os")
        .arg("switch")
        .arg(&format!("/etc/kryonixos#{}", hostname))
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| format!("Falha ao invocar 'nixos-rebuild': {}", e))?;

    if status.success() {
        println!(
            "{} Switch do sistema concluído com sucesso!",
            "[PASS]".green()
        );
        Ok(()) // nh os switch succeeded
    } else {
        Err(format!(
            "nh os switch abortado ou falhou com status: {}",
            status
        ))
    }
}
