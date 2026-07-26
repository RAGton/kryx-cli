//! `kryx kve` — proxy para `/api/v2/kve/*` do daemon kryxd.
//!
//! Fase 2: consumido pelos stubs V2. Implementacao real
//! (Incus + ZFS) entra na Fase 3 (backend) — esta CLI ja
//! tem o shape estavel.

use clap::Subcommand;
use cli_table::{print_stdout, Table};

use kryx::client;

#[derive(Subcommand, Debug)]
pub enum KveCommand {
    /// Lista instancias (VM/CT) gerenciadas pelo KVE.
    Instances,
    /// Lista datasets ZFS atrelados a pool do Incus.
    Storage,
}

#[derive(Table)]
struct InstancesRow {
    #[table(title = "Source")]
    source: String,
    #[table(title = "Status")]
    status: String,
    #[table(title = "Count")]
    count: String,
    #[table(title = "Detail")]
    detail: String,
}

pub fn run(cmd: KveCommand) -> Result<(), String> {
    match cmd {
        KveCommand::Instances => {
            let value = client::get_v2_raw("kve/instances")
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
                .get("instances")
                .and_then(|v| v.as_array())
                .map(|a| a.len().to_string())
                .unwrap_or_else(|| "?".to_string());

            let rows = vec![InstancesRow {
                source,
                status,
                count,
                detail: "see /api/v2/kve/instances for full payload".to_string(),
            }];
            print_stdout(rows).map_err(|e| e.to_string())?;
            Ok(())
        }
        KveCommand::Storage => {
            let value = client::get_v2_raw("kve/storage")
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
                .get("datasets")
                .and_then(|v| v.as_array())
                .map(|a| a.len().to_string())
                .unwrap_or_else(|| "?".to_string());

            let rows = vec![InstancesRow {
                source,
                status,
                count,
                detail: "see /api/v2/kve/storage for full payload".to_string(),
            }];
            print_stdout(rows).map_err(|e| e.to_string())?;
            Ok(())
        }
    }
}
