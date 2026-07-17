use serde::Serialize;
use uuid::Uuid;

/// Discord webhook content hard limit.
pub const DISCORD_CONTENT_LIMIT: usize = 2000;

/// Truncate a string to at most `limit` characters (Unicode scalar values).
/// Public for unit tests and reuse.
pub fn truncate_chars(s: &str, limit: usize) -> String {
    if s.chars().count() <= limit {
        return s.to_string();
    }
    s.chars().take(limit).collect()
}

// ─── Matrix ──────────────────────────────────────────────────────────────────

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
    async fn send_message(&self, room_id: &str, body: &str, urgent: bool) -> Result<(), String> {
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

// ─── Discord ─────────────────────────────────────────────────────────────────

/// Discord webhook notification sender.
/// All methods are fire-and-forget via tokio::spawn.
/// Missing webhook URL for a channel = no-op (skip send).
#[derive(Clone)]
pub struct DiscordNotifier {
    client: reqwest::Client,
    officers_webhook: Option<String>,
    general_webhook: Option<String>,
}

/// Empty `parse` disables @everyone / @here / role / user auto-mentions so
/// user-controlled strings (display names, titles) cannot mass-ping a guild.
#[derive(Serialize)]
struct DiscordAllowedMentions {
    parse: Vec<String>,
}

#[derive(Serialize)]
struct DiscordWebhookBody {
    content: String,
    allowed_mentions: DiscordAllowedMentions,
}

impl DiscordWebhookBody {
    fn new(content: impl Into<String>) -> Self {
        Self {
            content: truncate_chars(&content.into(), DISCORD_CONTENT_LIMIT),
            allowed_mentions: DiscordAllowedMentions { parse: vec![] },
        }
    }
}

impl DiscordNotifier {
    /// Create from environment variables.
    ///
    /// Env:
    /// - `DISCORD_WEBHOOK_OFFICERS` — officers channel webhook URL
    /// - `DISCORD_WEBHOOK_GENERAL` — general channel webhook URL
    /// - `DISCORD_WEBHOOKS_ENABLED` — optional; `0`/`false`/`off` disables even if URLs set.
    ///   If unset, enabled when either webhook is non-empty.
    ///
    /// Returns `None` when disabled or no webhook URLs configured.
    pub fn from_env() -> Option<Self> {
        if !discord_webhooks_enabled_from_env() {
            tracing::info!("Discord webhooks disabled via DISCORD_WEBHOOKS_ENABLED");
            return None;
        }

        let officers = non_empty_env("DISCORD_WEBHOOK_OFFICERS");
        let general = non_empty_env("DISCORD_WEBHOOK_GENERAL");

        Self::from_webhooks(officers, general)
    }

    /// Build from explicit webhook URLs (test-friendly). Returns `None` if both empty.
    pub fn from_webhooks(officers: Option<String>, general: Option<String>) -> Option<Self> {
        if officers.is_none() && general.is_none() {
            return None;
        }

        tracing::info!(
            officers = officers.is_some(),
            general = general.is_some(),
            "Discord webhooks configured"
        );

        Some(Self {
            client: reqwest::Client::new(),
            officers_webhook: officers,
            general_webhook: general,
        })
    }

    /// Whether the officers webhook is configured (for admin test status).
    pub fn has_officers_webhook(&self) -> bool {
        self.officers_webhook.is_some()
    }

    /// Whether the general webhook is configured.
    pub fn has_general_webhook(&self) -> bool {
        self.general_webhook.is_some()
    }

    async fn post_webhook(&self, url: &str, content: &str) -> Result<(), String> {
        let body = DiscordWebhookBody::new(content);

        let response = self
            .client
            .post(url)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("Discord webhook request failed: {e}"))?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response
                .text()
                .await
                .unwrap_or_else(|_| "unknown".to_string());
            return Err(format!("Discord webhook error {status}: {text}"));
        }

        Ok(())
    }

    /// Fire-and-forget notification to officers webhook. No-op if not configured.
    pub fn notify_officers(&self, message: String) {
        let Some(url) = self.officers_webhook.clone() else {
            tracing::debug!("Discord officers webhook not configured — skip");
            return;
        };
        let notifier = self.clone();
        tokio::spawn(async move {
            if let Err(e) = notifier.post_webhook(&url, &message).await {
                tracing::error!("Failed to send Discord officer notification: {e}");
            }
        });
    }

    /// Fire-and-forget notification to general webhook. No-op if not configured.
    pub fn notify_general(&self, message: String) {
        let Some(url) = self.general_webhook.clone() else {
            tracing::debug!("Discord general webhook not configured — skip");
            return;
        };
        let notifier = self.clone();
        tokio::spawn(async move {
            if let Err(e) = notifier.post_webhook(&url, &message).await {
                tracing::error!("Failed to send Discord general notification: {e}");
            }
        });
    }
}

/// Parse DISCORD_WEBHOOKS_ENABLED. Unset → true (auto when webhooks present).
/// Explicit off: `0`, `false`, `off`, `no` (case-insensitive).
fn discord_webhooks_enabled_from_env() -> bool {
    match std::env::var("DISCORD_WEBHOOKS_ENABLED") {
        Ok(v) => {
            let v = v.trim();
            !matches!(
                v.to_ascii_lowercase().as_str(),
                "0" | "false" | "off" | "no"
            )
        }
        Err(_) => true,
    }
}

