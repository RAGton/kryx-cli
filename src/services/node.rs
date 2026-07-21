use colored::Colorize;
use std::fs;
use std::process::{Command, Stdio};

pub enum NodeAction {
    List,
    Publish,
    Reload,
    Reboot { target: Option<String> },
}

pub fn run_node_command(action: NodeAction) -> Result<(), String> {
    match action {
        NodeAction::List => {
            println!("{} Consultando clientes PXE ativos...", "[INFO]".cyan());

            let lease_files = [
                "/var/lib/dnsmasq/dnsmasq.leases",
                "/var/lib/misc/dnsmasq.leases",
                "/var/lib/dhcp/dhcpd.leases",
            ];

            let mut found = false;
            for file in &lease_files {
                if let Ok(content) = fs::read_to_string(file) {
                    if !content.trim().is_empty() {
                        found = true;
                        println!("Leases encontrados em {}:", file.bold());
                        // Parsing básico para tabularização minimalista se for dnsmasq
                        for line in content.lines() {
                            let parts: Vec<&str> = line.split_whitespace().collect();
                            if parts.len() >= 4 {
                                println!(
                                    "IP: {:<15} MAC: {:<17} Hostname: {:<20}",
                                    parts[2], parts[1], parts[3]
                                );
                            } else {
                                println!("{}", line);
                            }
                        }
                    }
                }
            }
            if !found {
                println!("Nenhum cliente PXE registrado ou concessão de IP ativa encontrada.");
            }
            Ok(())
        }
        NodeAction::Publish => {
            println!(
                "{} Iniciando build declarativo da imagem diskless (node-client)...",
                "[INFO]".cyan()
            );
            let status = Command::new("nix")
                .args([
                    "build",
                    "/etc/kryonixos#nixosConfigurations.node-client.config.system.build.toplevel",
                    "-o",
                    "/tmp/kryonix-node-build",
                ])
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .status()
                .map_err(|e| format!("Falha ao invocar nix build: {}", e))?;

            if !status.success() {
                return Err("Configuração 'node-client' não encontrada no Flake /etc/kryonixos ou build falhou.".to_string());
            }

            println!(
                "{} Atualizando symlink do servidor PXE/HTTP...",
                "[INFO]".cyan()
            );

            let link_target = "/tmp/kryonix-node-build";
            // Usa o caminho principal pedido. Opcional testar se /var/lib/kryonix/pxe existe.
            let link_path = if std::path::Path::new("/srv/data/images").exists() {
                "/srv/data/images/latest"
            } else {
                "/var/lib/kryonix/pxe/latest"
            };

            // Garantir que a pasta pai exista
            let parent_dir = std::path::Path::new(link_path).parent().unwrap();
            if !parent_dir.exists() {
                if let Err(e) = fs::create_dir_all(parent_dir) {
                    println!(
                        "{} Aviso: Falha ao criar pasta {:?}: {}",
                        "[WARN]".yellow(),
                        parent_dir,
                        e
                    );
                }
            }

            let ln_status = Command::new("ln")
                .args(["-sfn", link_target, link_path])
                .status()
                .map_err(|e| format!("Falha ao invocar ln: {}", e))?;

            if ln_status.success() {
                println!(
                    "{} Publicação concluída com sucesso! Imagem disponível em {}",
                    "[PASS]".green(),
                    link_path
                );
                Ok(())
            } else {
                Err("Falha ao atualizar o link simbólico da imagem.".to_string())
            }
        }
        NodeAction::Reload => {
            println!(
                "{} Reiniciando serviços de boot de rede (ipxe-http-server, tftpd)...",
                "[INFO]".cyan()
            );
            let status = Command::new("systemctl")
                .args(["restart", "ipxe-http-server", "tftpd"])
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .status()
                .map_err(|e| format!("Falha ao invocar systemctl restart: {}", e))?;

            if status.success() {
                println!("{} Serviços reiniciados com sucesso!", "[PASS]".green());
                Ok(())
            } else {
                Err("Falha ao reiniciar serviços de boot.".to_string())
            }
        }
        NodeAction::Reboot { target } => {
            let t = target.unwrap_or_else(|| "all".to_string());
            println!(
                "{} Reinício remoto solicitado para estação(ões): {}",
                "[INFO]".cyan(),
                t
            );
            println!("Mock: Enviando sinal de reboot (não implementado).");
            Ok(())
        }
    }
}
