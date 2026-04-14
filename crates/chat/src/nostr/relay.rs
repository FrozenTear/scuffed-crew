//! Server-side WebSocket client for strfry relay communication.
//!
//! Uses tokio-tungstenite for async WebSocket. Handles connection management,
//! NIP-01 message framing, and subscription tracking.

use std::collections::HashMap;
use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use tokio::sync::{mpsc, Mutex, RwLock};
use tokio_tungstenite::tungstenite::Message;

use scuffed_types::nostr::{ClientRelayMessage, NostrEvent, NostrFilter, RelayMessage};

/// Errors from relay operations.
#[derive(Debug, thiserror::Error)]
pub enum RelayError {
    #[error("not connected to relay")]
    NotConnected,
    #[error("connection failed: {0}")]
    ConnectionFailed(String),
    #[error("send failed: {0}")]
    SendFailed(String),
    #[error("relay rejected event: {0}")]
    EventRejected(String),
    #[error("subscription error: {0}")]
    SubscriptionError(String),
    #[error("timeout waiting for relay response")]
    Timeout,
}

/// A received relay response for event publishing.
#[derive(Debug, Clone)]
pub struct PublishResult {
    pub event_id: String,
    pub accepted: bool,
    pub message: String,
}

/// Server-side relay client for communicating with strfry.
///
/// Thread-safe (Arc + Mutex internally). Clone is cheap.
#[derive(Clone)]
pub struct RelayClient {
    url: Arc<String>,
    sender: Arc<Mutex<Option<mpsc::Sender<String>>>>,
    subscriptions: Arc<RwLock<HashMap<String, Vec<NostrFilter>>>>,
    connected: Arc<RwLock<bool>>,
    next_sub_id: Arc<Mutex<u32>>,
}

impl RelayClient {
    /// Create a new relay client for the given URL.
    ///
    /// Does not connect automatically — call `connect()`.
    pub fn new(relay_url: &str) -> Self {
        Self {
            url: Arc::new(relay_url.to_string()),
            sender: Arc::new(Mutex::new(None)),
            subscriptions: Arc::new(RwLock::new(HashMap::new())),
            connected: Arc::new(RwLock::new(false)),
            next_sub_id: Arc::new(Mutex::new(1)),
        }
    }

    /// The relay URL this client connects to.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Whether the client is currently connected.
    pub async fn is_connected(&self) -> bool {
        *self.connected.read().await
    }

    /// Connect to the relay and spawn a read loop.
    ///
    /// Returns a receiver for incoming relay messages. The caller should spawn
    /// a task to process these messages.
    pub async fn connect(&self) -> Result<mpsc::Receiver<RelayMessage>, RelayError> {
        let (ws_stream, _response) =
            tokio_tungstenite::connect_async(self.url.as_str())
                .await
                .map_err(|e| RelayError::ConnectionFailed(e.to_string()))?;

        let (mut ws_sink, mut ws_stream_read) = ws_stream.split();

        // Channel for outbound messages (from our methods to the WebSocket sink)
        let (out_tx, mut out_rx) = mpsc::channel::<String>(256);

        // Channel for inbound parsed relay messages (to the caller)
        let (in_tx, in_rx) = mpsc::channel::<RelayMessage>(256);

        *self.sender.lock().await = Some(out_tx);
        *self.connected.write().await = true;

        let connected = self.connected.clone();

        // Write loop: forward outbound messages to the WebSocket
        tokio::spawn(async move {
            while let Some(msg) = out_rx.recv().await {
                if ws_sink.send(Message::Text(msg.into())).await.is_err() {
                    break;
                }
            }
            let _ = ws_sink.close().await;
        });

        // Read loop: parse incoming relay messages and forward to the channel
        let connected_read = connected.clone();
        tokio::spawn(async move {
            while let Some(Ok(msg)) = ws_stream_read.next().await {
                if let Message::Text(text) = msg {
                    match RelayMessage::from_json(&text) {
                        Ok(relay_msg) => {
                            if in_tx.send(relay_msg).await.is_err() {
                                break;
                            }
                        }
                        Err(e) => {
                            tracing::warn!("Failed to parse relay message: {e}");
                        }
                    }
                }
            }
            *connected_read.write().await = false;
            tracing::info!("Relay read loop ended");
        });

        tracing::info!("Connected to relay: {}", self.url);
        Ok(in_rx)
    }

