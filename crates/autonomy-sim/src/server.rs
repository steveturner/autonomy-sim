use std::{net::SocketAddr, sync::Arc, time::Duration};

use anyhow::{Context, Result};
use axum::{
    Json, Router,
    extract::{
        State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    response::IntoResponse,
    routing::get,
};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::{RwLock, broadcast};
use tower_http::{cors::CorsLayer, trace::TraceLayer};

use crate::{
    Simulation,
    wire::{HelloEnvelope, HelloPayload, SCHEMA, StateEnvelope},
};

#[derive(Clone)]
pub struct AppState {
    latest: Arc<RwLock<StateEnvelope>>,
    updates: broadcast::Sender<String>,
    hello: HelloEnvelope,
}

pub async fn run(mut simulation: Simulation, bind: SocketAddr) -> Result<()> {
    let initial = simulation.snapshot();
    let tick_hz = simulation.tick_hz();
    let hello = HelloEnvelope {
        schema: SCHEMA,
        message_type: "hello",
        sequence: 0,
        sim_time_s: 0.0,
        payload: HelloPayload {
            scenario: simulation.scenario_name().to_owned(),
            tick_hz,
            server: concat!("autonomy-sim/", env!("CARGO_PKG_VERSION")),
        },
    };
    let (updates, _) = broadcast::channel(64);
    let state = AppState {
        latest: Arc::new(RwLock::new(initial)),
        updates,
        hello,
    };

    let producer_state = state.clone();
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs_f64(1.0 / tick_hz));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            ticker.tick().await;
            let frame = simulation.tick();
            let encoded = match serde_json::to_string(&frame) {
                Ok(encoded) => encoded,
                Err(error) => {
                    tracing::error!(%error, "failed to serialize state frame");
                    continue;
                }
            };
            *producer_state.latest.write().await = frame;
            let _ = producer_state.updates.send(encoded);
        }
    });

    let app = Router::new()
        .route("/healthz", get(health))
        .route("/api/v1/snapshot", get(snapshot))
        .route("/api/v1/stream", get(stream))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state);
    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .with_context(|| format!("binding API to {bind}"))?;
    tracing::info!(%bind, "state API listening");
    axum::serve(listener, app).await.context("serving API")
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "status": "ok" }))
}

async fn snapshot(State(state): State<AppState>) -> Json<StateEnvelope> {
    Json(state.latest.read().await.clone())
}

async fn stream(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| stream_socket(socket, state))
}

async fn stream_socket(socket: WebSocket, state: AppState) {
    let (mut sender, mut receiver) = socket.split();
    let hello = match serde_json::to_string(&state.hello) {
        Ok(value) => value,
        Err(_) => return,
    };
    let latest = match serde_json::to_string(&*state.latest.read().await) {
        Ok(value) => value,
        Err(_) => return,
    };
    if sender.send(Message::Text(hello.into())).await.is_err()
        || sender.send(Message::Text(latest.into())).await.is_err()
    {
        return;
    }

    let mut updates = state.updates.subscribe();
    loop {
        tokio::select! {
            update = updates.recv() => match update {
                Ok(value) => if sender.send(Message::Text(value.into())).await.is_err() { break; },
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            },
            message = receiver.next() => match message {
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                _ => {}
            }
        }
    }
}
