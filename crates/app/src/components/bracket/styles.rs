pub const BRACKET_STYLES: &str = r#"
/* --- Bracket Layout --- */
.bracket-container {
    overflow-x: auto;
    padding: 2rem 0;
}
.bracket {
    display: flex;
    align-items: stretch;
    gap: 0;
    min-width: max-content;
}
.bracket-round {
    display: flex;
    flex-direction: column;
    justify-content: space-around;
    min-width: 220px;
    gap: 1rem;
    padding: 0 0.5rem;
}
.bracket-round-title {
    text-align: center;
    font-family: var(--font-head);
    font-weight: 700;
    font-size: 0.75rem;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-3);
    margin-bottom: 0.5rem;
}

/* --- Match Card --- */
.match-card {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 6px;
    overflow: hidden;
    min-width: 200px;
    transition: border-color 0.2s;
}
.match-card:hover {
    border-color: var(--accent-soft);
}
.match-card.live {
    border-color: var(--ok);
    box-shadow: 0 0 12px color-mix(in srgb, var(--ok) 30%, transparent);
}
.match-card.completed {
    opacity: 0.85;
}
.match-card.bye {
    border-style: dashed;
    opacity: 0.5;
}

.team-slot {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0.4rem 0.6rem;
    gap: 0.5rem;
    transition: opacity 0.2s;
}
.team-slot + .team-slot {
    border-top: 1px solid color-mix(in srgb, var(--text) 6%, transparent);
}
.team-slot.winner {
    background: color-mix(in srgb, var(--accent) 8%, transparent);
}
.team-slot.loser {
    opacity: 0.45;
}
.team-slot-seed {
    font-family: var(--font-mono);
    font-size: 0.65rem;
    color: var(--text-3);
    min-width: 1.5rem;
}
.team-slot-name {
    flex: 1;
    font-size: 0.82rem;
    font-weight: 600;
    color: var(--text);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}
.team-slot-name.tbd {
    font-style: italic;
    color: var(--text-3);
    font-weight: 400;
}
.team-slot-score {
    font-family: var(--font-mono);
    font-size: 0.85rem;
    font-weight: 700;
    min-width: 1.5rem;
    text-align: center;
}
.team-slot.winner .team-slot-score {
    color: var(--accent);
}

.match-card-status {
    text-align: center;
    font-family: var(--font-mono);
    font-size: 0.6rem;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    padding: 0.15rem 0;
    color: var(--text-3);
}
.match-card-status.live {
    color: var(--ok);
}
.match-card-status.live::before {
    content: '';
    display: inline-block;
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--ok);
    margin-right: 0.3rem;
    vertical-align: middle;
    animation: pulse-dot 1.5s ease-in-out infinite;
}
@keyframes pulse-dot {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.3; }
}

/* --- Replay Codes --- */
.match-replay-codes {
    display: flex;
    gap: 0.25rem;
    padding: 0.2rem 0.6rem 0.3rem;
    flex-wrap: wrap;
}
.replay-code {
    font-family: var(--font-mono);
    font-size: 0.6rem;
    color: var(--text-3);
    background: var(--surface-2);
    padding: 0.1rem 0.35rem;
    border-radius: 3px;
    letter-spacing: 0.04em;
    cursor: pointer;
    transition: background 0.15s, color 0.15s;
}
.replay-code:hover {
    background: color-mix(in srgb, var(--accent) 15%, transparent);
    color: var(--text);
}

/* --- Double Elim Layout --- */
.double-elim-container {
    display: flex;
    flex-direction: column;
    gap: 2rem;
}
.double-elim-section-label {
    font-family: var(--font-head);
    font-weight: 700;
    font-size: 0.8rem;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    color: var(--accent);
    padding-bottom: 0.5rem;
    border-bottom: 1px solid var(--border);
    margin-bottom: 0.5rem;
}

/* --- Round Robin Table --- */
.rr-table-container { overflow-x: auto; }
.rr-table { border-collapse: collapse; font-size: 0.8rem; }
.rr-table th, .rr-table td {
    padding: 0.4rem 0.6rem;
    border: 1px solid var(--border);
    text-align: center;
    min-width: 3rem;
}
.rr-table th {
    background: var(--surface-2);
    color: var(--text-3);
    font-family: var(--font-mono);
    font-size: 0.7rem;
    text-transform: uppercase;
}
.rr-table .rr-name {
    text-align: left;
    font-weight: 600;
    color: var(--text);
    white-space: nowrap;
}
.rr-table .rr-self { background: var(--bg); }
.rr-table .rr-win { color: var(--ok); font-weight: 700; }
.rr-table .rr-loss { color: var(--danger); font-weight: 700; }
.rr-table .rr-pending { color: var(--text-3); }
.rr-score-with-codes { border-bottom: 1px dotted var(--text-3); cursor: help; }

