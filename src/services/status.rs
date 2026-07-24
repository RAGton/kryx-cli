use colored::*;
use std::fs;
use std::process::Command;

// ============================================================================
// Helpers de leitura
// ============================================================================

/// Lê `/etc/os-release` e retorna o PRETTY_NAME, ou fallback.
fn read_os_release() -> Option<String> {
    let content = fs::read_to_string("/etc/os-release").ok()?;
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("PRETTY_NAME=") {
            return Some(rest.trim_matches('"').trim_end_matches('\n').to_string());
        }
    }
    None
}

/// Lê a identidade do host se existir em /etc/kryonix/identity.json.
fn read_host_identity() -> Option<String> {
    let content = fs::read_to_string("/etc/kryonix/identity.json")
        .or_else(|_| fs::read_to_string("/etc/kryonixos/identity.json"))
        .ok()?;
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

/// Lê o primeiro valor de /proc que casa com `prefixo ` (ex: "MemTotal").
fn read_proc_mem(prefix: &str) -> Option<u64> {
    let content = fs::read_to_string("/proc/meminfo").ok()?;
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix(prefix) {
            // "MemTotal:       16384000 kB"
            let mut parts = rest.split_whitespace();
            if let Some(num) = parts.next() {
                if let Ok(n) = num.parse::<u64>() {
                    return Some(n);
                }
            }
        }
    }
    None
}

fn read_loadavg() -> Option<String> {
    let content = fs::read_to_string("/proc/loadavg").ok()?;
    let parts: Vec<&str> = content.split_whitespace().collect();
    if parts.len() >= 3 {
        Some(format!("{} {} {} (cores: {})", parts[0], parts[1], parts[2], nproc()))
    } else {
        None
    }
}

fn nproc() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
}

/// Lê `model name` de /proc/cpuinfo.
fn read_cpu_model() -> Option<String> {
    let content = fs::read_to_string("/proc/cpuinfo").ok()?;
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("model name") {
            return Some(rest.trim_start_matches(':').trim().to_string());
        }
    }
    None
}

/// Lê temperatura via `sensors` ou `/sys/class/thermal/thermal_zone*/temp`.
fn read_cpu_temp() -> Option<String> {
    if let Ok(out) = Command::new("/run/current-system/sw/bin/sensors")
        .args(["-A"])
        .output()
    {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout);
            for line in s.lines() {
                if line.contains("°C") && (line.contains("Tctl") || line.contains("Package")) {
                    return Some(line.trim().to_string());
                }
            }
        }
    }
    // Fallback: thermal_zone
    if let Ok(entries) = fs::read_dir("/sys/class/thermal") {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with("thermal_zone") {
                if let Ok(t) = fs::read_to_string(e.path().join("temp")) {
                    if let Ok(milli) = t.trim().parse::<i64>() {
                        return Some(format!("{:.1}°C", milli as f64 / 1000.0));
                    }
                }
            }
        }
    }
    None
}

/// Lista interfaces de rede globais com IPv4.
fn read_network() -> Vec<String> {
    let out = Command::new("/run/current-system/sw/bin/ip")
        .args(["-o", "-4", "addr", "show", "scope", "global"])
        .output();
    let mut lines = Vec::new();
    if let Ok(o) = out {
        if o.status.success() {
            for line in String::from_utf8_lossy(&o.stdout).lines() {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 4 {
                    lines.push(format!("{}={}", parts[1], parts[3]));
                }
            }
        }
    }
    lines
}

/// Lê uso de disco (df -h) para mounts críticos.
fn read_disk_usage() -> Vec<String> {
    let out = Command::new("/run/current-system/sw/bin/df")
        .args(["-h", "--output=source,size,used,avail,pcent,target"])
        .output();
    let mut lines = Vec::new();
    if let Ok(o) = out {
        if o.status.success() {
            for line in String::from_utf8_lossy(&o.stdout).lines().skip(1) {
                // Filtra só mounts de interesse
                if line.contains("/nix") || line.contains("/home")
                    || line.contains("/var") || line.contains("/boot")
                {
                    lines.push(line.to_string());
                }
            }
        }
    }
    lines
}

/// Lista containers Incus.
fn read_incus_containers() -> Option<String> {
    let out = Command::new("/run/current-system/sw/bin/incus")
        .args(["list", "--format=csv", "-c", "n,s,4t"])
        .output();
    match out {
        Ok(o) if o.status.success() => {
            let s = String::from_utf8_lossy(&o.stdout);
            let total = s.lines().count().saturating_sub(1);
            let running = s
                .lines()
                .filter(|l| l.contains("RUNNING") || l.contains("running"))
                .count();
            Some(format!("{}/{} running", running, total))
        }
        _ => None,
    }
}

