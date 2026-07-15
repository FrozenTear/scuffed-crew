use dashmap::DashMap;
use scuffed_types::strategy::{CollabUserInfo, ServerMessage, StrategyId, WsResponse};
use std::collections::HashMap;
use tokio::sync::mpsc;

/// A collaboration room for a strategy
pub struct Room {
    /// Keyed by **connection id** (not user id) so multi-tab works.
    pub connections: HashMap<String, RoomUser>,
}

/// A single WebSocket connection in a room
pub struct RoomUser {
    pub info: CollabUserInfo,
    pub tx: mpsc::Sender<WsResponse>,
}

impl Room {
    pub fn new() -> Self {
        Self {
            connections: HashMap::new(),
        }
    }

    pub fn add_connection(
        &mut self,
        connection_id: String,
        user: CollabUserInfo,
        tx: mpsc::Sender<WsResponse>,
    ) {
        self.connections
            .insert(connection_id, RoomUser { info: user, tx });
    }

    /// Remove one connection. Returns the user info and whether any other
    /// connections for that **user id** remain.
    pub fn remove_connection(&mut self, connection_id: &str) -> Option<(CollabUserInfo, bool)> {
        let removed = self.connections.remove(connection_id)?;
        let user_id = removed.info.id.clone();
        let still_present = self.connections.values().any(|c| c.info.id == user_id);
        Some((removed.info, still_present))
    }

    /// Deduplicated user list for RoomJoined UI (one entry per user id).
    pub fn get_users(&self) -> Vec<CollabUserInfo> {
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        for c in self.connections.values() {
            if seen.insert(c.info.id.clone()) {
                out.push(c.info.clone());
            }
        }
        out
    }

    pub fn is_empty(&self) -> bool {
        self.connections.is_empty()
    }

    /// Broadcast to all connections except `sender_connection_id`.
    pub fn broadcast(&mut self, sender_connection_id: &str, message: ServerMessage) {
        let response = WsResponse::from(message);
        let mut dead = Vec::new();

        for (cid, user) in &self.connections {
            if cid != sender_connection_id && user.tx.try_send(response.clone()).is_err() {
                dead.push(cid.clone());
            }
        }

        for id in dead {
            tracing::warn!("Removing dead connection {} from room", id);
            self.connections.remove(&id);
        }
    }

    pub fn broadcast_all(&mut self, message: ServerMessage) {
        let response = WsResponse::from(message);
        let mut dead = Vec::new();

        for (cid, user) in &self.connections {
            if user.tx.try_send(response.clone()).is_err() {
                dead.push(cid.clone());
            }
        }

        for id in dead {
            tracing::warn!("Removing dead connection {} from room", id);
            self.connections.remove(&id);
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

    /// Join a room (creates it if it doesn't exist).
    /// `connection_id` must be unique per WebSocket.
    pub fn join_room(
        &self,
        strategy_id: &StrategyId,
        connection_id: String,
        user: CollabUserInfo,
        tx: mpsc::Sender<WsResponse>,
    ) {
        let mut room = self.rooms.entry(strategy_id.clone()).or_default();

        // Only announce UserJoined if this is the user's first connection in the room
        let first_for_user = !room.connections.values().any(|c| c.info.id == user.id);

        if first_for_user {
            room.broadcast_all(ServerMessage::UserJoined { user: user.clone() });
        }

        room.add_connection(connection_id, user, tx);
    }

    /// Leave a room for one connection. Broadcasts UserLeft only when the
    /// user has no remaining connections in the room.
    pub fn leave_room(&self, strategy_id: &StrategyId, connection_id: &str) {
        let should_remove = {
            if let Some(mut room) = self.rooms.get_mut(strategy_id) {
                if let Some((info, still_present)) = room.remove_connection(connection_id)
                    && !still_present
                {
                    room.broadcast_all(ServerMessage::UserLeft { user_id: info.id });
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

    pub fn get_room_users(&self, strategy_id: &StrategyId) -> Option<Vec<CollabUserInfo>> {
        self.rooms.get(strategy_id).map(|r| r.get_users())
    }

    pub fn broadcast(
        &self,
        strategy_id: &StrategyId,
        sender_connection_id: &str,
        message: ServerMessage,
    ) {
        if let Some(mut room) = self.rooms.get_mut(strategy_id) {
            room.broadcast(sender_connection_id, message);
        }
    }

    #[allow(dead_code)]
    pub fn room_count(&self) -> usize {
        self.rooms.len()
    }

    #[allow(dead_code)]
    pub fn total_users(&self) -> usize {
        self.rooms.iter().map(|r| r.connections.len()).sum()
    }
}

impl Default for RoomManager {
    fn default() -> Self {
        Self::new()
    }
}
