use dashmap::DashMap;
use scuffed_types::strategy::{
    CollabUserInfo, ServerMessage, StrategyId, WsResponse,
};
use std::collections::HashMap;
use tokio::sync::mpsc;

/// A collaboration room for a strategy
pub struct Room {
    pub users: HashMap<String, RoomUser>,
}

/// A user in a room
pub struct RoomUser {
    pub info: CollabUserInfo,
    pub tx: mpsc::Sender<WsResponse>,
}

impl Room {
    pub fn new() -> Self {
        Self {
            users: HashMap::new(),
        }
    }

    pub fn add_user(&mut self, user: CollabUserInfo, tx: mpsc::Sender<WsResponse>) {
        self.users.insert(
            user.id.clone(),
            RoomUser { info: user, tx },
        );
    }

    pub fn remove_user(&mut self, user_id: &str) -> Option<CollabUserInfo> {
        self.users.remove(user_id).map(|u| u.info)
    }

    pub fn get_users(&self) -> Vec<CollabUserInfo> {
        self.users.values().map(|u| u.info.clone()).collect()
    }

    pub fn is_empty(&self) -> bool {
        self.users.is_empty()
    }

    /// Broadcast a message to all users except the sender, using try_send
    /// to avoid blocking on slow clients. Dead clients are removed.
    pub fn broadcast(&mut self, sender_id: &str, message: ServerMessage) {
        let response = WsResponse::from(message);
        let mut dead_clients = Vec::new();

        for (user_id, user) in &self.users {
            if user_id != sender_id {
                if user.tx.try_send(response.clone()).is_err() {
                    dead_clients.push(user_id.clone());
                }
            }
        }

        for id in dead_clients {
            tracing::warn!("Removing dead client {} from room", id);
            self.users.remove(&id);
        }
    }

    /// Send a message to all users, using try_send
    pub fn broadcast_all(&mut self, message: ServerMessage) {
        let response = WsResponse::from(message);
        let mut dead_clients = Vec::new();

        for (user_id, user) in &self.users {
            if user.tx.try_send(response.clone()).is_err() {
                dead_clients.push(user_id.clone());
            }
        }

        for id in dead_clients {
            tracing::warn!("Removing dead client {} from room", id);
            self.users.remove(&id);
        }
    }
}

impl Default for Room {
    fn default() -> Self {
        Self::new()
    }
}

/// Manages all collaboration rooms with per-room locking via DashMap
pub struct RoomManager {
    rooms: DashMap<StrategyId, Room>,
}

impl RoomManager {
    pub fn new() -> Self {
        Self {
            rooms: DashMap::new(),
        }
    }

    /// Join a room (creates it if it doesn't exist)
    pub fn join_room(
        &self,
        strategy_id: &StrategyId,
        user: CollabUserInfo,
        tx: mpsc::Sender<WsResponse>,
    ) {
        let mut room = self
            .rooms
            .entry(strategy_id.clone())
            .or_insert_with(Room::new);

        // Notify existing users
        room.broadcast_all(ServerMessage::UserJoined {
            user: user.clone(),
        });

        room.add_user(user, tx);
    }

    /// Leave a room (removes it if empty)
    pub fn leave_room(&self, strategy_id: &StrategyId, user_id: &str) {
        let should_remove = {
            if let Some(mut room) = self.rooms.get_mut(strategy_id) {
                if let Some(_user) = room.remove_user(user_id) {
                    // Notify remaining users
                    room.broadcast_all(ServerMessage::UserLeft {
                        user_id: user_id.to_string(),
                    });
                }
                room.is_empty()
            } else {
                false
            }
        };

        if should_remove {
            self.rooms.remove(strategy_id);
        }
    }

    /// Get users in a room
    pub fn get_room_users(&self, strategy_id: &StrategyId) -> Option<Vec<CollabUserInfo>> {
        self.rooms.get(strategy_id).map(|r| r.get_users())
    }

    /// Broadcast to a room (excludes sender, removes dead clients)
    pub fn broadcast(&self, strategy_id: &StrategyId, sender_id: &str, message: ServerMessage) {
        if let Some(mut room) = self.rooms.get_mut(strategy_id) {
            room.broadcast(sender_id, message);
        }
    }

    /// Get number of rooms
    pub fn room_count(&self) -> usize {
        self.rooms.len()
    }

    /// Get total users across all rooms
    pub fn total_users(&self) -> usize {
        self.rooms.iter().map(|r| r.users.len()).sum()
    }
}

impl Default for RoomManager {
    fn default() -> Self {
        Self::new()
    }
}
