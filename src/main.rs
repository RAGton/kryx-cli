mod cli;
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

    // Authorization Hook
    let authorized = match &cli.command {
        Commands::Identity { .. } | Commands::Setup => true,
        Commands::Deploy { .. } | Commands::Node { .. } | Commands::Feature { .. } => is_core,
        _ => is_core || is_desktop,
    };

    if !authorized {
        eprintln!("Erro: Comando desconhecido ou não autorizado para este perfil.");
        exit(1);
    }

    // Identity Guard Block (Operações destrutivas)
    let is_critical = match &cli.command {
        Commands::Deploy { .. } | Commands::FactoryReset { .. } => true,
        Commands::Node {
            command: cli::NodeSubcommand::Publish,
        } => true,
        _ => false,
    };

    if is_critical {
        if let Err(e) = &identity_result {
            eprintln!("Identity Guard Blocked Operation: {}", e);
            exit(1);
        }
    }

    match cli.command {
        Commands::Switch { target } => {
            if let Err(e) = services::modules::run_switch(target) {
                eprintln!("Erro Crítico: {}", e);
                exit(1);
            }
        }
        Commands::Update => {
            if let Err(e) = services::update::run_update() {
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
            println!("Setup não implementado ainda.");
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
    }
}
