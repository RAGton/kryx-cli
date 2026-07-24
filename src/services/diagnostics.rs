use crate::domain::identity::HostIdentity;
use crate::services::identity;
use colored::Colorize;
use serde::Serialize;
use std::fs;
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CheckStatus {
    Pass,
    Warn,
    Fail,
}

#[derive(Debug, Clone, Serialize)]
pub struct CheckResult {
    pub category: String,
    pub name: String,
    pub status: CheckStatus,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DoctorSummary {
    pub pass: usize,
    pub warn: usize,
    pub fail: usize,
    pub exit_code: i32,
}

#[derive(Debug, Clone, Serialize)]
pub struct DoctorReport {
    pub host: String,
    pub role: Option<String>,
    pub edition: Option<String>,
    pub checks: Vec<CheckResult>,
    pub summary: DoctorSummary,
}

pub fn run_doctor(json: bool) -> Result<(), String> {
    let report = build_report();

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report)
                .map_err(|e| format!("Falha ao serializar doctor JSON: {}", e))?
        );
    } else {
        render_human(&report);
    }

    if report.summary.fail > 0 {
        Err(format!(
            "Doctor encontrou {} falha(s) crítica(s).",
            report.summary.fail
        ))
    } else {
        Ok(())
    }
}

fn build_report() -> DoctorReport {
    let host = hostname();
    let identity = identity::check_identity();
    let mut checks = Vec::new();

    check_identity(&mut checks, &identity);
    check_system(&mut checks);
    check_git(&mut checks);
    check_zfs(&mut checks, &host);
    check_network(&mut checks);
    check_cpu(&mut checks);
    check_memory(&mut checks);
    check_disk(&mut checks);
    check_services(&mut checks);
    check_security(&mut checks);

    let pass = checks
        .iter()
        .filter(|check| check.status == CheckStatus::Pass)
        .count();
    let warn = checks
        .iter()
        .filter(|check| check.status == CheckStatus::Warn)
        .count();
    let fail = checks
        .iter()
        .filter(|check| check.status == CheckStatus::Fail)
        .count();

    let (role, edition) = identity
        .ok()
        .map(|identity| (Some(format!("{:?}", identity.role)), Some(identity.edition)))
        .unwrap_or((None, None));

    DoctorReport {
        host,
        role,
        edition,
        checks,
        summary: DoctorSummary {
            pass,
            warn,
            fail,
            exit_code: if fail > 0 { 1 } else { 0 },
        },
    }
}

fn render_human(report: &DoctorReport) {
    println!("{}", "Kryonix Doctor".bold().cyan());
    println!(
        "Host: {} · Role: {} · Edition: {}",
        report.host.bold(),
        report.role.as_deref().unwrap_or("unknown"),
        report.edition.as_deref().unwrap_or("unknown")
    );
    println!(
        "{}",
        "────────────────────────────────────────────────────────".dimmed()
    );

    for check in &report.checks {
        let status = match check.status {
            CheckStatus::Pass => "[PASS]".green().bold(),
            CheckStatus::Warn => "[WARN]".yellow().bold(),
            CheckStatus::Fail => "[FAIL]".red().bold(),
        };
        println!(
            "{} {:<10} {:<20} {}",
            status, check.category, check.name, check.message
        );
    }

    println!(
        "{}",
        "────────────────────────────────────────────────────────".dimmed()
    );
    println!(
        "Resumo: {} PASS · {} WARN · {} FAIL",
        report.summary.pass.to_string().green(),
        report.summary.warn.to_string().yellow(),
        report.summary.fail.to_string().red()
    );
}

fn check_identity(checks: &mut Vec<CheckResult>, identity: &Result<HostIdentity, String>) {
    match identity {
        Ok(identity) => push(
            checks,
            "identity",
            "host-identity",
            CheckStatus::Pass,
            format!(
                "uuid={} role={:?} edition={}",
                identity.uuid, identity.role, identity.edition
            ),
        ),
        Err(error) => push(
            checks,
            "identity",
            "host-identity",
            CheckStatus::Fail,
            error.clone(),
        ),
    }
}

