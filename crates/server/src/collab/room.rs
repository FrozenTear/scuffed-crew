use dashmap::DashMap;
use scuffed_types::strategy::{CollabUserInfo, ServerMessage, StrategyId, WsResponse};
use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::{Mutex, Notify, OwnedSemaphorePermit, Semaphore, mpsc};

/// Joined-room cap. Matches the default `WS_MAX_CONNECTIONS` semaphore, which
/// is the admission limit for every socket (joined or not). `RoomManager::with_global_limit`
/// uses the configured value so the two caps stay aligned.
pub const MAX_GLOBAL_CONNECTIONS: usize = 512;
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
    /// Signalled when a broadcast drops this connection (its channel was full
    /// or closed). The socket task then closes the socket, so the client
    /// reconnects and gets a fresh `RoomJoined` instead of silently missing
    /// updates.
    pub evict: Arc<Notify>,
}

/// A connection a broadcast dropped because its channel was full or closed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pruned {
    pub user_id: String,
    /// No other connection of this user is left in the room.
    pub user_gone: bool,
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
        evict: Arc<Notify>,
    ) {
        self.connections.insert(
            connection_id,
            RoomUser {
                info: user,
                tx,
                evict,
            },
        );
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

    /// Send to every connection except `skip`, dropping any whose channel is
    /// full or closed (each dropped one is told through its `evict`).
    ///
    /// Returns the connections it dropped. The room manager owns the global
    /// connection count and decrements it once per entry
    /// (`RoomManager::deliver`).
    #[must_use]
    pub fn broadcast(&mut self, skip: Option<&str>, message: ServerMessage) -> Vec<Pruned> {
        let response = WsResponse::from(message);
        let mut dead = Vec::new();

        for (cid, user) in &self.connections {
            if Some(cid.as_str()) != skip && user.tx.try_send(response.clone()).is_err() {
                dead.push(cid.clone());
            }
        }

        let mut removed = Vec::with_capacity(dead.len());
        for id in dead {
            tracing::warn!("Removing dead connection {} from room", id);
            if let Some(user) = self.connections.remove(&id) {
                user.evict.notify_one();
                removed.push(user.info.id);
            }
        }
        removed
            .into_iter()
            .map(|user_id| Pruned {
                user_gone: !self.connections.values().any(|c| c.info.id == user_id),
                user_id,
            })
            .collect()
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
    max_global: usize,
    persist_sem: Arc<Semaphore>,
    strategy_locks: DashMap<StrategyId, Arc<Mutex<()>>>,
}

impl RoomManager {
    pub fn new() -> Self {
        Self::with_global_limit(MAX_GLOBAL_CONNECTIONS)
    }

    /// `max_global` is the joined-connection cap. Pass the same number as
    /// `WS_MAX_CONNECTIONS` so a socket that already holds an admission permit
    /// is not rejected by a lower hard-coded ceiling.
    pub fn with_global_limit(max_global: usize) -> Self {
        Self {
            rooms: DashMap::new(),
            global_connections: AtomicUsize::new(0),
            max_global: max_global.max(1),
            persist_sem: Arc::new(Semaphore::new(MAX_CONCURRENT_PERSISTS)),
            strategy_locks: DashMap::new(),
        }
    }

    /// Joined strategy connections. Sockets that have not joined are counted
    /// by the WebSocket admission semaphore instead.
    #[allow(dead_code)]
    pub fn global_connection_count(&self) -> usize {
        self.global_connections.load(Ordering::Relaxed)
    }

    /// Join a room (creates it if it doesn't exist).
    /// `connection_id` must be unique per WebSocket.
    ///
    /// Returns `Err` when global or per-room caps would be exceeded — caller
    /// must not treat the connection as joined.
    ///
    /// `evict` is signalled if a later broadcast drops this connection.
    pub fn join_room(
        &self,
        strategy_id: &StrategyId,
        connection_id: String,
        user: CollabUserInfo,
        tx: mpsc::Sender<WsResponse>,
        evict: Arc<Notify>,
    ) -> Result<(), JoinError> {
        // Optimistic global reserve; roll back on room-limit reject.
        let prev = self.global_connections.fetch_add(1, Ordering::AcqRel);
        if prev >= self.max_global {
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
            self.deliver(
                &mut room,
                None,
                ServerMessage::UserJoined { user: user.clone() },
            );
        }

        room.add_connection(connection_id, user, tx, evict);
        Ok(())
    }

    /// Send `message` to a room and settle every connection the send drops:
    /// one global-count decrement each, and `UserLeft` for a user with no
    /// connection left (which may drop more connections, handled the same way).
    fn deliver(&self, room: &mut Room, skip: Option<&str>, message: ServerMessage) {
        let mut queue = VecDeque::from([(skip.map(str::to_owned), message)]);
        while let Some((skip, message)) = queue.pop_front() {
            let pruned = room.broadcast(skip.as_deref(), message);
            if pruned.is_empty() {
                continue;
            }
            self.global_connections
                .fetch_sub(pruned.len(), Ordering::AcqRel);
            for p in pruned.into_iter().filter(|p| p.user_gone) {
                queue.push_back((None, ServerMessage::UserLeft { user_id: p.user_id }));
            }
        }
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
                        self.deliver(
                            &mut room,
                            None,
                            ServerMessage::UserLeft { user_id: info.id },
                        );
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
            self.deliver(&mut room, Some(sender_connection_id), message);
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

    fn evict() -> Arc<Notify> {
        Arc::new(Notify::new())
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
                evict(),
            )
            .expect("join within limit");
        }
        let (tx, rx) = dummy_conn();
        keep_alive.push(rx);
        let err = mgr
            .join_room(&sid, "overflow".into(), dummy_user("overflow"), tx, evict())
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
        mgr.join_room(&sid, "c1".into(), dummy_user("u1"), tx, evict())
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

    /// API-1: a broadcast that drops a slow client must give its slot back.
    /// The old code removed it from the room without touching the global
    /// count, and the later `leave_room` found nothing to remove, so every
    /// slow client leaked one of the 512 joined slots for good.
    #[tokio::test]
    async fn pruned_slow_client_returns_its_global_slot() {
        let mgr = RoomManager::new();
        let sid = "strat-slow".to_string();
        let (fast_tx, _fast_rx) = dummy_conn();
        mgr.join_room(&sid, "fast".into(), dummy_user("fast"), fast_tx, evict())
            .unwrap();
        // One-slot channel that nobody drains: the first message fills it.
        let (slow_tx, _slow_rx) = mpsc::channel(1);
        let slow_evict = evict();
        mgr.join_room(
            &sid,
            "slow".into(),
            dummy_user("slow"),
            slow_tx,
            slow_evict.clone(),
        )
        .unwrap();
        assert_eq!(mgr.global_connection_count(), 2);

        mgr.broadcast(&sid, "fast", ServerMessage::Pong);
        assert_eq!(mgr.global_connection_count(), 2, "first send fits");
        mgr.broadcast(&sid, "fast", ServerMessage::Pong);
        assert_eq!(
            mgr.global_connection_count(),
            1,
            "the full channel is dropped and its slot returned"
        );
        tokio::time::timeout(std::time::Duration::from_secs(1), slow_evict.notified())
            .await
            .expect("dropped client is told to close");

        // The socket task still calls leave_room on disconnect; it must not
        // decrement a second time.
        mgr.leave_room(&sid, "slow");
        assert_eq!(mgr.global_connection_count(), 1);
        mgr.leave_room(&sid, "fast");
        assert_eq!(mgr.global_connection_count(), 0);
        assert_eq!(mgr.room_count(), 0);
    }

    /// A closed channel (socket task gone) is pruned the same way, from the
    /// `UserJoined` broadcast inside `join_room` too.
    #[test]
    fn closed_client_pruned_on_join_returns_its_slot() {
        let mgr = RoomManager::new();
        let sid = "strat-closed".to_string();
        let (dead_tx, dead_rx) = dummy_conn();
        mgr.join_room(&sid, "dead".into(), dummy_user("dead"), dead_tx, evict())
            .unwrap();
        drop(dead_rx);

        let (tx, mut rx) = dummy_conn();
        mgr.join_room(&sid, "live".into(), dummy_user("live"), tx, evict())
            .unwrap();
        assert_eq!(mgr.global_connection_count(), 1);
        assert_eq!(mgr.total_users(), 1);
        assert!(
            rx.try_recv().is_err(),
            "joiner gets no stale UserLeft for itself"
        );

        mgr.leave_room(&sid, "dead");
        mgr.leave_room(&sid, "live");
        assert_eq!(mgr.global_connection_count(), 0);
        assert_eq!(mgr.room_count(), 0);
    }

    /// Others are told when a pruned user has no connection left.
    #[test]
    fn pruned_user_is_announced_as_left() {
        let mgr = RoomManager::new();
        let sid = "strat-left".to_string();
        let (watch_tx, mut watch_rx) = dummy_conn();
        mgr.join_room(&sid, "watch".into(), dummy_user("watch"), watch_tx, evict())
            .unwrap();
        let (gone_tx, gone_rx) = dummy_conn();
        mgr.join_room(&sid, "gone".into(), dummy_user("gone"), gone_tx, evict())
            .unwrap();
        drop(gone_rx);
        while watch_rx.try_recv().is_ok() {}

        mgr.broadcast(&sid, "nobody", ServerMessage::Pong);
        assert_eq!(mgr.global_connection_count(), 1);
        let mut saw_left = false;
        while let Ok(msg) = watch_rx.try_recv() {
            if let ServerMessage::UserLeft { user_id } = msg.message {
                assert_eq!(user_id, "gone");
                saw_left = true;
            }
        }
        assert!(saw_left, "watcher must get UserLeft for the pruned user");
    }
}
