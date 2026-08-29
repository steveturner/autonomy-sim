use std::{net::SocketAddr, sync::Arc, time::Duration};

use anyhow::{Context, Result};
use axum::{
    Json, Router,
    extract::{
        Query, State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use tokio::sync::{Mutex, RwLock, broadcast};
use tower_http::{cors::CorsLayer, trace::TraceLayer};

use crate::{
    Simulation,
    scenario::{ScenarioDescriptor, ScenarioRegistry},
    wire::{HelloEnvelope, HelloPayload, SCHEMA, StateEnvelope},
};

#[derive(Clone, Debug)]
struct ActiveScenario {
    id: String,
    tick_hz: f64,
}

#[derive(Clone)]
pub struct AppState {
    simulation: Arc<Mutex<Simulation>>,
    latest: Arc<RwLock<StateEnvelope>>,
    active: Arc<RwLock<ActiveScenario>>,
    updates: broadcast::Sender<String>,
    scenarios: Arc<Vec<ScenarioDescriptor>>,
    registry: ScenarioRegistry,
}

#[derive(Debug, Default, Deserialize)]
struct ScenarioQuery {
    scenario: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ScenarioSelection {
    id: String,
}

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    message: String,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(serde_json::json!({ "error": self.message })),
        )
            .into_response()
    }
}