fn check_system(checks: &mut Vec<CheckResult>) {
    match fs::read_link("/run/current-system") {
        Ok(path) => push(
            checks,
            "system",
            "generation",
            CheckStatus::Pass,
            path.display().to_string(),
        ),
        Err(error) => push(
            checks,
            "system",
            "generation",
            CheckStatus::Fail,
            format!("não foi possível ler /run/current-system: {}", error),
        ),
    }

    match command_output("systemctl", &["--failed", "--no-legend", "--no-pager"]) {
        Ok(output) if output.trim().is_empty() => push(
            checks,
            "system",
            "failed-units",
            CheckStatus::Pass,
            "nenhuma unit falhada".to_string(),
        ),
        Ok(output) => push(
            checks,
            "system",
            "failed-units",
            CheckStatus::Fail,
            first_lines(&output, 3),
        ),
        Err(error) => push(checks, "system", "failed-units", CheckStatus::Warn, error),
    }

    let user = std::env::var("USER").unwrap_or_else(|_| "rocha".to_string());
    let service = format!("home-manager-{}.service", user);
    check_systemd_unit(checks, "system", &service, true);
}

fn check_git(checks: &mut Vec<CheckResult>) {
    for repo in ["/etc/kryonix", "/etc/kryonixos"] {
        if !Path::new(repo).exists() {
            push(
                checks,
                "git",
                repo,
                CheckStatus::Warn,
                "diretório não existe".to_string(),
            );
            continue;
        }

        let args = [
            "-c",
            &format!("safe.directory={repo}"),
            "-C",
            repo,
            "status",
            "-sb",
        ];
        match command_output("git", &args) {
            Ok(output) => {
                let mut lines = output.lines();
                let branch = lines.next().unwrap_or("status indisponível");
                let dirty: Vec<&str> = lines.collect();
                let mut status = CheckStatus::Pass;
                let mut message = branch.to_string();

                if branch.contains("ahead")
                    || branch.contains("behind")
                    || branch.contains("adelante")
                    || branch.contains("detrás")
                {
                    status = CheckStatus::Warn;
                }

                if !dirty.is_empty() {
                    status = CheckStatus::Warn;
                    message = format!("{}; mudanças locais: {}", message, dirty.join("; "));
                }

                push(checks, "git", repo, status, message);
            }
            Err(error) => push(checks, "git", repo, CheckStatus::Warn, error),
        }
    }
}

fn check_zfs(checks: &mut Vec<CheckResult>, host: &str) {
    let runtime_hostid = command_output("hostid", &[]).map(|value| value.trim().to_string());
    let eval_hostid = command_output(
        "nix",
        &[
            "--extra-experimental-features",
            "nix-command flakes",
            "eval",
            "--impure",
            "--raw",
            &format!(
                "/etc/kryonixos#nixosConfigurations.{}.config.networking.hostId",
                host
            ),
        ],
    )
    .map(|value| value.trim().to_string());

    match (runtime_hostid, eval_hostid) {
        (Ok(runtime), Ok(evaluated)) if runtime == evaluated => push(
            checks,
            "zfs",
            "hostid",
            CheckStatus::Pass,
            format!("runtime/config={runtime}"),
        ),
        (Ok(runtime), Ok(evaluated)) => push(
            checks,
            "zfs",
            "hostid",
            CheckStatus::Fail,
            format!("runtime={runtime} config={evaluated}"),
        ),
        (Err(error), _) | (_, Err(error)) => {
            push(checks, "zfs", "hostid", CheckStatus::Warn, error)
        }
    }

    match command_output("zpool", &["status", "-x"]) {
        Ok(output) if output.contains("all pools are healthy") => push(
            checks,
            "zfs",
            "pools",
            CheckStatus::Pass,
            output.trim().to_string(),
        ),
        Ok(output) if output.contains("no pools available") => push(
            checks,
            "zfs",
            "pools",
            CheckStatus::Warn,
            "nenhum pool ZFS disponível".to_string(),
        ),
        Ok(output) => push(
            checks,
            "zfs",
            "pools",
            CheckStatus::Fail,
            first_lines(&output, 4),
        ),
        Err(error) => push(checks, "zfs", "pools", CheckStatus::Warn, error),
    }

    let trim_enabled = command_output(
        "nix",
        &[
            "--extra-experimental-features",
            "nix-command flakes",
            "eval",
            "--impure",
            "--json",
            &format!(
                "/etc/kryonixos#nixosConfigurations.{}.config.services.zfs.trim.enable",
                host
            ),
        ],
    );
    match trim_enabled.map(|value| value.trim().to_string()) {
        Ok(value) if value == "true" => push(
            checks,
            "zfs",
            "trim",
            CheckStatus::Pass,
            "services.zfs.trim.enable=true".to_string(),
        ),
        Ok(value) => push(
            checks,
            "zfs",
            "trim",
            CheckStatus::Warn,
            format!("services.zfs.trim.enable={value}"),
        ),
        Err(error) => push(checks, "zfs", "trim", CheckStatus::Warn, error),
    }
}