/// Lista containers Podman.
fn read_podman_containers() -> Option<String> {
    let out = Command::new("/run/current-system/sw/bin/podman")
        .args(["ps", "--format", "{{.ID}} {{.State}}"])
        .output();
    match out {
        Ok(o) if o.status.success() => {
            let s = String::from_utf8_lossy(&o.stdout);
            Some(format!("{} rodando", s.lines().count()))
        }
        _ => None,
    }
}

/// Versão do NixOS em uso.
fn read_nixos_version() -> Option<String> {
    let content = fs::read_to_string("/etc/os-release").ok()?;
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("VERSION=") {
            return Some(rest.trim_matches('"').to_string());
        }
    }
    None
}

/// Kernel atual.
fn read_kernel() -> String {
    fs::read_to_string("/proc/sys/kernel/osrelease")
        .ok()
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "?".to_string())
}

/// Uptime formatado.
fn read_uptime() -> String {
    if let Ok(content) = fs::read_to_string("/proc/uptime") {
        if let Some(secs_str) = content.split_whitespace().next() {
            if let Ok(secs) = secs_str.parse::<u64>() {
                let d = secs / 86400;
                let h = (secs % 86400) / 3600;
                let m = (secs % 3600) / 60;
                if d > 0 {
                    return format!("{}d {}h {}m", d, h, m);
                }
                if h > 0 {
                    return format!("{}h {}m", h, m);
                }
                return format!("{}m", m);
            }
        }
    }
    "?".to_string()
}

/// Status de uma systemd unit (active/inactive/failed/...).
fn read_unit(unit: &str) -> String {
    let out = Command::new("/run/current-system/sw/bin/systemctl")
        .args(["is-active", unit])
        .output();
    match out {
        Ok(o) => {
            let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if s.is_empty() {
                "?".to_string()
            } else {
                s
            }
        }
        Err(_) => "?".to_string(),
    }
}

/// Pinta um status de serviço.
fn paint_service_status(status: &str) -> ColoredString {
    match status {
        "active" => status.green().bold(),
        "inactive" | "dead" => status.dimmed(),
        "failed" => status.red().bold(),
        "activating" | "reloading" => status.yellow(),
        _ => status.normal(),
    }
}

// ============================================================================
// Render
// ============================================================================

