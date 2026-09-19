mod cli;
use colored::Colorize;
use kryx::services;

use clap::{CommandFactory, FromArgMatches};
use cli::{Cli, Commands};
use std::process::exit;

fn main() {
    let mut cmd = Cli::command();

    let identity_result = kryx::services::identity::check_identity();
    let role = identity_result.as_ref().map(|i| &i.role).ok();

    let is_core = matches!(
        role,
        Some(kryx::domain::identity::Role::Core) | Some(kryx::domain::identity::Role::ThinkServer)
    );
    let is_desktop = matches!(role, Some(kryx::domain::identity::Role::Desktop));

    if is_desktop {
        cmd = cmd
            .mut_subcommand("deploy", |c| c.hide(true))
            .mut_subcommand("node", |c| c.hide(true))
            .mut_subcommand("feature", |c| c.hide(true));
    } else if !is_core && !is_desktop {
        // Zombie mode
        cmd = cmd
            .mut_subcommand("deploy", |c| c.hide(true))
            .mut_subcommand("node", |c| c.hide(true))
            .mut_subcommand("switch", |c| c.hide(true))
            .mut_subcommand("factory-reset", |c| c.hide(true))
            .mut_subcommand("doctor", |c| c.hide(true))
            .mut_subcommand("system", |c| c.hide(true))
            .mut_subcommand("theme", |c| c.hide(true))
            .mut_subcommand("feature", |c| c.hide(true));
    }

    let matches = cmd.get_matches();
    let cli = match Cli::from_arg_matches(&matches) {
        Ok(c) => c,
        Err(e) => {
            e.exit();
        }
    };

    // Print the Kryonix banner at the top of every long-running command
    // (switch/update/status/doctor/gc/clean). Other commands stay quiet
    // so power users don't get banner fatigue. This is opt-out via
    // KRYX_NO_BANNER=1 (e.g. for scripts that pipe kryx output).
    let show_banner = std::env::var("KRYX_NO_BANNER").is_err()
        && matches!(
            &cli.command,
            Commands::Switch { .. }
                | Commands::Update { .. }
                | Commands::Status
                | Commands::Doctor { .. }
                | Commands::Gc { .. }
                | Commands::Clean { .. }
                | Commands::System { .. }
        );
    if show_banner {
        kryx::ui::print_banner();
    }

    // Authorization Hook
    let authorized = match &cli.command {
        Commands::Identity { .. }
        | Commands::Setup
        | Commands::Check { .. }
        | Commands::Switch { .. }
        | Commands::HomeManager { .. }
        | Commands::Status => true,
        Commands::Deploy { .. } | Commands::Node { .. } | Commands::Feature { .. } => is_core,
        _ => is_core || is_desktop,
    };

    if !authorized {
        eprintln!("Erro: Comando desconhecido ou não autorizado para este perfil.");
        exit(1);
    }

    // Identity Guard Block (Operações destrutivas)
    let is_critical = matches!(
        &cli.command,
        Commands::Deploy { .. }
            | Commands::FactoryReset { .. }
            | Commands::Node {
                command: cli::NodeSubcommand::Publish,
            }
    );

    if is_critical && let Err(e) = &identity_result {
        eprintln!("Identity Guard Blocked Operation: {}", e);
        exit(1);
    }

    match cli.command {
        Commands::Switch { target } => {
            if let Err(e) = services::modules::run_switch(target) {
                eprintln!("Erro Crítico: {}", e);
                exit(1);
            }
        }
        Commands::Update {
            force_sync,
            no_stash,
            cleanup_stash,
        } => {
            if let Err(e) = services::update::run_update(force_sync, no_stash, cleanup_stash) {
                eprintln!("Erro Crítico: {}", e);
                exit(1);
            }
        }
        Commands::Status => {
            if let Err(e) = services::status::run_status() {
                eprintln!("Erro Crítico: {}", e);
                exit(1);
            }
        }
        Commands::Deploy {
            config_path,
            force,
            hostname,
        } => {
            // Environment Guard
            if !force && !services::env::check_is_live_iso() {
                eprintln!(
                    "ERRO: O comando 'deploy' é exclusivo para Live ISOs. Use 'kryx factory-reset' para restaurar o sistema instalado."
                );
                exit(1);
            }

            if let Err(e) =
                services::deployment::run_deploy(config_path.as_deref(), hostname.as_deref())
            {
                eprintln!("Erro Crítico: {}", e);
                exit(1);
            }
        }
        Commands::FactoryReset { preserve_home } => {
            if let Err(e) = services::deployment::run_factory_reset(preserve_home) {
                eprintln!("Erro Crítico no Reset: {}", e);
                exit(1);
            }
        }
        Commands::Doctor { json } => match services::diagnostics::run_doctor(json) {
            Ok(_) => {}
            Err(e) => {
                eprintln!("Erro: {}", e);
                exit(1);
            }
        },
        Commands::Identity { json } => match services::identity::check_identity() {
            Ok(identity) => {
                if json {
                    println!(
                        "{}",
                        serde_json::to_string(&identity).unwrap_or_else(|_| "{}".to_string())
                    );
                } else {
                    println!("Host Identity Guard: Ativo");
                    println!("UUID: {}", identity.uuid);
                    println!("Role: {:?}", identity.role);
                    println!("Edition: {}", identity.edition);
                }
            }
            Err(e) => {
                if json {
                    eprintln!("{{\"error\": \"{}\"}}", e);
                } else {
                    eprintln!("Erro: {}", e);
                }
                exit(1);
            }
        },
        Commands::Setup => {
            eprintln!("AVISO: kryx setup é um stub. Implementação real agendada para Fase 3.");
            exit(1);
        }
        Commands::System { command } => match command {
            cli::SystemSubcommand::Report => {
                if let Err(e) = services::telemetry::report_heartbeat() {
                    eprintln!("Erro: {}", e);
                    exit(1);
                }
            }
        },
        Commands::Theme => {
            if let Err(e) = services::theme::run_apply_theme() {
                eprintln!("Erro Crítico: {}", e);
                exit(1);
            }
        }
        Commands::Node { command } => {
            let action = match command {
                cli::NodeSubcommand::List => services::node::NodeAction::List,
                cli::NodeSubcommand::Publish => services::node::NodeAction::Publish,
                cli::NodeSubcommand::Reload => services::node::NodeAction::Reload,
                cli::NodeSubcommand::Reboot { mac_or_ip } => {
                    services::node::NodeAction::Reboot { target: mac_or_ip }
                }
            };
            if let Err(e) = services::node::run_node_command(action) {
                eprintln!("Erro Crítico: {}", e);
                exit(1);
            }
        }
        Commands::Feature { command } => match command {
            cli::FeatureSubcommand::List { json } => {
                if let Err(e) = services::feature::list_features(json) {
                    if json {
                        eprintln!("{{\"error\": \"{}\"}}", e);
                    } else {
                        eprintln!("{}", e);
                    }
                    exit(1);
                }
            }
        },
        Commands::Shell { args } => {
            if let Err(e) = services::passthrough::shell(args) {
                eprintln!("Erro: {}", e);
                exit(1);
            }
        }
        Commands::Search { args } => {
            if let Err(e) = services::passthrough::search(args) {
                eprintln!("Erro: {}", e);
                exit(1);
            }
        }
        Commands::Clean { args } => {
            // kryx clean runs the full cleanup pass (auto-gc 2d default +
            // .hm-bak-* + result* + /tmp/kryx-*) by default. If args are
            // passed, we forward them to nh clean as before.
            if args.is_empty() {
                kryx::ui::print_banner();
                eprintln!(
                    "{} Running full cleanup pass (gc 2d + .hm-bak + result + tmp)",
                    "[INFO]".cyan()
                );
                let report =
                    kryx::cleanup::run_full_cleanup(kryx::cleanup::DEFAULT_GC_KEEP, false, false);
                eprintln!("{} Cleanup summary: {}", "[PASS]".green(), report.summary());
                if !report.errors.is_empty() {
                    for err in &report.errors {
                        eprintln!("{} {}", "[WARN]".yellow(), err);
                    }
                    exit(1);
                }
            } else {
                if let Err(e) = services::passthrough::clean(args) {
                    eprintln!("Erro: {}", e);
                    exit(1);
                }
            }
        }
        Commands::Gc { args } => {
            if let Err(e) = services::passthrough::gc(args) {
                eprintln!("Erro: {}", e);
                exit(1);
            }
        }
        Commands::HomeManager { args } => {
            if let Err(e) = services::passthrough::home_manager(args) {
                eprintln!("Erro: {}", e);
                exit(1);
            }
        }
        Commands::CopyClosure { args } => {
            if let Err(e) = services::passthrough::copy_closure(args) {
                eprintln!("Erro: {}", e);
                exit(1);
            }
        }
        Commands::NixEnv { args } => {
            if let Err(e) = services::passthrough::nix_env(args) {
                eprintln!("Erro: {}", e);
                exit(1);
            }
        }
        Commands::NixChannel { args } => {
            if let Err(e) = services::passthrough::nix_channel(args) {
                eprintln!("Erro: {}", e);
                exit(1);
            }
        }
        Commands::Build { args } => {
            if let Err(e) = services::passthrough::build(args) {
                eprintln!("Erro: {}", e);
                exit(1);
            }
        }
        Commands::Check { path } => {
            if let Err(e) = services::passthrough::check(vec![path]) {
                eprintln!("Erro: {}", e);
                exit(1);
            }
        }
        Commands::Run { args } => {
            if let Err(e) = services::passthrough::run(args) {
                eprintln!("Erro: {}", e);
                exit(1);
            }
        }
        Commands::Develop { args } => {
            if let Err(e) = services::passthrough::develop(args) {
                eprintln!("Erro: {}", e);
                exit(1);
            }
        }
        Commands::Repl { args } => {
            if let Err(e) = services::passthrough::repl(args) {
                eprintln!("Erro: {}", e);
                exit(1);
            }
        }
        Commands::Fmt { args } => {
            if let Err(e) = services::passthrough::fmt(args) {
                eprintln!("Erro: {}", e);
                exit(1);
            }
        }
        Commands::Eval { args } => {
            if let Err(e) = services::passthrough::eval(args) {
                eprintln!("Erro: {}", e);
                exit(1);
            }
        }
        Commands::Flake { args } => {
            if let Err(e) = services::passthrough::flake(args) {
                eprintln!("Erro: {}", e);
                exit(1);
            }
        }
        Commands::PathInfo { args } => {
            if let Err(e) = services::passthrough::path_info(args) {
                eprintln!("Erro: {}", e);
                exit(1);
            }
        }
        Commands::Hash { args } => {
            if let Err(e) = services::passthrough::hash(args) {
                eprintln!("Erro: {}", e);
                exit(1);
            }
        }
        Commands::Store { args } => {
            if let Err(e) = services::passthrough::store(args) {
                eprintln!("Erro: {}", e);
                exit(1);
            }
        }
        Commands::Prefetch { args } => {
            if let Err(e) = kryx::nix_extra::prefetch(&args) {
                eprintln!("{}", e);
                exit(1);
            }
        }
        Commands::Registry { args } => {
            if let Err(e) = kryx::nix_extra::registry(&args) {
                eprintln!("{}", e);
                exit(1);
            }
        }
        Commands::Edit { args } => {
            if let Err(e) = kryx::nix_extra::edit(&args) {
                eprintln!("{}", e);
                exit(1);
            }
        }
        Commands::SignPaths { args } => {
            if let Err(e) = kryx::nix_extra::sign_paths(&args) {
                eprintln!("{}", e);
                exit(1);
            }
        }
        Commands::Copy { args } => {
            if let Err(e) = kryx::nix_extra::copy(&args) {
                eprintln!("{}", e);
                exit(1);
            }
        }
        Commands::NixDoctor { args } => {
            if let Err(e) = kryx::nix_extra::nix_doctor(&args) {
                eprintln!("{}", e);
                exit(1);
            }
        }
        Commands::NhCleanAll { args } => {
            if let Err(e) = kryx::nix_extra::nh_clean_all(&args) {
                eprintln!("{}", e);
                exit(1);
            }
        }
        Commands::NixosRebuild { args } => {
            if let Err(e) = kryx::nix_extra::nixos_rebuild(&args) {
                eprintln!("{}", e);
                exit(1);
            }
        }
        Commands::NhOs { args } => {
            // args[0] must be a variant; rest are forwarded.
            if args.is_empty() {
                eprintln!(
                    "kryx nhos: missing variant. Use: boot, test, dry-activate, dry-build, build"
                );
                exit(1);
            }
            let variant = args[0].clone();
            let rest: Vec<String> = args.iter().skip(1).cloned().collect();
            if let Err(e) = kryx::nix_extra::nh_os_variant(&variant, &rest) {
                eprintln!("{}", e);
                exit(1);
            }
        }
        Commands::Completion { shell } => {
            use clap_complete::{Shell, generate};
            let shell_enum = match shell.as_str() {
                "bash" => Shell::Bash,
                "zsh" => Shell::Zsh,
                "fish" => Shell::Fish,
                "elvish" => Shell::Elvish,
                "powershell" => Shell::PowerShell,
                other => {
                    eprintln!(
                        "Shell não suportado: {}. Use: bash, zsh, fish, elvish, powershell",
                        other
                    );
                    exit(1);
                }
            };
            let mut cmd = Cli::command();
            let bin_name = cmd.get_name().to_string();
            generate(shell_enum, &mut cmd, bin_name, &mut std::io::stdout());
        }
        Commands::Kve { command } => {
            if let Err(e) = cli::kve::run(command) {
                eprintln!("{}", e);
                exit(2);
            }
        }
        Commands::Vm { command } => {
            let mapped = map_vm_to_kve(command);
            if let Err(e) = cli::kve::run(mapped) {
                eprintln!("{}", e);
                exit(2);
            }
        }
        Commands::Ct { command } => {
            let mapped = map_ct_to_kve(command);
            if let Err(e) = cli::kve::run(mapped) {
                eprintln!("{}", e);
                exit(2);
            }
        }
        Commands::Think { command } => {
            if let Err(e) = cli::think::run(command) {
                eprintln!("{}", e);
                exit(2);
            }
        }
    }
}

/// Converte `kryx vm <subcmd>` para o subcomando KVE equivalente.
fn map_vm_to_kve(cmd: cli::VmSubcommand) -> cli::kve::KveCommand {
    use cli::VmSubcommand as V;
    use cli::kve::KveCommand as K;
    match cmd {
        V::List { json } => K::Vms { json },
        V::Info { name, json } => K::Instance { name, json },
    }
}

/// Converte `kryx ct <subcmd>` para o subcomando KVE equivalente.
fn map_ct_to_kve(cmd: cli::CtSubcommand) -> cli::kve::KveCommand {
    use cli::CtSubcommand as C;
    use cli::kve::KveCommand as K;
    match cmd {
        C::List { json } => K::Containers { json },
        C::Info { name, json } => K::Instance { name, json },
    }
}