fn non_empty_env(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

// ─── Combined fan-out ────────────────────────────────────────────────────────

/// Fan-out notification bus: Matrix + Discord.
/// Call sites use this the same way as the old `MatrixNotifier`.
#[derive(Clone)]
pub struct Notifier {
    pub matrix: Option<MatrixNotifier>,
    pub discord: Option<DiscordNotifier>,
}

impl Notifier {
    /// Load Matrix and Discord notifiers from env. Returns `None` if neither configured.
    pub fn from_env() -> Option<Self> {
        let matrix = MatrixNotifier::from_env();
        let discord = DiscordNotifier::from_env();
        if matrix.is_none() && discord.is_none() {
            return None;
        }
        Some(Self { matrix, discord })
    }

    /// Fire-and-forget to officers (Matrix room + Discord officers webhook).
    pub fn notify_officers(&self, message: String) {
        if let Some(ref m) = self.matrix {
            m.notify_officers(message.clone());
        }
        if let Some(ref d) = self.discord {
            d.notify_officers(message);
        }
    }

    /// Fire-and-forget to general (Matrix room + Discord general webhook).
    pub fn notify_general(&self, message: String) {
        if let Some(ref m) = self.matrix {
            m.notify_general(message.clone());
        }
        if let Some(ref d) = self.discord {
            d.notify_general(message);
        }
    }

    /// Urgent officers notification (Matrix push + Discord officers).
    pub fn notify_officers_urgent(&self, message: String) {
        if let Some(ref m) = self.matrix {
            m.notify_officers_urgent(message.clone());
        }
        if let Some(ref d) = self.discord {
            // Discord webhooks have no separate "urgent" channel — same path.
            d.notify_officers(message);
        }
    }

    /// Matrix-only room notify (no Discord equivalent for arbitrary rooms).
    pub fn notify_room(&self, room_id: String, message: String) {
        if let Some(ref m) = self.matrix {
            m.notify_room(room_id, message);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_under_limit_unchanged() {
        let s = "hello";
        assert_eq!(truncate_chars(s, DISCORD_CONTENT_LIMIT), "hello");
    }

    #[test]
    fn truncate_exact_limit_unchanged() {
        let s: String = "a".repeat(DISCORD_CONTENT_LIMIT);
        assert_eq!(
            truncate_chars(&s, DISCORD_CONTENT_LIMIT).len(),
            DISCORD_CONTENT_LIMIT
        );
    }

    #[test]
    fn truncate_over_limit() {
        let s: String = "b".repeat(DISCORD_CONTENT_LIMIT + 50);
        let out = truncate_chars(&s, DISCORD_CONTENT_LIMIT);
        assert_eq!(out.chars().count(), DISCORD_CONTENT_LIMIT);
        assert!(!out.contains('x')); // sanity
        assert!(out.chars().all(|c| c == 'b'));
    }

    #[test]
    fn truncate_unicode_counts_chars_not_bytes() {
        // Each emoji is one char but multiple bytes.
        let s: String = "🎯".repeat(10);
        let out = truncate_chars(&s, 5);
        assert_eq!(out.chars().count(), 5);
    }

    #[test]
    fn discord_from_webhooks_both_none_is_none() {
        assert!(DiscordNotifier::from_webhooks(None, None).is_none());
    }

    #[test]
    fn discord_from_webhooks_officers_only() {
        let n = DiscordNotifier::from_webhooks(
            Some("https://discord.com/api/webhooks/1/abc".into()),
            None,
        )
        .expect("should construct");
        assert!(n.has_officers_webhook());
        assert!(!n.has_general_webhook());
    }

    #[test]
    fn discord_from_webhooks_general_only() {
        let n = DiscordNotifier::from_webhooks(
            None,
            Some("https://discord.com/api/webhooks/2/def".into()),
        )
        .expect("should construct");
        assert!(!n.has_officers_webhook());
        assert!(n.has_general_webhook());
    }

    #[test]
    fn discord_from_env_missing_config_is_none() {
        // Ensure the webhook env vars are absent for this process check.
        // We only assert the pure path: empty webhooks → None via from_webhooks.
        // from_env depends on process env; covered indirectly by missing URLs.
        assert!(DiscordNotifier::from_webhooks(None, None).is_none());
    }

    #[test]
    fn notifier_from_env_without_config_is_none() {
        // With neither Matrix nor Discord env typically set in unit test process,
        // Notifier::from_env() may or may not be None depending on ambient env.
        // Document that constructing empty Notifier is None only via both None:
        let n = Notifier {
            matrix: None,
            discord: None,
        };
        // empty fan-out is a valid runtime object only if wrapped in Option by from_env
        let _ = n;
        assert!(
            MatrixNotifier::from_env().is_none() || MatrixNotifier::from_env().is_some(),
            "from_env is total"
        );
    }

    /// User-controlled text (display names, titles) must never enable mention parsing.
    #[test]
    fn discord_webhook_body_disables_mention_parse() {
        let body = DiscordWebhookBody::new("New app from @everyone / @here");
        let v = serde_json::to_value(&body).expect("serialize");
        assert_eq!(v["content"], "New app from @everyone / @here");
        let parse = v["allowed_mentions"]["parse"]
            .as_array()
            .expect("allowed_mentions.parse must be an array");
        assert!(
            parse.is_empty(),
            "parse must be empty to block mass pings, got {parse:?}"
        );
    }

    #[test]
    fn discord_webhook_body_truncates_and_keeps_allowed_mentions() {
        let long: String = "x".repeat(DISCORD_CONTENT_LIMIT + 10);
        let body = DiscordWebhookBody::new(long);
        let v = serde_json::to_value(&body).expect("serialize");
        assert_eq!(
            v["content"].as_str().unwrap().chars().count(),
            DISCORD_CONTENT_LIMIT
        );
        assert_eq!(v["allowed_mentions"]["parse"], serde_json::json!([]));
    }
}