pub fn run_status() -> Result<(), String> {
    let header = " ╭─ Kryonix System Dashboard ─╮ "
        .on_bright_black()
        .white()
        .bold();
    println!("\n{}\n", header);

    // ── Identidade ──
    let pretty = read_os_release().unwrap_or_else(|| "Desconhecido".to_string());
    let version = read_nixos_version().unwrap_or_default();
    let identity = read_host_identity().unwrap_or_default();
    let kernel = read_kernel();
    let uptime = read_uptime();

    println!("  {}  {}", "OS".cyan().bold(), pretty.bold());
    if !version.is_empty() {
        println!("  {}  {}", "NixOS".cyan().bold(), version.dimmed());
    }
    if !identity.is_empty() {
        println!("  {}  {}", "Host".cyan().bold(), identity.dimmed());
    }
    println!(
        "  {}  {} · {} · {}",
        "Info".cyan().bold(),
        format!("kernel {}", kernel).dimmed(),
        format!("up {}", uptime).dimmed(),
        format!("nproc {}", nproc()).dimmed()
    );

    // ── CPU ──
    println!("\n  {}  CPU", "🖥️ ".cyan());
    if let Some(model) = read_cpu_model() {
        println!("    {}  {}", "Modelo:".dimmed(), model);
    }
    if let Some(load) = read_loadavg() {
        println!("    {}  {}", "Load:".dimmed(), load);
    }
    if let Some(temp) = read_cpu_temp() {
        println!("    {}  {}", "Temp:".dimmed(), temp);
    }

    // ── Memória ──
    println!("\n  {}  Memória", "🧠".cyan());
    let total_kb = read_proc_mem("MemTotal:").unwrap_or(0);
    let avail_kb = read_proc_mem("MemAvailable:").unwrap_or(0);
    let swap_total = read_proc_mem("SwapTotal:").unwrap_or(0);
    let swap_free = read_proc_mem("SwapFree:").unwrap_or(0);
    if total_kb > 0 {
        let used_kb = total_kb - avail_kb;
        let used_pct = (used_kb as f64 / total_kb as f64) * 100.0;
        let total_gb = total_kb as f64 / 1_048_576.0;
        let used_gb = used_kb as f64 / 1_048_576.0;
        let pct_colored = if used_pct > 90.0 {
            format!("{:.1}%", used_pct).red().bold()
        } else if used_pct > 75.0 {
            format!("{:.1}%", used_pct).yellow()
        } else {
            format!("{:.1}%", used_pct).green()
        };
        println!(
            "    {}  {:.1}G / {:.1}G ({})",
            "RAM:".dimmed(),
            used_gb,
            total_gb,
            pct_colored
        );
    }
    if swap_total > 0 {
        let swap_used = swap_total - swap_free;
        let pct = (swap_used as f64 / swap_total as f64) * 100.0;
        let mb = (swap_total as f64) / 1024.0;
        let used_mb = (swap_used as f64) / 1024.0;
        println!(
            "    {}  {:.0}M / {:.0}M ({:.0}%)",
            "Swap:".dimmed(),
            used_mb,
            mb,
            pct
        );
    }

    // ── Storage ──
    println!("\n  {}  Storage (ZFS)", "💾".cyan());
    let zpool_status = Command::new("/run/current-system/sw/bin/zpool")
        .arg("status")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output();
    match zpool_status {
        Ok(out) if out.status.success() => {
            let raw = String::from_utf8_lossy(&out.stdout);
            // Mostra só o resumo (primeiras 30 linhas)
            for line in render_zpool_status(&raw).lines().take(30) {
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
    // Espaço em disco
    let disks = read_disk_usage();
    if !disks.is_empty() {
        println!("    {}", "── mounts ──".dimmed());
        for d in disks.iter().take(5) {
            println!("    {}", d.dimmed());
        }
    }

    // ── Network ──
    println!("\n  {}  Rede", "🌐".cyan());
    let net = read_network();
    if net.is_empty() {
        println!(
            "    {}",
            "Nenhuma interface global IPv4 encontrada.".yellow()
        );
    } else {
        for n in &net {
            println!("    {}  {}", "iface:".dimmed(), n);
        }
    }
    // Gateway
    if let Ok(o) = Command::new("/run/current-system/sw/bin/ip")
        .args(["route", "show", "default"])
        .output()
    {
        if o.status.success() {
            for line in String::from_utf8_lossy(&o.stdout).lines().take(1) {
                println!("    {}  {}", "gw:".dimmed(), line.dimmed());
            }
        }
    }
    // DNS
    if let Ok(content) = fs::read_to_string("/etc/resolv.conf") {
        let nameservers: Vec<&str> = content
            .lines()
            .filter_map(|l| l.strip_prefix("nameserver "))
            .take(2)
            .collect();
        if !nameservers.is_empty() {
            println!("    {}  {}", "dns:".dimmed(), nameservers.join(" "));
        }
    }

    // ── Virtualização / Containers ──
    println!("\n  {}  Virtualização", "📦".cyan());
    let incus_unit = read_unit("incus");
    println!(
        "    {}  incus.service: {}",
        "Incus:".dimmed(),
        paint_service_status(&incus_unit)
    );
    if incus_unit == "active" {
        if let Some(c) = read_incus_containers() {
            println!("    {}  containers: {}", "".dimmed(), c.green());
        }
    }

    let podman_unit = read_unit("podman");
    if podman_unit != "?" && podman_unit != "inactive" {
        println!(
            "    {}  podman.service: {}",
            "Podman:".dimmed(),
            paint_service_status(&podman_unit)
        );
        if let Some(c) = read_podman_containers() {
            println!("    {}  containers: {}", "".dimmed(), c.green());
        }
    }

    // ── Serviços do ecossistema Kryonix ──
    println!("\n  {}  Serviços Kryonix", "⚙️ ".cyan());
    for unit in &["kryxd.service", "kryx-telemetry.service"] {
        let status = read_unit(unit);
        let name = unit.trim_end_matches(".service");
        if status != "?" {
            println!(
                "    {}  {:<20} {}",
                "•".dimmed(),
                name,
                paint_service_status(&status)
            );
        }
    }

    // ── Security ──
    println!("\n  {}  Security", "🔐".cyan());
    // Lockdown status: checa se wrappers do nix existem em home.packages
    let lockdown = if fs::read_to_string("/home/rocha/.nix-profile/bin/nix")
        .map(|c| c.contains("Kryonix Guard"))
        .unwrap_or(false)
    {
        "ativo (Kryonix Guard v2)".green()
    } else {
        "inativo".yellow()
    };
    println!("    {}  {}", "Lockdown:".dimmed(), lockdown);

    // Sudo setuid
    if let Ok(meta) = fs::metadata("/run/wrappers/bin/sudo") {
        // setuid bit = 0o4000
        use std::os::unix::fs::PermissionsExt;
        let mode = meta.permissions().mode();
        if mode & 0o4000 != 0 {
            println!("    {}  {}", "Sudo:".dimmed(), "setuid ok".green());
        } else {
            println!("    {}  {}", "Sudo:".dimmed(), "sem setuid".red());
        }
    }

    println!();
    Ok(())
}
