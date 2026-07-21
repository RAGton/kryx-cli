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

fn command_output(command: &str, args: &[&str]) -> Result<String, String> {
    let output = Command::new(command)
        .args(args)
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
