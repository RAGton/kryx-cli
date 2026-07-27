//! `kryx think` — proxy para `/api/v2/think/*` do daemon kryxd.
//!
//! Fase 2: consumido pelos stubs V2. Implementacao real
//! (cluster map + zpool status) entra na Fase 3.

use clap::Subcommand;
use cli_table::{print_stdout, Table};

use kryx::client;

#[derive(Subcommand, Debug)]
pub enum ThinkCommand {
    /// Mostra a topologia do cluster Think (nodes + rede PXE/DHCP).
    Topology,
    /// Lista pools ZFS registrados no Think.
    #[command(name = "storage")]
    StorageZfs,
}

#[derive(Table)]
struct ThinkRow {
    #[table(title = "Source")]
    source: String,
    #[table(title = "Status")]
    status: String,
    #[table(title = "Count")]
    count: String,
    #[table(title = "Detail")]
    detail: String,
}

pub fn run(cmd: ThinkCommand) -> Result<(), String> {
    match cmd {
        ThinkCommand::Topology => {
            let value = client::get_v2_raw("think/topology")
                .map_err(|e| e.to_string())?;
            let status = value
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("?")
                .to_string();
            let count = value
                .get("nodes")
                .and_then(|v| v.as_array())
                .map(|a| a.len().to_string())
                .unwrap_or_else(|| "?".to_string());
            let network = value
                .get("network")
                .and_then(|v| v.as_object())
                .map(|m| {
                    let pxe = m.get("pxe").and_then(|v| v.as_str()).unwrap_or("?");
                    let dhcp = m.get("dhcp").and_then(|v| v.as_str()).unwrap_or("?");
                    format!("pxe={} dhcp={}", pxe, dhcp)
                })
                .unwrap_or_else(|| "?".to_string());

            let rows = vec![ThinkRow {
                source: "think:topology".to_string(),
                status,
                count,
                detail: network,
            }];
            print_stdout(rows).map_err(|e| e.to_string())?;
            Ok(())
        }
        ThinkCommand::StorageZfs => {
            let value = client::get_v2_raw("think/storage/zfs")
                .map_err(|e| e.to_string())?;
            let source = value
                .get("source")
                .and_then(|v| v.as_str())
                .unwrap_or("?")
                .to_string();
            let status = value
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("?")
                .to_string();
            let count = value
                .get("pools")
                .and_then(|v| v.as_array())
                .map(|a| a.len().to_string())
                .unwrap_or_else(|| "?".to_string());

            let rows = vec![ThinkRow {
                source,
                status,
                count,
                detail: "see /api/v2/think/storage/zfs for full payload".to_string(),
            }];
            print_stdout(rows).map_err(|e| e.to_string())?;
            Ok(())
        }
    }
}
