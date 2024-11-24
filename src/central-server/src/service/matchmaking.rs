use std::{
    collections::{HashSet, VecDeque},
    sync::Arc,
};

use tokio::sync::{
    broadcast,
    mpsc::{Receiver, Sender},
    Mutex,
};
use tracing::{error, info, warn};

use crate::model::messages::{
    MatchDetails, MatchmakingRequest, MatchmakingResponse, Player, UserId,
};

// Structure to represent a player in queue
#[derive(Debug)]
struct Game {
    pub id: String,
    pub player1: Player,
    pub player2: Player,
}

pub struct MatchmakingService {
    queue: VecDeque<Player>,
    users_in_queue: HashSet<UserId>,
    games: Vec<Game>,
}

impl MatchmakingService {
    async fn read_queue(&mut self) {
        info!("Reading queue");
        // BUG: need to lock queue, couldn't we be inserting into it?
        let mut unmatched_players: VecDeque<Player> = VecDeque::new();
        let mut matches: Vec<(Player, Player)> = vec![];
        while let Some(player) = self.queue.pop_front() {
            if let Some(enemy) = unmatched_players.pop_front() {
                info!("Matched {:?} and {:?}", player, enemy);
                matches.push((player, enemy));
                continue;
            }
            unmatched_players.push_back(player);
        }
        for (player1, player2) in matches.iter() {
            self.users_in_queue.remove(&player1.id);
            self.users_in_queue.remove(&player2.id);
            let game = Game {
                id: format!("Game: {:?} created", self.games.len()),
                player1: player1.clone(),
                player2: player2.clone(),
            };
            let _ = player1
                .sender
                .send(MatchmakingResponse::MatchFound(MatchDetails {
                    server: game.id.clone(),
                }))
                .await;
            let _ = player2
                .sender
                .send(MatchmakingResponse::MatchFound(MatchDetails {
                    server: game.id.clone(),
                }))
                .await;
            self.games.push(game);
        }
        self.queue = unmatched_players;
    }

    pub fn new() -> Self {
        Self {
            queue: VecDeque::new(),
            users_in_queue: HashSet::new(),
            games: Vec::new(),
        }
    }
    pub fn add_user(&mut self, player: Player) -> Result<(), &'static str> {
        let user_id = player.clone().id;
        if !self.users_in_queue.contains(&user_id) {
            info!("Adding user {:?} to queue", user_id);
            self.queue.push_back(player);
            self.users_in_queue.insert(user_id);
            Ok(())
        } else {
            Err("User already in queue")
        }
    }

    pub async fn listen(
        &mut self,
        shutdown_receiver: &mut broadcast::Receiver<()>,
        ws_sender: Sender<MatchmakingResponse>,
        ws_receiver: Arc<Mutex<Receiver<MatchmakingRequest>>>,
    ) {
        let mut receiver = ws_receiver.lock().await;
        let mut matchmaking_interval = tokio::time::interval(tokio::time::Duration::from_secs(5));

        info!("Initialized matchmaking service");
        loop {
            // Listen for queue messages / shutdown signal
            tokio::select! {
                _ = shutdown_receiver.recv() => {
                    break
                }
                message = receiver.recv() => {
                    self.handle_message(message).await;
                }
                _ = matchmaking_interval.tick() => {
                    self.read_queue().await;
                }
            }
        }
        info!("Exiting matchmaking service");
    }

    async fn handle_message(&mut self, message: Option<MatchmakingRequest>) {
        let Some(message) = message else {
            info!("Got empty message");
            return;
        };
        match message {
            MatchmakingRequest::JoinQueue(player) => {
                let sender = player.sender.clone();
                if sender.is_closed() {
                    warn!("Sender {:?} is closed!", player.id);
                }
                let _ = self.add_user(player);
                let result = sender.send(MatchmakingResponse::QueueJoined).await;
                if let Err(err) = result {
                    error!("Got error when sending MatchmakingResponse: {}", err);
                }
            }
            MatchmakingRequest::LeaveQueue(user_id) => {
                match self.users_in_queue.get(&user_id) {
                    Some(_) => {
                        let position = self
                            .queue
                            .iter()
                            .enumerate()
                            .find(|(_, user)| user.id == user_id);
                        if let Some((position, user)) = position {
                            info!("Removing user {:?} from queue", user.id);
                            self.queue.remove(position);
                        } else {
                            warn!(
                                "User {:?} was in users_in_queue but not in actual queue",
                                user_id
                            );
                        }
                    }
                    None => warn!("User {:?} not in queue", user_id),
                }
                if self.users_in_queue.contains(&user_id) {}
            }
            MatchmakingRequest::Disconnected(user_id) => {
                warn!("User {:?} disconnected. What to do..?", user_id);
            }
        };
    }
}

impl Default for MatchmakingService {
    fn default() -> Self {
        Self::new()
    }
}
