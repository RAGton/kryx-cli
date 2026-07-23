use colored::Colorize;
use std::process::{Command, Stdio};

pub fn run_switch(target: Option<String>, bypass_lock: bool) -> Result<(), String> {
    println!(
        "{} Iniciando operação atômica de switch...",
        "[INFO]".cyan()
    );

    // 1. Validate if the git tree is clean (skip only if bypass_lock)
    if !bypass_lock {
        let git_status = Command::new("git")
            .arg("status")
            .arg("--porcelain")
            .output()
            .map_err(|e| format!("Falha ao executar 'git status': {}", e))?;

        if !git_status.stdout.is_empty() {
            return Err(format!(
                "{}\n{}",
                "A árvore do git não está limpa. Faça commit ou stash das suas alterações antes de executar o switch.\n\
                 Para emergências, use: kryx switch --bypass-lock",
                String::from_utf8_lossy(&git_status.stdout)
            ));
        }
    } else {
        println!(
            "{} [BREAK-GLASS] Verificação de árvore git suja ignorada.",
            "[WARN]".yellow()
        );
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

    // 3. Run nh os switch via ABSOLUTE PATH (break-glass: /run/current-system/sw/bin/nh)
    // This path survives even if the Kryonix ecosystem is broken.
    let nh_path = "/run/current-system/sw/bin/nh";
    println!("{} Executando nh os switch...", "[INFO]".cyan());

    let status = Command::new(nh_path)
        .arg("os")
        .arg("switch")
        .arg(&format!("/etc/kryonixos#{}", hostname))
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
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
