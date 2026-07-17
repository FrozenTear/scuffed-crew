use dashmap::DashMap;
use scuffed_types::strategy::{CollabUserInfo, ServerMessage, StrategyId, WsResponse};
use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::{Mutex, OwnedSemaphorePermit, Semaphore, mpsc};

/// Hard caps against connection floods (strategy collab WS).
pub const MAX_GLOBAL_CONNECTIONS: usize = 256;
pub const MAX_ROOM_CONNECTIONS: usize = 32;
/// Max concurrent fire-and-forget DB persist tasks across all rooms.
pub const MAX_CONCURRENT_PERSISTS: usize = 64;

/// Why a join was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinError {
    GlobalLimit,
    RoomLimit,
}

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

/// Manages all collaboration rooms with per-room locking via DashMap.
///
/// Also owns:
/// - global / per-room connection caps (DoS)
/// - a semaphore bounding concurrent DB persist tasks
/// - per-strategy mutexes so content RMW is serialized
pub struct RoomManager {
    rooms: DashMap<StrategyId, Room>,
    global_connections: AtomicUsize,
    persist_sem: Arc<Semaphore>,
    strategy_locks: DashMap<StrategyId, Arc<Mutex<()>>>,
}

impl RoomManager {
    pub fn new() -> Self {
        Self {
            rooms: DashMap::new(),
            global_connections: AtomicUsize::new(0),
            persist_sem: Arc::new(Semaphore::new(MAX_CONCURRENT_PERSISTS)),
            strategy_locks: DashMap::new(),
        }
    }

    pub fn global_connection_count(&self) -> usize {
        self.global_connections.load(Ordering::Relaxed)
    }

    /// Join a room (creates it if it doesn't exist).
    /// `connection_id` must be unique per WebSocket.
    ///
    /// Returns `Err` when global or per-room caps would be exceeded — caller
    /// must not treat the connection as joined.
    pub fn join_room(
        &self,
        strategy_id: &StrategyId,
        connection_id: String,
        user: CollabUserInfo,
        tx: mpsc::Sender<WsResponse>,
    ) -> Result<(), JoinError> {
        // Optimistic global reserve; roll back on room-limit reject.
        let prev = self.global_connections.fetch_add(1, Ordering::AcqRel);
        if prev >= MAX_GLOBAL_CONNECTIONS {
            self.global_connections.fetch_sub(1, Ordering::AcqRel);
            return Err(JoinError::GlobalLimit);
        }

        let mut room = self.rooms.entry(strategy_id.clone()).or_default();
        if room.connections.len() >= MAX_ROOM_CONNECTIONS {
            drop(room);
            self.global_connections.fetch_sub(1, Ordering::AcqRel);
            return Err(JoinError::RoomLimit);
        }

        // Only announce UserJoined if this is the user's first connection in the room
        let first_for_user = !room.connections.values().any(|c| c.info.id == user.id);

        if first_for_user {
            room.broadcast_all(ServerMessage::UserJoined { user: user.clone() });
        }

        room.add_connection(connection_id, user, tx);
        Ok(())
    }

