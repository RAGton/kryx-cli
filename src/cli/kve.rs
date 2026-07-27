//! `kryx kve` — proxy para `/api/v2/kve/*` do daemon kryxd.
//!
//! Implementação real consumindo o slice vertical Incus do kryxd.
//! A CLI fala **somente** com o daemon; nunca toca em Incus diretamente.
//!
//! Endpoints consumidos (v2):
//! - GET /api/v2/kve/health
//! - GET /api/v2/kve/instances
//! - GET /api/v2/kve/storage
//!
//! Subcomandos:
//! - `kryx kve health`
//! - `kryx kve instances [--json]`
//! - `kryx kve storage   [--json]`
//! - `kryx vm list/info`  (alias via wrapper em main.rs)
//! - `kryx ct list/info`  (alias via wrapper em main.rs)

use clap::Subcommand;
use cli_table::{print_stdout, Table};
use serde::{Deserialize, Serialize};

use kryx::client;

#[derive(Subcommand, Debug)]
pub enum KveCommand {
    /// Verifica a saúde do backend Incus via kryxd.
    Health {
        #[arg(long)]
        json: bool,
    },
    /// Lista todas as instâncias (VMs + containers).
    Instances {
        #[arg(long)]
        json: bool,
    },
    /// Lista apenas containers.
    Containers {
        #[arg(long)]
        json: bool,
    },
    /// Lista apenas VMs.
    Vms {
        #[arg(long)]
        json: bool,
    },
    /// Detalhe de uma instância por nome.
    Instance {
        name: String,
        #[arg(long)]
        json: bool,
    },
    /// Lista storage pools.
    Storage {
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Deserialize, Serialize)]
struct InstancesEnvelope {
    #[serde(default)]
    instances: Vec<serde_json::Value>,
}

#[derive(Debug, Deserialize, Serialize)]
struct StorageEnvelope {
    #[serde(default)]
    storage: Vec<serde_json::Value>,
}

#[derive(Debug, Deserialize, Serialize)]
struct HealthEnvelope {
    status: String,
    source: String,
    #[serde(default)]
    socket: Option<String>,
}

#[derive(Table)]
struct InstanceRow {
    #[table(title = "Name")]
    name: String,
    #[table(title = "Kind")]
    kind: String,
    #[table(title = "State")]
    state: String,
    #[table(title = "IPv4")]
    ipv4: String,
    #[table(title = "Arch")]
    arch: String,
}

#[derive(Table)]
struct StorageRow {
    #[table(title = "Name")]
    name: String,
    #[table(title = "Driver")]
    driver: String,
    #[table(title = "State")]
    state: String,
    #[table(title = "Description")]
    description: String,
}

fn row_of_instance(v: &serde_json::Value) -> InstanceRow {
    let name = v.get("name").and_then(|x| x.as_str()).unwrap_or("?").to_string();
    let kind = v.get("kind").and_then(|x| x.as_str()).unwrap_or("?").to_string();
    let state = v.get("state").and_then(|x| x.as_str()).unwrap_or("?").to_string();
    let ipv4 = v
        .get("ipv4")
        .and_then(|x| x.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    let arch = v
        .get("architecture")
        .and_then(|x| x.as_str())
        .unwrap_or("?")
        .to_string();
    InstanceRow { name, kind, state, ipv4, arch }
}

fn row_of_storage(v: &serde_json::Value) -> StorageRow {
    let name = v.get("name").and_then(|x| x.as_str()).unwrap_or("?").to_string();
    let driver = v.get("driver").and_then(|x| x.as_str()).unwrap_or("?").to_string();
    let state = v.get("state").and_then(|x| x.as_str()).unwrap_or("?").to_string();
    let description = v
        .get("description")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    StorageRow { name, driver, state, description }
}

fn list_instances(json: bool, kind_filter: Option<&str>) -> Result<(), String> {
    let raw = client::get_v2_raw("kve/instances").map_err(|e| e.to_string())?;
    let env: InstancesEnvelope = serde_json::from_value(raw.clone())
        .map_err(|e| format!("resposta kryxd invalida: {e}"))?;
    let items: Vec<serde_json::Value> = env
        .instances
        .into_iter()
        .filter(|v| match kind_filter {
            Some(k) => v.get("kind").and_then(|x| x.as_str()) == Some(k),
            None => true,
        })
        .collect();

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "instances": items,
                "count": items.len(),
            }))
            .map_err(|e| e.to_string())?
        );
        return Ok(());
    }

    if items.is_empty() {
        eprintln!("(no instances)");
        return Ok(());
    }

    let rows: Vec<InstanceRow> = items.iter().map(row_of_instance).collect();
    print_stdout(rows).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn run(cmd: KveCommand) -> Result<(), String> {
    match cmd {
        KveCommand::Health { json } => {
            let raw = client::get_v2_raw("kve/health").map_err(|e| e.to_string())?;
            let env: HealthEnvelope = serde_json::from_value(raw.clone())
                .map_err(|e| format!("resposta kryxd invalida: {e}"))?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&env).map_err(|e| e.to_string())?
                );
                return Ok(());
            }
            println!("status : {}", env.status);
            println!("source : {}", env.source);
            if let Some(s) = env.socket {
                println!("socket : {s}");
            }
            Ok(())
        }
        KveCommand::Instances { json } => list_instances(json, None),
        KveCommand::Containers { json } => list_instances(json, Some("container")),
        KveCommand::Vms { json } => list_instances(json, Some("virtual-machine")),
        KveCommand::Instance { name, json } => {
            let raw = client::get_v2_raw("kve/instances").map_err(|e| e.to_string())?;
            let env: InstancesEnvelope = serde_json::from_value(raw.clone())
                .map_err(|e| format!("resposta kryxd invalida: {e}"))?;
            let found = env.instances.into_iter().find(|v| {
                v.get("name").and_then(|x| x.as_str()) == Some(name.as_str())
            });
            match found {
                Some(v) => {
                    if json {
                        println!(
                            "{}",
                            serde_json::to_string_pretty(&v).map_err(|e| e.to_string())?
                        );
                        Ok(())
                    } else {
                        let row = row_of_instance(&v);
                        print_stdout(vec![row]).map_err(|e| e.to_string())?;
                        Ok(())
                    }
                }
                None => Err(format!("instance '{name}' nao encontrada")),
            }
        }
        KveCommand::Storage { json } => {
            let raw = client::get_v2_raw("kve/storage").map_err(|e| e.to_string())?;
            let env: StorageEnvelope = serde_json::from_value(raw.clone())
                .map_err(|e| format!("resposta kryxd invalida: {e}"))?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&env).map_err(|e| e.to_string())?
                );
                return Ok(());
            }
            if env.storage.is_empty() {
                eprintln!("(no storage pools)");
                return Ok(());
            }
            let rows: Vec<StorageRow> = env.storage.iter().map(row_of_storage).collect();
            print_stdout(rows).map_err(|e| e.to_string())?;
            Ok(())
        }
    }
}