fn check_network(checks: &mut Vec<CheckResult>) {
    match command_output("ip", &["-o", "-4", "addr", "show", "scope", "global"]) {
        Ok(output) if !output.trim().is_empty() => {
            let ips = output
                .lines()
                .filter_map(|line| {
                    let mut parts = line.split_whitespace();
                    let _index = parts.next()?;
                    let interface = parts.next()?;
                    let _family = parts.next()?;
                    let cidr = parts.next()?;
                    Some(format!(
                        "{}={}",
                        interface,
                        cidr.split('/').next().unwrap_or(cidr)
                    ))
                })
                .collect::<Vec<_>>()
                .join(" ");
            push(checks, "network", "ipv4", CheckStatus::Pass, ips);
        }
        Ok(_) => push(
            checks,
            "network",
            "ipv4",
            CheckStatus::Warn,
            "nenhum IPv4 global encontrado".to_string(),
        ),
        Err(error) => push(checks, "network", "ipv4", CheckStatus::Warn, error),
    }

    match command_output("tailscale", &["status", "--self"]) {
        Ok(output) if !output.trim().is_empty() => push(
            checks,
            "network",
            "tailscale",
            CheckStatus::Pass,
            first_lines(&output, 1),
        ),
        Ok(_) => push(
            checks,
            "network",
            "tailscale",
            CheckStatus::Warn,
            "tailscale sem saída para --self".to_string(),
        ),
        Err(error) => push(checks, "network", "tailscale", CheckStatus::Warn, error),
    }

    if fs::read_to_string("/etc/resolv.conf")
        .map(|content| content.lines().any(|line| line.starts_with("nameserver ")))
        .unwrap_or(false)
    {
        push(
            checks,
            "network",
            "dns-config",
            CheckStatus::Pass,
            "/etc/resolv.conf contém nameserver".to_string(),
        );
    } else {
        push(
            checks,
            "network",
            "dns-config",
            CheckStatus::Warn,
            "nenhum nameserver em /etc/resolv.conf".to_string(),
        );
    }
}

// ============================================================================
// NOVOS CHECKS: CPU, MEMORY, DISK, SERVICES, SECURITY
// ============================================================================

fn check_cpu(checks: &mut Vec<CheckResult>) {
    // Load average
    if let Ok(content) = fs::read_to_string("/proc/loadavg") {
        let parts: Vec<&str> = content.split_whitespace().collect();
        if parts.len() >= 3 {
            let load1: f64 = parts[0].parse().unwrap_or(0.0);
            let nproc = std::thread::available_parallelism()
                .map(|n| n.get() as f64)
                .unwrap_or(1.0);
            let ratio = load1 / nproc;
            let (status, msg) = if ratio > 2.0 {
                (CheckStatus::Fail, format!("load1={:.2} ({}x nproc) saturado", load1, ratio))
            } else if ratio > 1.0 {
                (CheckStatus::Warn, format!("load1={:.2} ({}x nproc) alto", load1, ratio))
            } else {
                (CheckStatus::Pass, format!("load1={:.2} ({}x nproc)", load1, ratio))
            };
            push(checks, "cpu", "loadavg", status, msg);
        }
    }

    // Temperatura
    if let Some(c) = read_cpu_temp_celsius() {
        let (status, msg) = if c > 90 {
            (CheckStatus::Fail, format!("CPU {}C critico", c))
        } else if c > 80 {
            (CheckStatus::Warn, format!("CPU {}C alto", c))
        } else {
            (CheckStatus::Pass, format!("CPU {}C", c))
        };
        push(checks, "cpu", "temperature", status, msg);
    }
}

