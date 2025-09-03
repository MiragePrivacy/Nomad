use std::sync::Arc;

use aide::{
    axum::{routing::post_with, ApiRouter, IntoApiResponse},
    openapi::OpenApi,
    scalar::Scalar,
    transform::TransformOperation,
};
use axum::{extract::State, http::StatusCode, Extension, Json};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::{net::TcpListener, sync::mpsc::UnboundedSender};
use tracing::{debug, info};

use nomad_types::{EncryptedSignal, Signal, SignalPayload};

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

#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
#[schemars(untagged)]
pub enum SignalRequest {
    Unencrypted(Signal),
    Encrypted(EncryptedSignal),
}

impl From<SignalRequest> for SignalPayload {
    fn from(v: SignalRequest) -> Self {
        match v {
            SignalRequest::Encrypted(s) => Self::Encrypted(s),
            SignalRequest::Unencrypted(s) => Self::Unencrypted(s),
        }
    }
}

#[axum::debug_handler]
async fn signal(
    State(tx): State<UnboundedSender<SignalPayload>>,
    Json(req): Json<SignalRequest>,
) -> impl IntoApiResponse {
    info!("Received");
    if tx.send(req.into()).is_err() {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to broadcast signal".to_string(),
        )
    } else {
        (StatusCode::OK, "Signal acknowledged".into())
    }
}

fn signal_docs(op: TransformOperation) -> TransformOperation {
    op.description("Submit a new signal to the node")
        .response::<200, String>()
}

async fn serve_docs(Extension(api): Extension<Arc<OpenApi>>) -> Json<Arc<OpenApi>> {
    Json(api)
}

pub async fn spawn_api_server(
    config: ApiConfig,
    signal_tx: UnboundedSender<SignalPayload>,
) -> eyre::Result<()> {
    debug!(?config);

    aide::generate::on_error(|error| {
        println!("{error}");
    });
    aide::generate::extract_schemas(true);

    let mut api = OpenApi::default();
    let app = ApiRouter::new()
        .api_route("/signal", post_with(signal, signal_docs))
        .route("/scalar", Scalar::new("/openapi.json").axum_route())
        .finish_api_with(&mut api, |api| api)
        .route("/openapi.json", axum::routing::get(serve_docs))
        .layer(Extension(Arc::new(api)))
        .with_state(signal_tx);

    let listener = TcpListener::bind(("0.0.0.0", config.port)).await?;
    info!("RPC server running on {:?}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await });
    Ok(())
}
