use std::{collections::HashMap, sync::Arc};

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Extension, Json, Router,
};
use common::{
    model::messages::{CreateGameRequest, CreateGameResponse, GetGameRequest, GetGameResponse, Id},
    utility::{shutdown_signal, Channel},
};
use tokio::sync::{
    broadcast,
    mpsc::{self, Receiver},
    Mutex,
};
use tower_http::trace::TraceLayer;
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::model::internal::{GameRequest, Player};

#[derive(Debug)]
struct Game {
    id: Id,
    players: (Id, Id),
    to_game: mpsc::Sender<GameRequest>,
}

struct GameManagerState {
    pub games: HashMap<Id, Arc<Mutex<Game>>>,
    pub player_assignment: HashMap<Id, Id>,
}
pub struct GameManager {}

impl GameManager {
    pub fn new() -> Self {
        GameManager {}
    }

    pub async fn listen(
        &mut self,
        address: String,
        shutdown_receiver: &mut broadcast::Receiver<()>,
        from_socket: Receiver<GameRequest>,
    ) {
        let state = Arc::new(Mutex::new(GameManagerState {
            games: HashMap::new(),
            player_assignment: HashMap::new(),
        }));
        // Serve REST endpoint
        let rest_shutdown = shutdown_receiver.resubscribe();
        self.serve_rest_endpoint(address, state.clone(), rest_shutdown)
            .await;

        // Spawn main thread to route game messages to game threads
        Self::game_router_thread(state.clone(), shutdown_receiver, from_socket).await;
        // TODO: some sort of collector to cleanup dead games? or threads clean themselves
    }

    // Game logic loop
    async fn game_router_thread(
        state: Arc<Mutex<GameManagerState>>,
        shutdown_receiver: &mut broadcast::Receiver<()>,
        mut from_socket: Receiver<GameRequest>,
    ) {
        loop {
            tokio::select! {
                result = from_socket.recv() => {
                    match result {
                        Some(request) => {
                            Self::route_request(state.clone(), request).await;
                        },
                        _ => {}
                    }
                },
                _ = shutdown_receiver.recv() => {
                    break;
                }
            };
        }
    }

    async fn route_request(state: Arc<Mutex<GameManagerState>>, request: GameRequest) {
        let state = state.lock().await;
        let player_id = request.player.id.clone();
        match state.games.get(&player_id) {
            Some(game) => {
                let game = game.lock().await;
                game.to_game.send(request);
            }
            None => warn!("No game for player {:?}", player_id),
        };
    }

    // Game logic

    // REST functions
    async fn serve_rest_endpoint(
        &self,
        address: String,
        state: Arc<Mutex<GameManagerState>>,
        mut shutdown_receiver: broadcast::Receiver<()>,
    ) {
        let app: Router = Router::new()
            .layer(TraceLayer::new_for_http())
            .route("/", get(Self::root))
            .route("/create_game", post(Self::create_game))
            .route("/game/{game_id}", get(Self::get_game))
            .with_state(state);
        let listener = tokio::net::TcpListener::bind(address.clone())
            .await
            .unwrap();
        info!("Game manager listening on {}", address);
        axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                shutdown_receiver
                    .recv()
                    .await
                    .expect("Failed to receive shutdown signal");
            })
            .await
            .unwrap();
    }

    async fn root() -> &'static str {
        "Hello, World!"
    }

    async fn create_game(
        State(state): State<Arc<Mutex<GameManagerState>>>,
        Json(request): Json<CreateGameRequest>,
    ) -> Response {
        // TODO:
        let mut state = state.lock().await;
        // Unpack player IDs
        let [player_1, player_2] = request.players.as_slice() else {
            panic!("Expected 2 player IDs")
        };
        // Check if players are already in a game
        if state.player_assignment.contains_key(player_1)
            || state.player_assignment.contains_key(player_2)
        {
            return (StatusCode::CONFLICT, "A player is already in a game").into_response();
        }

        // Insert new game
        let id = Id::new();
        let channel = Channel::<GameRequest>::from(mpsc::channel(100)); // TODO:
                                                                        // what's the size here
        let game = Game {
            id: id.clone(),
            players: (player_1.clone(), player_2.clone()),
            to_game: channel.sender,
        };
        state.games.insert(id.clone(), Arc::new(Mutex::new(game)));

        (
            StatusCode::CREATED,
            Json(CreateGameResponse { game_id: id }),
        )
            .into_response()
    }

    async fn get_game(
        Path(game_id): Path<Id>,
        State(state): State<Arc<Mutex<GameManagerState>>>,
    ) -> Response {
        let state = state.lock().await;
        if !state.games.contains_key(&game_id) {
            return StatusCode::NOT_FOUND.into_response();
        }
        let game = state.games.get(&game_id).unwrap().lock().await;

        (
            StatusCode::OK,
            Json(GetGameResponse {
                game_id,
                players: game.players.clone(),
            }),
        )
            .into_response()
    }
}

impl Default for GameManager {
    fn default() -> Self {
        Self::new()
    }
}
