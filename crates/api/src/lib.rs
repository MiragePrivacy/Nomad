use std::{sync::Arc, time::SystemTime};

use aide::{
    axum::{
        routing::{get_with, post_with},
        ApiRouter, IntoApiResponse,
    },
    openapi::OpenApi,
    scalar::Scalar,
    transform::TransformOperation,
};
use axum::{extract::State, http::StatusCode, Extension, Json};
use serde::{Deserialize, Serialize};
use tokio::{net::TcpListener, sync::mpsc::UnboundedSender};
use tracing::{debug, info};

use nomad_types::SignalPayload;

pub mod types;

use crate::types::{HealthResponse, SignalRequest};

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

async fn health(State(app_state): State<AppState>) -> impl IntoApiResponse {
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

async fn signal(
    State(app_state): State<AppState>,
    Json(req): Json<SignalRequest>,
) -> (StatusCode, String) {
    info!("Received signal");
    if app_state.signal_tx.send(req.into()).is_err() {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to broadcast signal".to_string(),
        )
    } else {
        (StatusCode::OK, "Signal acknowledged".into())
    }
}

fn health_docs(op: TransformOperation) -> TransformOperation {
    op.tag("Nomad API")
        .description("Get node health information")
        .response::<200, Json<HealthResponse>>()
}

fn signal_docs(op: TransformOperation) -> TransformOperation {
    op.tag("Nomad API")
        .description("Submit a new signal to the node")
        .response_with::<200, String, _>(|t| t.example("Signal acknowledged"))
        .response_with::<500, String, _>(|t| t.example("Failed to broadcast signal"))
}

async fn serve_docs(Extension(api): Extension<Arc<OpenApi>>) -> impl IntoApiResponse {
    Json(api)
}

pub async fn spawn_api_server(
    config: ApiConfig,
    is_bootstrap: bool,
    read_only: bool,
    signal_tx: UnboundedSender<SignalPayload>,
) -> eyre::Result<()> {
    debug!(?config);

    aide::generate::extract_schemas(true);
    let mut api = OpenApi::default();
    let app = ApiRouter::new()
        .api_route("/health", get_with(health, health_docs))
        .api_route("/signal", post_with(signal, signal_docs))
        .route(
            "/scalar",
            Scalar::new("/openapi.json")
                .with_title("Nomad Playground")
                .axum_route(),
        )
        .finish_api_with(&mut api, |api| api)
        .route("/openapi.json", axum::routing::get(serve_docs))
        .layer(Extension(Arc::new(api)))
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
