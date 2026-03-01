use serde::Serialize;
use uuid::Uuid;

/// Matrix notification sender using the Client-Server API.
/// All methods are fire-and-forget via tokio::spawn.
#[derive(Clone)]
pub struct MatrixNotifier {
    client: reqwest::Client,
    homeserver_url: String,
    access_token: String,
    officer_room_id: String,
    general_room_id: String,
}

#[derive(Serialize)]
struct MatrixMessage {
    msgtype: String,
    body: String,
}

impl MatrixNotifier {
    /// Create from environment variables. Returns None if not configured.
    pub fn from_env() -> Option<Self> {
        let homeserver_url = std::env::var("MATRIX_HOMESERVER_URL").ok()?;
        let access_token = std::env::var("MATRIX_BOT_ACCESS_TOKEN").ok()?;
        let officer_room_id = std::env::var("MATRIX_OFFICER_ROOM_ID").ok()?;
        let general_room_id = std::env::var("MATRIX_GENERAL_ROOM_ID").ok()?;

        if homeserver_url.is_empty()
            || access_token.is_empty()
            || officer_room_id.is_empty()
            || general_room_id.is_empty()
        {
            return None;
        }

        tracing::info!("Matrix notifications configured");
        Some(Self {
            client: reqwest::Client::new(),
            homeserver_url: homeserver_url.trim_end_matches('/').to_string(),
            access_token,
            officer_room_id,
            general_room_id,
        })
    }

    /// Send a message to a Matrix room. Uses m.notice (no push) or m.text (triggers push).
    async fn send_message(
        &self,
        room_id: &str,
        body: &str,
        urgent: bool,
    ) -> Result<(), String> {
        let txn_id = Uuid::new_v4().to_string();
        let msgtype = if urgent { "m.text" } else { "m.notice" };
        let url = format!(
            "{}/_matrix/client/v3/rooms/{}/send/m.room.message/{}",
            self.homeserver_url,
            urlencoding::encode(room_id),
            txn_id,
        );

        let message = MatrixMessage {
            msgtype: msgtype.to_string(),
            body: body.to_string(),
        };

        let response = self
            .client
            .put(&url)
            .header("Authorization", format!("Bearer {}", self.access_token))
            .json(&message)
            .send()
            .await
            .map_err(|e| format!("Matrix request failed: {e}"))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "unknown".to_string());
            return Err(format!("Matrix API error {status}: {body}"));
        }

        Ok(())
    }

    /// Fire-and-forget notification to officers room.
    pub fn notify_officers(&self, message: String) {
        let notifier = self.clone();
        tokio::spawn(async move {
            if let Err(e) = notifier
                .send_message(&notifier.officer_room_id, &message, false)
                .await
            {
                tracing::error!("Failed to send officer notification: {e}");
            }
        });
    }

    /// Fire-and-forget notification to general room.
    pub fn notify_general(&self, message: String) {
        let notifier = self.clone();
        tokio::spawn(async move {
            if let Err(e) = notifier
                .send_message(&notifier.general_room_id, &message, false)
                .await
            {
                tracing::error!("Failed to send general notification: {e}");
            }
        });
    }

    /// Fire-and-forget urgent notification to officers room (triggers push).
    pub fn notify_officers_urgent(&self, message: String) {
        let notifier = self.clone();
        tokio::spawn(async move {
            if let Err(e) = notifier
                .send_message(&notifier.officer_room_id, &message, true)
                .await
            {
                tracing::error!("Failed to send urgent officer notification: {e}");
            }
        });
    }

    /// Send to a specific room (e.g., team room).
    pub fn notify_room(&self, room_id: String, message: String) {
        let notifier = self.clone();
        tokio::spawn(async move {
            if let Err(e) = notifier.send_message(&room_id, &message, false).await {
                tracing::error!("Failed to send room notification: {e}");
            }
        });
    }
}
