use std::time::SystemTime;

use axum::{extract::State, http::StatusCode, Json};
use serde::{Deserialize, Serialize};
use tokio::{net::TcpListener, sync::mpsc::UnboundedSender};
use tower_http::cors::{self, CorsLayer};
use tracing::{debug, info};

use nomad_types::SignalPayload;
use utoipa::OpenApi;
use utoipa_axum::{router::OpenApiRouter, routes};
use utoipa_scalar::{Scalar, Servable};

pub mod types;

use crate::types::HealthResponse;

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct ApiConfig {
    pub port: u16,
}

impl Default for ApiConfig {
    fn default() -> Self {
        Self { port: 8000 }
    }
}

#[derive(Clone)]
pub struct AppState {
    pub signal_tx: UnboundedSender<SignalPayload>,
    pub start_time: SystemTime,
    pub is_bootstrap: bool,
    pub read_only: bool,
}

#[utoipa::path(
    get, path = "/health",
    responses((status = OK, body = HealthResponse))
)]
async fn health(State(app_state): State<AppState>) -> Json<HealthResponse> {
    let uptime_seconds = app_state.start_time.elapsed().unwrap_or_default().as_secs();
    Json(HealthResponse {
        status: "healthy".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        kind: "nomad".to_string(),
        uptime_seconds,
        is_bootstrap: app_state.is_bootstrap,
        read_only: app_state.read_only,
    })
}

#[utoipa::path(
    post, path = "/signal",
    request_body = SignalPayload,
    responses(
        (status = OK, body = str, description = "Signal acknowledged"),
        (status = BAD_REQUEST, body = str, description = "Signal puzzle must have at least 500 bytes"),
        (status = INTERNAL_SERVER_ERROR, body = str, description = "Failed to broadcast signal")
    )
)]
async fn signal(
    State(app_state): State<AppState>,
    Json(req): Json<SignalPayload>,
) -> (StatusCode, String) {
    if app_state.signal_tx.send(req).is_err() {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to broadcast signal".to_string(),
        )
    } else {
        (StatusCode::OK, "Signal acknowledged".into())
    }
}

#[derive(OpenApi)]
#[openapi()]
struct ApiDoc;

pub async fn spawn_api_server(
    config: ApiConfig,
    is_bootstrap: bool,
    read_only: bool,
    signal_tx: UnboundedSender<SignalPayload>,
) -> eyre::Result<()> {
    debug!(?config);

    let (router, api) = OpenApiRouter::with_openapi(ApiDoc::openapi())
        .routes(routes!(health, signal))
        .split_for_parts();

    let app = router
        .merge(Scalar::with_url("/scalar", api))
        .layer(
            CorsLayer::new()
                .allow_origin(cors::Any)
                .allow_headers(cors::Any)
                .allow_methods([
                    axum::http::Method::GET,
                    axum::http::Method::POST,
                    axum::http::Method::OPTIONS,
                ]),
        )
        .with_state(AppState {
            is_bootstrap,
            read_only,
            signal_tx,
            start_time: SystemTime::now(),
        });

    let listener = TcpListener::bind(("0.0.0.0", config.port)).await?;
    info!("RPC server running on {:?}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await });
    Ok(())
}