fn read_cpu_temp_celsius() -> Option<i64> {
    if let Ok(out) = Command::new("/run/current-system/sw/bin/sensors").args(["-A"]).output() {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout);
            for line in s.lines() {
                if line.contains("°C") && (line.contains("Tctl") || line.contains("Package")) {
                    if let Some(pos) = line.find('+') {
                        let after = &line[pos+1..];
                        let num_str: String = after.chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
                        if let Ok(c) = num_str.parse::<f64>() {
                            return Some(c as i64);
                        }
                    }
                }
            }
        }
    }
    if let Ok(entries) = fs::read_dir("/sys/class/thermal") {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with("thermal_zone") {
                if let Ok(t) = fs::read_to_string(e.path().join("temp")) {
                    if let Ok(milli) = t.trim().parse::<i64>() {
                        return Some(milli / 1000);
                    }
                }
            }
        }
    }
    None
}

fn check_memory(checks: &mut Vec<CheckResult>) {
    let total_kb = read_proc_mem_kb("MemTotal:").unwrap_or(0);
    let avail_kb = read_proc_mem_kb("MemAvailable:").unwrap_or(0);
    if total_kb == 0 {
        return;
    }
    let used_kb = total_kb - avail_kb;
    let pct = (used_kb as f64 / total_kb as f64) * 100.0;
    let total_gb = total_kb as f64 / 1_048_576.0;
    let used_gb = used_kb as f64 / 1_048_576.0;
    let (status, msg) = if pct > 95.0 {
        (CheckStatus::Fail, format!("{:.1}G/{:.1}G ({:.0}%) critico", used_gb, total_gb, pct))
    } else if pct > 85.0 {
        (CheckStatus::Warn, format!("{:.1}G/{:.1}G ({:.0}%) alto", used_gb, total_gb, pct))
    } else {
        (CheckStatus::Pass, format!("{:.1}G/{:.1}G ({:.0}%)", used_gb, total_gb, pct))
    };
    push(checks, "memory", "ram", status, msg);

    // Swap
    let swap_total = read_proc_mem_kb("SwapTotal:").unwrap_or(0);
    let swap_free = read_proc_mem_kb("SwapFree:").unwrap_or(0);
    if swap_total > 0 {
        let swap_used = swap_total - swap_free;
        let swap_pct = (swap_used as f64 / swap_total as f64) * 100.0;
        let used_mb = swap_used as f64 / 1024.0;
        let total_mb = swap_total as f64 / 1024.0;
        let (status, msg) = if swap_pct > 50.0 {
            (CheckStatus::Warn, format!("{:.0}M/{:.0}M ({:.0}%) pressao", used_mb, total_mb, swap_pct))
        } else {
            (CheckStatus::Pass, format!("{:.0}M/{:.0}M ({:.0}%)", used_mb, total_mb, swap_pct))
        };
        push(checks, "memory", "swap", status, msg);
    }
}