    /// Leave a room for one connection. Broadcasts UserLeft only when the
    /// user has no remaining connections in the room.
    pub fn leave_room(&self, strategy_id: &StrategyId, connection_id: &str) {
        let mut removed = false;
        let should_remove = {
            if let Some(mut room) = self.rooms.get_mut(strategy_id) {
                if let Some((info, still_present)) = room.remove_connection(connection_id) {
                    removed = true;
                    if !still_present {
                        room.broadcast_all(ServerMessage::UserLeft { user_id: info.id });
                    }
                }
                room.is_empty()
            } else {
                false
            }
        };

        if removed {
            self.global_connections.fetch_sub(1, Ordering::AcqRel);
        }

        if should_remove {
            self.rooms.remove(strategy_id);
            // Drop idle strategy lock entry to avoid unbounded growth of empty locks
            self.strategy_locks.remove(strategy_id);
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

    /// Per-strategy mutex for serializing content RMW (element/phase patches).
    pub fn strategy_lock(&self, strategy_id: &StrategyId) -> Arc<Mutex<()>> {
        self.strategy_locks
            .entry(strategy_id.clone())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    /// Try to reserve a persist slot. `None` means the server is saturated —
    /// caller should reject the mutation (backpressure) rather than spawn.
    pub fn try_persist_permit(&self) -> Option<OwnedSemaphorePermit> {
        self.persist_sem.clone().try_acquire_owned().ok()
    }

    /// Spawn a strategy content persist task under the global permit + strategy lock.
    ///
    /// Returns `false` if the persist semaphore is saturated (caller should
    /// surface a busy error to the client). The live collab broadcast may still
    /// have already been sent — that is intentional for latency; DB will catch
    /// up on the next successful mutation / full save.
    pub fn try_spawn_persist<F, Fut>(&self, strategy_id: &StrategyId, work: F) -> bool
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let Some(permit) = self.try_persist_permit() else {
            return false;
        };
        let lock = self.strategy_lock(strategy_id);
        tokio::spawn(async move {
            let _permit = permit;
            let _guard = lock.lock().await;
            work().await;
        });
        true
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

#[cfg(test)]
mod tests {
    use super::*;
    use scuffed_types::strategy::CollabUserInfo;

    fn dummy_user(id: &str) -> CollabUserInfo {
        CollabUserInfo {
            id: id.into(),
            username: id.into(),
            avatar_url: None,
        }
    }

    /// Keep the receiver alive so `try_send` does not treat the peer as dead
    /// and prune it during UserJoined broadcast. Buffer must absorb N-1
    /// UserJoined events when filling a room to capacity.
    fn dummy_conn() -> (mpsc::Sender<WsResponse>, mpsc::Receiver<WsResponse>) {
        mpsc::channel(MAX_ROOM_CONNECTIONS + 8)
    }

    #[test]
    fn join_rejects_at_room_limit() {
        let mgr = RoomManager::new();
        let sid = "strat-1".to_string();
        let mut keep_alive = Vec::new();
        for i in 0..MAX_ROOM_CONNECTIONS {
            let (tx, rx) = dummy_conn();
            keep_alive.push(rx);
            mgr.join_room(
                &sid,
                format!("c{i}"),
                dummy_user(&format!("u{i}")),
                tx,
            )
            .expect("join within limit");
        }
        let (tx, rx) = dummy_conn();
        keep_alive.push(rx);
        let err = mgr
            .join_room(&sid, "overflow".into(), dummy_user("overflow"), tx)
            .unwrap_err();
        assert_eq!(err, JoinError::RoomLimit);
        assert_eq!(mgr.global_connection_count(), MAX_ROOM_CONNECTIONS);
        // silence unused warning for last rx
        drop(keep_alive);
    }

    #[test]
    fn leave_decrements_global_count() {
        let mgr = RoomManager::new();
        let sid = "strat-2".to_string();
        let (tx, _rx) = dummy_conn();
        mgr.join_room(&sid, "c1".into(), dummy_user("u1"), tx)
            .unwrap();
        assert_eq!(mgr.global_connection_count(), 1);
        mgr.leave_room(&sid, "c1");
        assert_eq!(mgr.global_connection_count(), 0);
        assert_eq!(mgr.room_count(), 0);
    }

    #[test]
    fn persist_semaphore_bounds_concurrent_tasks() {
        let mgr = RoomManager::new();
        let mut permits = Vec::new();
        for _ in 0..MAX_CONCURRENT_PERSISTS {
            permits.push(mgr.try_persist_permit().expect("permit available"));
        }
        assert!(mgr.try_persist_permit().is_none());
        drop(permits);
        assert!(mgr.try_persist_permit().is_some());
    }
}