/* --- Swiss Standings --- */
.swiss-standings { width: 100%; border-collapse: collapse; font-size: 0.85rem; }
.swiss-standings th {
    text-align: left;
    padding: 0.5rem 0.75rem;
    background: var(--surface-2);
    color: var(--text-3);
    font-size: 0.7rem;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    border-bottom: 1px solid var(--border);
}
.swiss-standings td {
    padding: 0.5rem 0.75rem;
    border-bottom: 1px solid color-mix(in srgb, var(--text) 6%, transparent);
    color: var(--text);
}
.swiss-standings .rank-col { font-family: var(--font-mono); color: var(--text-3); width: 2.5rem; }
.swiss-standings .record-col { font-family: var(--font-mono); font-weight: 700; }

/* --- Mobile Fallback --- */
.bracket-mobile { display: none; }
.bracket-desktop { display: block; }
@media (max-width: 600px) {
    .bracket-mobile { display: block; }
    .bracket-desktop { display: none; }
    .match-card { min-width: 0; }
}
.bracket-mobile-round { margin-bottom: 1.5rem; }
.bracket-mobile-round-title {
    font-family: var(--font-head);
    font-weight: 700;
    font-size: 0.8rem;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-3);
    margin-bottom: 0.5rem;
    padding-bottom: 0.25rem;
    border-bottom: 1px solid var(--border);
}
.bracket-mobile-matches {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
}

/* --- Tournament Pages --- */
.tournaments-page, .tournament-detail-page {
    min-height: 100vh;
    padding: 2rem 2rem 4rem;
    max-width: 1000px;
    margin: 0 auto;
}
.tournaments-page h1, .tournament-detail-page h1 {
    font-family: var(--font-head);
    font-size: 3rem;
    color: var(--text);
    letter-spacing: 3px;
    margin-bottom: 0.5rem;
    text-align: center;
}
.tournaments-page h1::after, .tournament-detail-page h1::after {
    content: '';
    display: block;
    width: 60px;
    height: 3px;
    background: var(--accent);
    margin-top: 0.75rem;
    margin-bottom: 2.5rem;
    border-radius: 2px;
    margin-left: auto;
    margin-right: auto;
}
.tournament-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(320px, 1fr));
    gap: 1rem;
}
.tournament-card {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 1.25rem;
    text-decoration: none;
    color: inherit;
    transition: border-color 0.2s, transform 0.15s;
    display: block;
    text-align: center;
}
.tournament-card:hover {
    border-color: var(--accent-soft);
    transform: translateY(-2px);
}
.tournament-card-name {
    font-family: var(--font-head);
    font-weight: 700;
    font-size: 1.2rem;
    color: var(--text);
    margin-bottom: 0.4rem;
}
.tournament-card-meta {
    display: flex;
    justify-content: center;
    gap: 0.75rem;
    flex-wrap: wrap;
    font-size: 0.78rem;
    color: var(--text-3);
}
.tournament-card-desc {
    color: var(--text-2);
    font-size: 0.82rem;
    margin-top: 0.4rem;
    line-height: 1.5;
}
.tournament-card-status {
    display: inline-block;
    padding: 0.15rem 0.5rem;
    border-radius: var(--radius-pill);
    font-size: 0.65rem;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    margin-top: 0.5rem;
}
/* registration/upcoming → accent (upcoming opportunity) */
.tournament-card-status.registration {
    background: color-mix(in srgb, var(--accent) 20%, transparent);
    color: var(--accent);
}
/* in_progress → ok (active/live) */
.tournament-card-status.in_progress {
    background: color-mix(in srgb, var(--ok) 20%, transparent);
    color: var(--ok);
}
/* completed → text-2 (neutral, past) */
.tournament-card-status.completed {
    background: color-mix(in srgb, var(--text-2) 15%, transparent);
    color: var(--text-2);
}
/* draft → warn (pending/not yet open) */
.tournament-card-status.draft {
    background: color-mix(in srgb, var(--warn) 20%, transparent);
    color: var(--warn);
}

.tournament-header { margin-bottom: 2rem; text-align: center; }
.tournament-meta {
    display: flex;
    justify-content: center;
    gap: 1.5rem;
    flex-wrap: wrap;
    color: var(--text-2);
    font-size: 0.85rem;
    margin-bottom: 1rem;
}
.tournament-meta span { font-family: var(--font-mono); font-size: 0.8rem; }
.tournament-description {
    color: var(--text-2);
    max-width: 700px;
    line-height: 1.7;
    margin: 0 auto 1rem;
    text-align: center;
}
.tournament-external-link {
    display: inline-block;
    color: var(--accent);
    font-size: 0.85rem;
    margin-bottom: 1.5rem;
}
.tournament-home-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(280px, 1fr));
    gap: 1rem;
}
.tournament-section-title {
    font-family: var(--font-head);
    font-weight: 700;
    font-size: 1rem;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-3);
    margin: 2rem 0 1rem;
    padding-bottom: 0.5rem;
    border-bottom: 1px solid var(--border);
    text-align: center;
}
"#;