fn read_proc_mem_kb(prefix: &str) -> Option<u64> {
    let content = fs::read_to_string("/proc/meminfo").ok()?;
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix(prefix) {
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

fn check_disk(checks: &mut Vec<CheckResult>) {
    let out = Command::new("/run/current-system/sw/bin/df")
        .args(["-P"])
        .output();
    let Some(out) = out.ok() else { return; };
    if !out.status.success() {
        return;
    }
    let content = String::from_utf8_lossy(&out.stdout);
    let mut critical: Vec<String> = Vec::new();
    let mut high: Vec<String> = Vec::new();
    for line in content.lines().skip(1) {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 6 {
            continue;
        }
        let mount = parts[5];
        if !matches!(mount, "/" | "/nix" | "/home" | "/var" | "/var/log" | "/boot") {
            continue;
        }
        let cap_str = parts[4].trim_end_matches('%');
        if let Ok(cap) = cap_str.parse::<i64>() {
            if cap >= 95 {
                critical.push(format!("{}={}%", mount, cap));
            } else if cap >= 85 {
                high.push(format!("{}={}%", mount, cap));
            }
        }
    }
    if !critical.is_empty() {
        push(checks, "disk", "space", CheckStatus::Fail, format!("critico: {}", critical.join(", ")));
    } else if !high.is_empty() {
        push(checks, "disk", "space", CheckStatus::Warn, format!("alto: {}", high.join(", ")));
    } else {
        push(checks, "disk", "space", CheckStatus::Pass, "mounts criticos ok".to_string());
    }
}

fn check_services(checks: &mut Vec<CheckResult>) {
    let critical_units = &[
        ("kryxd.service", false),
        ("NetworkManager.service", false),
        ("sshd.service", true),
    ];
    for (unit, warn_if_missing) in critical_units {
        check_systemd_unit(checks, "services", unit, *warn_if_missing);
    }
}

fn check_security(checks: &mut Vec<CheckResult>) {
    // Lockdown v2 (Kryonix Guard)
    let lockdown_active = fs::read_to_string("/home/rocha/.nix-profile/bin/nix")
        .map(|c| c.contains("Kryonix Guard"))
        .unwrap_or(false);
    if lockdown_active {
        push(checks, "security", "lockdown", CheckStatus::Pass,
             "Kryonix Guard v2 ativo em home.packages".to_string());
    } else {
        push(checks, "security", "lockdown", CheckStatus::Warn,
             "Kryonix Guard v2 nao detectado".to_string());
    }

    // sudo setuid
    if let Ok(meta) = fs::metadata("/run/wrappers/bin/sudo") {
        use std::os::unix::fs::PermissionsExt;
        let mode = meta.permissions().mode();
        if mode & 0o4000 != 0 {
            push(checks, "security", "sudo-setuid", CheckStatus::Pass,
                 "setuid bit presente".to_string());
        } else {
            push(checks, "security", "sudo-setuid", CheckStatus::Fail,
                 "setuid bit ausente (nh escalation quebrada)".to_string());
        }
    } else {
        push(checks, "security", "sudo-setuid", CheckStatus::Warn,
             "/run/wrappers/bin/sudo nao encontrado".to_string());
    }
}

fn check_systemd_unit(
    checks: &mut Vec<CheckResult>,
    category: &'static str,
    unit: &str,
    warn_if_missing: bool,
) {
    match command_output("systemctl", &["is-active", unit]) {
        Ok(output) if output.trim() == "active" => push(
            checks,
            category,
            unit,
            CheckStatus::Pass,
            "active".to_string(),
        ),
        Ok(output) => push(
            checks,
            category,
            unit,
            if warn_if_missing {
                CheckStatus::Warn
            } else {
                CheckStatus::Fail
            },
            output.trim().to_string(),
        ),
        Err(error) => push(checks, category, unit, CheckStatus::Warn, error),
    }
}

fn hostname() -> String {
    fs::read_to_string("/etc/hostname")
        .map(|value| value.trim().to_string())
        .unwrap_or_else(|_| "unknown".to_string())
}

#[inline(never)]
fn discover_real_nix() -> Option<String> {
    let entries = std::fs::read_dir("/nix/store").ok()?;
    let mut best: Option<String> = None;
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
        // Take the lexicographically last one (most recent hash usually wins).
        let path = bin.to_string_lossy().to_string();
        if best.as_ref().map_or(true, |b| path > *b) {
            best = Some(path);
        }
    }
    best
}

fn command_output(command: &str, args: &[&str]) -> Result<String, String> {
    // Resolve the real nix binary path so we bypass the cli-lockdown
    // wrapper installed at /run/current-system/sw/bin/nix.
    let resolved = if command == "nix" {
        discover_real_nix().unwrap_or_else(|| command.to_string())
    } else {
        command.to_string()
    };

    let output = Command::new(&resolved)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .map_err(|e| format!("falha ao executar {command}: {e}"))?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let detail = if !stderr.is_empty() { stderr } else { stdout };
        Err(format!("{command} saiu com {}: {detail}", output.status))
    }
}

fn first_lines(value: &str, limit: usize) -> String {
    value
        .lines()
        .take(limit)
        .collect::<Vec<_>>()
        .join("; ")
        .trim()
        .to_string()
}

fn push(
    checks: &mut Vec<CheckResult>,
    category: impl Into<String>,
    name: impl Into<String>,
    status: CheckStatus,
    message: String,
) {
    checks.push(CheckResult {
        category: category.into(),
        name: name.into(),
        status,
        message,
    });
}
