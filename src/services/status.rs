use colored::Colorize;
use std::fs;
use std::process::Command;

pub fn run_status() -> Result<(), String> {
    println!("{} === Status do Sistema Kryonix ===", "[INFO]".cyan());

    // 1. Versão do Kryonix
    let version = fs::read_to_string("/etc/kryonix/version")
        .unwrap_or_else(|_| "Desconhecida".to_string())
        .trim()
        .to_string();
    println!("Versão do Kryonix: {}", version.bold());

    // 2. Status do Zpool
    println!("\n{} Status do ZFS:", "[INFO]".cyan());
    let zpool_status = Command::new("zpool")
        .arg("status")
        .output()
        .map_err(|e| format!("Falha ao invocar zpool: {}", e))?;

    if zpool_status.status.success() {
        println!("{}", String::from_utf8_lossy(&zpool_status.stdout));
    } else {
        println!(
            "{}",
            "Não foi possível recuperar status do zpool ou não existe.".yellow()
        );
    }

    // 3. Status do Incus/KVE
    println!("\n{} Status do Incus (KVE):", "[INFO]".cyan());
    let incus_status = Command::new("systemctl")
        .args(["is-active", "incus"])
        .output()
        .map_err(|e| format!("Falha ao checar systemctl: {}", e))?;

    let incus_output = String::from_utf8_lossy(&incus_status.stdout)
        .trim()
        .to_string();
    if incus_output == "active" {
        println!("Serviço Incus: {}", "Ativo".green());
    } else {
        println!("Serviço Incus: {}", "Inativo ou Não Instalado".yellow());
    }

    Ok(())
}