pub async fn run(
    mut simulation: Simulation,
    bind: SocketAddr,
    registry: ScenarioRegistry,
) -> Result<()> {
    let initial = simulation.snapshot()?;
    let active = ActiveScenario {
        id: simulation.scenario_name().to_owned(),
        tick_hz: simulation.tick_hz(),
    };
    let scenarios = registry.descriptors()?;
    let (updates, _) = broadcast::channel(64);
    let state = AppState {
        simulation: Arc::new(Mutex::new(simulation)),
        latest: Arc::new(RwLock::new(initial)),
        active: Arc::new(RwLock::new(active)),
        updates,
        scenarios: Arc::new(scenarios),
        registry,
    };

    let producer_state = state.clone();
    tokio::spawn(async move {
        loop {
            let tick_hz = producer_state.active.read().await.tick_hz;
            tokio::time::sleep(Duration::from_secs_f64(1.0 / tick_hz)).await;
            let frame = {
                let mut simulation = producer_state.simulation.lock().await;
                match simulation.tick() {
                    Ok(frame) => frame,
                    Err(error) => {
                        tracing::error!(%error, "simulation tick failed");
                        continue;
                    }
                }
            };
            publish_frame(&producer_state, frame).await;
        }
    });

    let app = Router::new()
        .route("/healthz", get(health))
        .route("/api/v1/snapshot", get(snapshot))
        .route("/api/v1/scenarios", get(list_scenarios))
        .route("/api/v1/scenario", post(switch_scenario))
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

async fn snapshot(
    Query(query): Query<ScenarioQuery>,
    State(state): State<AppState>,
) -> Result<Json<StateEnvelope>, ApiError> {
    select_if_requested(&state, query.scenario.as_deref()).await?;
    Ok(Json(state.latest.read().await.clone()))
}

async fn list_scenarios(State(state): State<AppState>) -> Json<serde_json::Value> {
    let active = state.active.read().await.id.clone();
    Json(serde_json::json!({
        "active": active,
        "scenarios": state.scenarios.as_ref(),
    }))
}

async fn switch_scenario(
    State(state): State<AppState>,
    Json(selection): Json<ScenarioSelection>,
) -> Result<Json<serde_json::Value>, ApiError> {
    select_scenario(&state, &selection.id).await?;
    Ok(Json(serde_json::json!({ "active": selection.id })))
}

async fn stream(
    ws: WebSocketUpgrade,
    Query(query): Query<ScenarioQuery>,
    State(state): State<AppState>,
) -> Result<Response, ApiError> {
    select_if_requested(&state, query.scenario.as_deref()).await?;
    Ok(ws
        .on_upgrade(move |socket| stream_socket(socket, state))
        .into_response())
}

async fn select_if_requested(state: &AppState, scenario: Option<&str>) -> Result<(), ApiError> {
    if let Some(scenario) = scenario {
        select_scenario(state, scenario).await?;
    }
    Ok(())
}

async fn select_scenario(state: &AppState, scenario_id: &str) -> Result<(), ApiError> {
    if state.active.read().await.id == scenario_id {
        return Ok(());
    }
    let config = state.registry.load(scenario_id).map_err(|error| ApiError {
        status: StatusCode::BAD_REQUEST,
        message: error.to_string(),
    })?;
    let mut replacement = Simulation::try_new(&config).map_err(|error| ApiError {
        status: StatusCode::BAD_REQUEST,
        message: error.to_string(),
    })?;
    let initial = replacement.snapshot().map_err(|error| ApiError {
        status: StatusCode::INTERNAL_SERVER_ERROR,
        message: error.to_string(),
    })?;
    let active = ActiveScenario {
        id: replacement.scenario_name().to_owned(),
        tick_hz: replacement.tick_hz(),
    };

    *state.simulation.lock().await = replacement;
    *state.active.write().await = active;
    publish_frame(state, initial).await;
    tracing::info!(scenario = scenario_id, "active scenario switched");
    Ok(())
}

async fn publish_frame(state: &AppState, frame: StateEnvelope) {
    match serde_json::to_string(&frame) {
        Ok(encoded) => {
            *state.latest.write().await = frame;
            let _ = state.updates.send(encoded);
        }
        Err(error) => tracing::error!(%error, "failed to serialize state frame"),
    }
}

async fn stream_socket(socket: WebSocket, state: AppState) {
    let (mut sender, mut receiver) = socket.split();
    let mut updates = state.updates.subscribe();
    let active = state.active.read().await.clone();
    let hello = HelloEnvelope {
        schema: SCHEMA,
        message_type: "hello",
        scenario: active.id.clone(),
        sequence: 0,
        sim_time_s: 0.0,
        payload: HelloPayload {
            scenario: active.id,
            tick_hz: active.tick_hz,
            server: concat!("autonomy-sim/", env!("CARGO_PKG_VERSION")),
        },
    };
    let hello = match serde_json::to_string(&hello) {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn test_state() -> AppState {
        let scenario_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../scenarios");
        let registry = ScenarioRegistry::new(scenario_dir);
        let mut config = registry.load("isr-relay-demo").unwrap();
        config.cot.sink = "disabled".into();
        let mut simulation = Simulation::try_new(&config).unwrap();
        let initial = simulation.snapshot().unwrap();
        let active = ActiveScenario {
            id: simulation.scenario_name().into(),
            tick_hz: simulation.tick_hz(),
        };
        let (updates, _) = broadcast::channel(8);
        AppState {
            simulation: Arc::new(Mutex::new(simulation)),
            latest: Arc::new(RwLock::new(initial)),
            active: Arc::new(RwLock::new(active)),
            updates,
            scenarios: Arc::new(registry.descriptors().unwrap()),
            registry,
        }
    }

    #[tokio::test]
    async fn list_contract_and_query_selection_track_active_scenario() {
        let state = test_state();
        let Json(list) = list_scenarios(State(state.clone())).await;
        assert_eq!(list["active"], "isr-relay-demo");
        assert_eq!(list["scenarios"][0]["id"], "isr-relay-demo");
        assert_eq!(list["scenarios"][0]["name"], "ISR Relay Demo");
        assert_eq!(list["scenarios"][0]["entity_count"], 6);
        assert_eq!(list["scenarios"][0]["default"], true);
        assert_eq!(list["scenarios"][1]["id"], "wildfire-paradise");
        assert_eq!(list["scenarios"][1]["name"], "Wildfire - Paradise");
        assert_eq!(list["scenarios"][1]["entity_count"], 14);
        assert_eq!(list["scenarios"][1]["default"], false);

        let Json(frame) = snapshot(
            Query(ScenarioQuery {
                scenario: Some("wildfire-paradise".into()),
            }),
            State(state.clone()),
        )
        .await
        .unwrap();
        assert_eq!(frame.scenario, "wildfire-paradise");
        assert_eq!(state.active.read().await.id, "wildfire-paradise");

        let Json(list) = list_scenarios(State(state)).await;
        assert_eq!(list["active"], "wildfire-paradise");
    }

    #[tokio::test]
    async fn unknown_selection_fails_without_changing_active_scenario() {
        let state = test_state();
        let error = select_scenario(&state, "not-registered").await.unwrap_err();
        assert_eq!(error.status, StatusCode::BAD_REQUEST);
        assert_eq!(state.active.read().await.id, "isr-relay-demo");
    }
}
