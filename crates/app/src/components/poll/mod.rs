mod poll_card;
mod poll_create;

use chrono::{DateTime, Utc};
use serde::Deserialize;

pub use poll_card::PollCard;
pub use poll_create::PollCreate;

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct PollSummary {
    pub id: String,
    pub title: String,
    pub description: Option<String>,
    pub options: Vec<String>,
    pub close_at: Option<DateTime<Utc>>,
    pub allow_multiple: bool,
    pub created_by: String,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct PollOptionResult {
    pub option_index: u32,
    pub option_text: String,
    pub vote_count: u32,
    pub percentage: f64,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct PollResults {
    pub poll_id: String,
    pub total_votes: u32,
    pub options: Vec<PollOptionResult>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct PollDetailResponse {
    pub poll: PollSummary,
    pub results: PollResults,
    pub viewer_votes: Vec<u32>,
}

pub const POLL_CSS: &str = r#"
    .poll-create {
        background: var(--bg-card);
        border: 1px solid var(--border);
        border-radius: 12px;
        padding: 1.25rem;
        margin-bottom: 1.25rem;
    }
    .poll-create h2 {
        margin: 0 0 0.75rem;
        color: var(--text-bright);
        font-family: 'Rajdhani', sans-serif;
        font-size: 1.1rem;
        font-weight: 700;
    }
    .poll-create-grid {
        display: grid;
        grid-template-columns: 1fr 1fr;
        gap: 0.75rem;
    }
    .poll-create-field {
        display: flex;
        flex-direction: column;
        gap: 0.35rem;
    }
    .poll-create-field.full {
        grid-column: 1 / -1;
    }
    .poll-create-label {
        color: var(--text-muted);
        font-size: 0.72rem;
        text-transform: uppercase;
        letter-spacing: 0.04em;
    }
    .poll-create-input,
    .poll-create-textarea {
        width: 100%;
        background: var(--bg-surface);
        border: 1px solid var(--border);
        border-radius: 8px;
        color: var(--text-bright);
        padding: 0.6rem 0.7rem;
        font-size: 0.85rem;
        font-family: 'Source Sans 3', sans-serif;
    }
    .poll-create-textarea {
        min-height: 88px;
        resize: vertical;
    }
    .poll-create-actions {
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 0.75rem;
        margin-top: 0.9rem;
    }
    .poll-create-check {
        display: inline-flex;
        align-items: center;
        gap: 0.45rem;
        color: var(--text-secondary);
        font-size: 0.8rem;
    }
    .poll-btn {
        border: 1px solid var(--border);
        background: var(--bg-surface);
        color: var(--text-bright);
        border-radius: 8px;
        padding: 0.45rem 0.85rem;
        font-size: 0.8rem;
        font-weight: 600;
        cursor: pointer;
        transition: border-color 0.15s, background 0.15s;
    }
    .poll-btn:hover {
        border-color: var(--accent-soft);
    }
    .poll-btn.primary {
        background: var(--accent);
        border-color: transparent;
        color: #fff;
    }
    .poll-btn.primary:hover {
        filter: brightness(1.08);
    }
    .poll-btn:disabled {
        opacity: 0.6;
        cursor: not-allowed;
    }

    .poll-card {
        background: var(--bg-card);
        border: 1px solid var(--border);
        border-radius: 12px;
        padding: 1rem;
        display: flex;
        flex-direction: column;
        gap: 0.85rem;
    }
    .poll-head {
        display: flex;
        align-items: flex-start;
        justify-content: space-between;
        gap: 0.75rem;
    }
    .poll-title {
        margin: 0;
        color: var(--text-bright);
        font-family: 'Rajdhani', sans-serif;
        font-weight: 700;
        font-size: 1.05rem;
    }
    .poll-desc {
        margin: 0.25rem 0 0;
        color: var(--text-secondary);
        font-size: 0.85rem;
        line-height: 1.55;
    }
    .poll-meta {
        display: inline-flex;
        align-items: center;
        gap: 0.35rem;
        font-size: 0.68rem;
        color: var(--text-muted);
        text-transform: uppercase;
        letter-spacing: 0.05em;
    }
    .poll-badge {
        display: inline-block;
        font-size: 0.64rem;
        font-weight: 700;
        letter-spacing: 0.04em;
        text-transform: uppercase;
        border-radius: 999px;
        padding: 0.16rem 0.58rem;
        background: #3b82f633;
        color: #60a5fa;
        white-space: nowrap;
    }
    .poll-badge.closed {
        background: #ef444433;
        color: #f87171;
    }
    .poll-options {
        display: flex;
        flex-direction: column;
        gap: 0.5rem;
    }
    .poll-option {
        position: relative;
        border: 1px solid var(--border);
        background: var(--bg-surface);
        border-radius: 10px;
        overflow: hidden;
    }
    .poll-option-fill {
        position: absolute;
        inset: 0;
        width: 0%;
        background: linear-gradient(90deg, rgba(59, 130, 246, 0.18), rgba(124, 58, 237, 0.08));
        pointer-events: none;
    }
    .poll-option-btn {
        position: relative;
        z-index: 1;
        width: 100%;
        border: none;
        background: transparent;
        color: var(--text-bright);
        text-align: left;
        cursor: pointer;
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 0.75rem;
        padding: 0.6rem 0.75rem;
    }
    .poll-option-btn[disabled] {
        cursor: not-allowed;
    }
    .poll-option-label {
        font-size: 0.85rem;
        line-height: 1.4;
    }
    .poll-option-stats {
        display: inline-flex;
        align-items: baseline;
        gap: 0.45rem;
        font-family: 'DM Mono', monospace;
        color: var(--text-secondary);
        font-size: 0.72rem;
        white-space: nowrap;
    }
    .poll-option.selected {
        border-color: var(--accent-soft);
    }
    .poll-foot {
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 0.75rem;
        color: var(--text-muted);
        font-size: 0.74rem;
    }
    .poll-total {
        font-family: 'DM Mono', monospace;
        color: var(--text-secondary);
    }

    @media (max-width: 760px) {
        .poll-create-grid {
            grid-template-columns: 1fr;
        }
    }
"#;
