use colored::Colorize;
use std::fs;
use std::process::Command;

/// Lê `/etc/os-release` e retorna o PRETTY_NAME, ou fallback.
fn read_os_release() -> Option<String> {
    let content = fs::read_to_string("/etc/os-release").ok()?;
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("PRETTY_NAME=") {
            return Some(
                rest.trim_matches('"')
                    .trim_end_matches('\n')
                    .to_string(),
            );
        }
    }
    None
}

/// Lê a identidade do host se existir em /etc/kryonix/identity.json.
fn read_host_identity() -> Option<String> {
    let content = fs::read_to_string("/etc/kryonix/identity.json")
        .or_else(|_| fs::read_to_string("/etc/kryonixos/identity.json"))
        .ok()?;
    // Extrai só os campos que importam — sem JSON parser pra evitar dep extra.
    let mut parts = Vec::new();
    for line in content.lines() {
        if let Some((k, v)) = line.split_once(':') {
            let key = k.trim().trim_matches('"');
            let val = v.trim().trim_matches(',').trim_matches('"');
            if matches!(key, "role" | "edition") {
                parts.push(format!("{}={}", key, val));
            }
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" "))
    }
}

/// Pinta o output do `zpool status` (verde ONLINE, vermelho DEGRADED/FAULTED).
fn render_zpool_status(raw: &str) -> String {
    let mut out = String::new();
    for line in raw.lines() {
        let colored = if line.contains("DEGRADED") || line.contains("FAULTED") {
            line.red().to_string()
        } else if line.contains("ONLINE") {
            line.green().to_string()
        } else {
            line.to_string()
        };
        out.push_str(&colored);
        out.push('\n');
    }
    out
}

pub fn run_status() -> Result<(), String> {
    let header = " ╭─ Kryonix System Dashboard ─╮ ".on_bright_black().white().bold();
    println!("\n{}\n", header);

    // 1. Sistema
    let pretty = read_os_release().unwrap_or_else(|| "Desconhecido".to_string());
    let identity = read_host_identity().unwrap_or_default();
    println!(
        "  {}  {}",
        "OS".cyan().bold(),
        pretty.bold()
    );
    if !identity.is_empty() {
        println!(
            "  {}  {}",
            "Host".cyan().bold(),
            identity.dimmed()
        );
    }

    // 2. ZFS Storage
    println!("\n  {}  Storage (ZFS)", "💾".cyan());
    let zpool_status = Command::new("/run/current-system/sw/bin/zpool")
        .arg("status")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output();

    match zpool_status {
        Ok(out) if out.status.success() => {
            let raw = String::from_utf8_lossy(&out.stdout);
            for line in render_zpool_status(&raw).lines() {
                println!("    {}", line);
            }
        }
        _ => {
            println!(
                "    {}",
                "Não foi possível recuperar status do zpool ou não instalado.".yellow()
            );
        }
    }

    // 3. Incus / Virtualização
    println!("\n  {}  Virtualização (Incus)", "📦".cyan());
    let incus_status = Command::new("/run/current-system/sw/bin/systemctl")
        .args(["is-active", "incus"])
        .output()
        .ok();

    match incus_status {
        Some(o) if String::from_utf8_lossy(&o.stdout).trim() == "active" => {
            println!(
                "    {}  {}",
                "Serviço:".dimmed(),
                "Ativo".green().bold()
            );
        }
        _ => {
            println!(
                "    {}  {}",
                "Serviço:".dimmed(),
                "Inativo ou não instalado".yellow()
            );
        }
    }

    println!();
    Ok(())
}