    /// Disconnect from the relay.
    pub async fn disconnect(&self) {
        // Close all subscriptions first
        let subs: Vec<String> = self.subscriptions.read().await.keys().cloned().collect();
        for sub_id in subs {
            let _ = self.send_message(ClientRelayMessage::Close(sub_id)).await;
        }
        self.subscriptions.write().await.clear();

        // Drop the sender which will cause the write loop to end
        *self.sender.lock().await = None;
        *self.connected.write().await = false;

        tracing::info!("Disconnected from relay: {}", self.url);
    }

    /// Publish an event to the relay.
    pub async fn publish_event(&self, event: NostrEvent) -> Result<(), RelayError> {
        self.send_message(ClientRelayMessage::Event(event)).await
    }

    /// Send a NIP-42 AUTH event.
    pub async fn send_auth(&self, auth_event: NostrEvent) -> Result<(), RelayError> {
        self.send_message(ClientRelayMessage::Auth(auth_event)).await
    }

    /// Create a subscription with the given filters.
    ///
    /// Returns the subscription ID.
    pub async fn subscribe(
        &self,
        prefix: &str,
        filters: Vec<NostrFilter>,
    ) -> Result<String, RelayError> {
        let sub_id = {
            let mut counter = self.next_sub_id.lock().await;
            let id = format!("srv-{prefix}-{counter}");
            *counter += 1;
            id
        };

        self.subscriptions
            .write()
            .await
            .insert(sub_id.clone(), filters.clone());

        self.send_message(ClientRelayMessage::Req {
            subscription_id: sub_id.clone(),
            filters,
        })
        .await?;

        Ok(sub_id)
    }

    /// Close a subscription.
    pub async fn unsubscribe(&self, subscription_id: &str) -> Result<(), RelayError> {
        self.subscriptions.write().await.remove(subscription_id);
        self.send_message(ClientRelayMessage::Close(subscription_id.to_string()))
            .await
    }

    /// Send a raw client relay message.
    async fn send_message(&self, message: ClientRelayMessage) -> Result<(), RelayError> {
        let json = message
            .to_json()
            .map_err(|e| RelayError::SendFailed(e.to_string()))?;

        let sender = self.sender.lock().await;
        let tx = sender.as_ref().ok_or(RelayError::NotConnected)?;

        tx.send(json)
            .await
            .map_err(|e| RelayError::SendFailed(e.to_string()))
    }
}

/// Publish a single event to a relay via a one-shot WebSocket connection.
///
/// Connects, sends the EVENT message, waits for the relay OK response
/// (with a timeout), then disconnects. Suitable for fire-and-forget
/// publishing where maintaining a persistent connection is unnecessary.
pub async fn publish_event_oneshot(
    relay_url: &str,
    event: NostrEvent,
) -> Result<(), RelayError> {
    let (ws_stream, _) = tokio_tungstenite::connect_async(relay_url)
        .await
        .map_err(|e| RelayError::ConnectionFailed(e.to_string()))?;

    let (mut sink, mut stream) = ws_stream.split();

    let msg = ClientRelayMessage::Event(event)
        .to_json()
        .map_err(|e| RelayError::SendFailed(e.to_string()))?;

    sink.send(Message::Text(msg.into()))
        .await
        .map_err(|e| RelayError::SendFailed(e.to_string()))?;

    // Wait for OK response (up to 5 seconds)
    let result = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while let Some(Ok(msg)) = stream.next().await {
            if let Message::Text(text) = msg {
                if let Ok(RelayMessage::Ok { accepted, message, .. }) =
                    RelayMessage::from_json(&text)
                {
                    if !accepted {
                        return Err(RelayError::EventRejected(message));
                    }
                    return Ok(());
                }
            }
        }
        Err(RelayError::NotConnected)
    })
    .await;

    let _ = sink.close().await;

    match result {
        Ok(inner) => inner,
        Err(_) => Err(RelayError::Timeout),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relay_client_creation() {
        let client = RelayClient::new("ws://localhost:7777");
        assert_eq!(client.url(), "ws://localhost:7777");
    }

    #[tokio::test]
    async fn relay_client_not_connected_initially() {
        let client = RelayClient::new("ws://localhost:7777");
        assert!(!client.is_connected().await);
    }

    #[tokio::test]
    async fn publish_without_connect_fails() {
        let client = RelayClient::new("ws://localhost:7777");
        let event = NostrEvent {
            id: "test".into(),
            pubkey: "abc".into(),
            created_at: 0,
            kind: 1,
            tags: vec![],
            content: "hello".into(),
            sig: "sig".into(),
        };
        let result = client.publish_event(event).await;
        assert!(matches!(result, Err(RelayError::NotConnected)));
    }
}
