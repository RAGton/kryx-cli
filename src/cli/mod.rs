use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "kryx", version = "0.1.0", author, about = "Kryonix Unified CLI", long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Operação atômica de reconstrução e transição do sistema
    Switch {
        /// Hostname alvo opcional
        target: Option<String>,
    },
    /// Gerencia deploy de imagens diskless (NODE)
    Deploy {
        /// Caminho para a configuração gerada do instalador
        config_path: Option<String>,
        /// Ignora a verificação do Environment Guard e força o deploy em sistemas instalados
        #[arg(long, short)]
        force: bool,
        /// Hostname alvo (flake attribute) para instanciar (ex: thinkServer)
        #[arg(long, short)]
        hostname: Option<String>,
    },
    /// Reseta o sistema físico para as configurações originais
    FactoryReset {
        /// Preserva os dados do usuário em partições separadas (/home ou subvolumes persistentes)
        #[arg(long, default_value_t = true)]
        preserve_home: bool,
    },
    /// Gestão de estado do sistema e telemetria
    System {
        #[command(subcommand)]
        command: SystemSubcommand,
    },
    /// Diagnóstico contextual do host atual
    Doctor {
        /// Emite o relatório em JSON para automações e agentes
        #[arg(long)]
        json: bool,
    },
    /// Validação e exibição da identidade do host
    Identity {
        #[arg(long)]
        json: bool,
    },
    /// Configuração inicial (Bootstrap)
    Setup,
    /// Gerenciamento de temas
    Theme,
    /// Gerenciamento de Nodos (NODE Clientes)
    Node {
        #[command(subcommand)]
        command: NodeSubcommand,
    },
    /// Gerenciamento de Features
    Feature {
        #[command(subcommand)]
        command: FeatureSubcommand,
    },
    /// Atualiza repositórios Git (/etc/kryonix) e locks de flake
    Update {
        /// Faz stash de alterações locais antes do pull (usar com cautela)
        #[arg(long)]
        force_sync: bool,
    },
    /// Inspeção de saúde (ZFS, KVE/Incus, Serviços, Telemetria)
    Status,
}

#[derive(Subcommand)]
pub enum NodeSubcommand {
    /// Lista clientes PXE ativos (IP/MAC/Status)
    List,
    /// Compila e publica nova imagem de cliente no repositório HTTP/PXE
    Publish,
    /// Reinicia serviços de boot por rede (TFTP/iPXE/HTTP)
    Reload,
    /// Reinício remoto de estações diskless
    Reboot { mac_or_ip: Option<String> },
}

#[derive(Subcommand, Debug)]
pub enum FeatureSubcommand {
    /// Lista o status das features baseadas na identidade atual
    List {
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum SystemSubcommand {
    /// Exibe e reporta a telemetria baseada no manifesto local
    Report,
}
