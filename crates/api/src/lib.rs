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
}

async fn health(State(app_state): State<AppState>) -> impl IntoApiResponse {
    let uptime_seconds = app_state.start_time.elapsed().unwrap_or_default().as_secs();

    let health_response = HealthResponse {
        status: "healthy".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        node_type: "nomad".to_string(),
        uptime_seconds,
    };

    Json(health_response)
}

async fn signal(
    State(app_state): State<AppState>,
    Json(req): Json<SignalRequest>,
) -> impl IntoApiResponse {
    info!("Received");
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
    op.description("Get node health information")
        .response::<200, Json<HealthResponse>>()
}

fn signal_docs(op: TransformOperation) -> TransformOperation {
    op.description("Submit a new signal to the node")
        .response::<200, String>()
}

async fn serve_docs(Extension(api): Extension<Arc<OpenApi>>) -> impl IntoApiResponse {
    Json(api)
}

pub async fn spawn_api_server(
    config: ApiConfig,
    signal_tx: UnboundedSender<SignalPayload>,
) -> eyre::Result<()> {
    debug!(?config);

    aide::generate::extract_schemas(true);
    let mut api = OpenApi::default();
    let app = ApiRouter::new()
        .api_route("/health", get_with(health, health_docs))
        .api_route("/signal", post_with(signal, signal_docs))
        .route("/scalar", Scalar::new("/openapi.json").axum_route())
        .finish_api_with(&mut api, |api| api)
        .route("/openapi.json", axum::routing::get(serve_docs))
        .layer(Extension(Arc::new(api)))
        .with_state(AppState {
            signal_tx,
            start_time: SystemTime::now(),
        });

    let listener = TcpListener::bind(("0.0.0.0", config.port)).await?;
    info!("RPC server running on {:?}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await });
    Ok(())
}
