mod state;
mod storage;
mod workers;

use secp256k1::hashes::hex::DisplayHex;
use serde::Serialize;
use state::{ApplicationState, ServiceStatus};
use std::{ops::Deref, sync::Arc};
use storage::SpotEntryEvent;
use tokio::sync::mpsc;
use workers::WorkerRunner;

use axum::{
    Json, Router,
    extract::State,
    http::{StatusCode, header::CONTENT_TYPE},
    response::{AppendHeaders, IntoResponse},
    routing::get,
};

// TODO: add proper serialiser
#[derive(Serialize)]
struct Data {
    twap: String,
    signature: String,
    pk: String,
}

async fn data_handler(State(state): State<Arc<ApplicationState>>) -> impl IntoResponse {
    let storage = { state.storage.read().unwrap() };

    let twap = if let Some(value) = storage.twap.clone() {
        value
    } else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            AppendHeaders([(CONTENT_TYPE, "application/json")]),
            Json(Result::Err("Data not ready".to_string())),
        );
    };

    let signature = if let Some(value) = storage.signature {
        value
    } else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            AppendHeaders([(CONTENT_TYPE, "application/json")]),
            Json(Result::Err("Data not ready".to_string())),
        );
    };

    let twap_bytes = [twap.to_bytes_be()].concat();
    let twap_serialised = twap_bytes.to_lower_hex_string();

    let signature = signature.serialize_compact().to_lower_hex_string();

    (
        StatusCode::OK,
        AppendHeaders([(CONTENT_TYPE, "application/json")]),
        Json(Result::Ok(Data { twap: twap_serialised, signature, pk: state.public_key.to_string() })),
    )
}

async fn health_handler(State(state): State<Arc<ApplicationState>>) -> impl IntoResponse {
    if let ServiceStatus::Failed { message } = state.fetcher_status.read().unwrap().deref() {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            AppendHeaders([(CONTENT_TYPE, "application/json")]),
            Json(Result::Err(message.to_string())),
        );
    }

    if let ServiceStatus::Failed { message } = state.processor_status.read().unwrap().deref() {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            AppendHeaders([(CONTENT_TYPE, "application/json")]),
            Json(Result::Err(message.to_string())),
        );
    }

    (StatusCode::OK, AppendHeaders([(CONTENT_TYPE, "application/json")]), Json(Result::Ok("Good".to_string())))
}

#[tokio::main]
async fn main() {
    let app_state = match ApplicationState::new() {
        Ok(state) => Arc::new(state),
        Err(message) => panic!("{}", message),
    };

    let app = Router::new()
        .route("/data", get(data_handler))
        .route("/health", get(health_handler))
        .with_state(app_state.clone());

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();

    let (tx, rx) = mpsc::unbounded_channel::<Vec<SpotEntryEvent>>();

    let fetching_handle = tokio::spawn(app_state.clone().start_fetcher(tx));
    let processing_handle = tokio::spawn(app_state.clone().start_processor(rx));

    if let Err(z) = axum::serve(listener, app).await {
        panic!("{z}");
    };

    fetching_handle.abort();
    processing_handle.abort();
}
