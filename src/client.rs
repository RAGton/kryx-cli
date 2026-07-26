//! Cliente HTTP para o daemon `kryxd` (Axum/Vite Control Plane).
//!
//! Fase 2: consumido por `kryx kve *` e `kryx think *` para falar com
//! os stubs V2 (HTTP 200 + JSON com `status: "stub"`).
//!
//! Decisoes:
//! - Usa `ureq` (sincrono, ja presente) — `reqwest` async so na Fase 2.5.
//! - Falha de conexao NAO quebra a CLI: retorna `KryxdError` que o
//!   dispatcher imprime de forma elegante e sai com codigo 2.
//! - Endpoint base: KRYXD_URL env > `http://127.0.0.1:8080` (default).
//! - Sem auth nesta fase: a Fase 1 do KCP provera token via
//!   `services.kryxd.token`; a CLI passara a usar header
//!   `X-Kryonix-Installer-Token` em fase posterior.

use serde::de::DeserializeOwned;
use std::fmt;

const DEFAULT_KRYXD_URL: &str = "http://127.0.0.1:8080";

/// Erro de comunicacao com o daemon kryxd.
#[derive(Debug)]
pub enum KryxdError {
    /// Nao conseguiu conectar / timeout / conexao recusada
    Unreachable(String),
    /// kryxd respondeu com HTTP >= 400
    HttpStatus { status: u16, body: String },
    /// Resposta nao e JSON valido, ou nao desserializa no tipo esperado
    Decode(String),
}

impl fmt::Display for KryxdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            KryxdError::Unreachable(detail) => {
                write!(f, "KVE Backend Error: daemon is unreachable ({})", detail)
            }
            KryxdError::HttpStatus { status, body } => write!(
                f,
                "KVE Backend Error: HTTP {} — {}",
                status,
                body.chars().take(200).collect::<String>()
            ),
            KryxdError::Decode(detail) => {
                write!(f, "KVE Backend Error: malformed response ({})", detail)
            }
        }
    }
}

impl std::error::Error for KryxdError {}

/// Resolve a URL base do kryxd.
pub fn base_url() -> String {
    std::env::var("KRYXD_URL").unwrap_or_else(|_| DEFAULT_KRYXD_URL.to_string())
}

/// GET em `/api/v2/{path}` retornando JSON desserializado em `T`.
pub fn get_v2<T: DeserializeOwned>(path: &str) -> Result<T, KryxdError> {
    let url = format!("{}/api/v2/{}", base_url().trim_end_matches('/'), path);
    let agent = ureq::AgentBuilder::new()
        .timeout_read(std::time::Duration::from_secs(5))
        .timeout_write(std::time::Duration::from_secs(5))
        .build();

    let response = agent.get(&url).call().map_err(|e| match e {
        ureq::Error::Status(status, response) => {
            let body = response
                .into_string()
                .unwrap_or_else(|_| "<body unreadable>".to_string());
            KryxdError::HttpStatus {
                status: status,
                body,
            }
        }
        ureq::Error::Transport(t) => KryxdError::Unreachable(t.to_string()),
    })?;

    let body = response
        .into_string()
        .map_err(|e| KryxdError::Decode(e.to_string()))?;

    serde_json::from_str::<T>(&body).map_err(|e| KryxdError::Decode(e.to_string()))
}

/// Helper: GET retornando `serde_json::Value` cru (para casos onde
/// o caller quer decidir a formatacao).
pub fn get_v2_raw(path: &str) -> Result<serde_json::Value, KryxdError> {
    get_v2::<serde_json::Value>(path)
}
