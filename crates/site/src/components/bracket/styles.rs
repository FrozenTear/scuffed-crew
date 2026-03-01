pub const BRACKET_STYLES: &str = r#"
/* ─── Bracket Layout ─── */
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
    font-family: 'Rajdhani', sans-serif;
    font-weight: 700;
    font-size: 0.75rem;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-muted);
    margin-bottom: 0.5rem;
}

/* ─── Connector Lines ─── */
.bracket-connector {
    display: flex;
    flex-direction: column;
    justify-content: space-around;
    width: 24px;
    position: relative;
}
.bracket-connector-pair {
    flex: 1;
    display: flex;
    flex-direction: column;
    justify-content: center;
    position: relative;
}
.bracket-connector-pair::before,
.bracket-connector-pair::after {
    content: '';
    position: absolute;
    right: 0;
    width: 12px;
    border-right: 2px solid var(--border);
}
.bracket-connector-pair::before {
    top: 25%;
    height: 25%;
    border-top: 2px solid var(--border);
}
.bracket-connector-pair::after {
    bottom: 25%;
    height: 25%;
    border-bottom: 2px solid var(--border);
}
.bracket-connector-line {
    position: absolute;
    left: 0;
    top: 50%;
    width: 12px;
    border-top: 2px solid var(--border);
}

/* ─── Match Card ─── */
.match-card {
    background: var(--bg-card);
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
    border-color: var(--accent);
    box-shadow: 0 0 12px var(--accent-glow);
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
    border-top: 1px solid var(--border-light, rgba(255,255,255,0.06));
}
.team-slot.winner {
    background: rgba(124, 58, 237, 0.08);
}
.team-slot.loser {
    opacity: 0.45;
}
.team-slot-seed {
    font-family: 'DM Mono', monospace;
    font-size: 0.65rem;
    color: var(--text-muted);
    min-width: 1.5rem;
}
.team-slot-name {
    flex: 1;
    font-size: 0.82rem;
    font-weight: 600;
    color: var(--text-primary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}
.team-slot-name.tbd {
    font-style: italic;
    color: var(--text-muted);
    font-weight: 400;
}
.team-slot-score {
    font-family: 'DM Mono', monospace;
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
    font-family: 'DM Mono', monospace;
    font-size: 0.6rem;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    padding: 0.15rem 0;
    color: var(--text-muted);
}
.match-card-status.live {
    color: var(--accent);
}
.match-card-status.live::before {
    content: '';
    display: inline-block;
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--accent);
    margin-right: 0.3rem;
    vertical-align: middle;
    animation: pulse-dot 1.5s ease-in-out infinite;
}
@keyframes pulse-dot {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.3; }
}

/* ─── Double Elim Layout ─── */
.double-elim-container {
    display: flex;
    flex-direction: column;
    gap: 2rem;
}
.double-elim-section-label {
    font-family: 'Rajdhani', sans-serif;
    font-weight: 700;
    font-size: 0.8rem;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    color: var(--accent);
    padding-bottom: 0.5rem;
    border-bottom: 1px solid var(--border);
    margin-bottom: 0.5rem;
}

/* ─── Round Robin Table ─── */
.rr-table-container {
    overflow-x: auto;
}
.rr-table {
    border-collapse: collapse;
    font-size: 0.8rem;
}
.rr-table th, .rr-table td {
    padding: 0.4rem 0.6rem;
    border: 1px solid var(--border);
    text-align: center;
    min-width: 3rem;
}
.rr-table th {
    background: var(--bg-elevated);
    color: var(--text-muted);
    font-family: 'DM Mono', monospace;
    font-size: 0.7rem;
    text-transform: uppercase;
}
.rr-table .rr-name {
    text-align: left;
    font-weight: 600;
    color: var(--text-primary);
    white-space: nowrap;
}
.rr-table .rr-self {
    background: var(--bg-surface);
}
.rr-table .rr-win {
    color: var(--green);
    font-weight: 700;
}
.rr-table .rr-loss {
    color: var(--red);
    font-weight: 700;
}
.rr-table .rr-pending {
    color: var(--text-muted);
}

/* ─── Swiss Standings ─── */
.swiss-standings {
    width: 100%;
    border-collapse: collapse;
    font-size: 0.85rem;
}
.swiss-standings th {
    text-align: left;
    padding: 0.5rem 0.75rem;
    background: var(--bg-elevated);
    color: var(--text-muted);
    font-size: 0.7rem;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    border-bottom: 1px solid var(--border);
}
.swiss-standings td {
    padding: 0.5rem 0.75rem;
    border-bottom: 1px solid var(--border-light, rgba(255,255,255,0.06));
    color: var(--text-primary);
}
.swiss-standings .rank-col {
    font-family: 'DM Mono', monospace;
    color: var(--text-muted);
    width: 2.5rem;
}
.swiss-standings .record-col {
    font-family: 'DM Mono', monospace;
    font-weight: 700;
}

/* ─── Mobile Fallback ─── */
.bracket-mobile {
    display: none;
}
.bracket-desktop {
    display: block;
}

@media (max-width: 600px) {
    .bracket-mobile {
        display: block;
    }
    .bracket-desktop {
        display: none;
    }
    .match-card {
        min-width: 0;
    }
}

.bracket-mobile-round {
    margin-bottom: 1.5rem;
}
.bracket-mobile-round-title {
    font-family: 'Rajdhani', sans-serif;
    font-weight: 700;
    font-size: 0.8rem;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-muted);
    margin-bottom: 0.5rem;
    padding-bottom: 0.25rem;
    border-bottom: 1px solid var(--border);
}
.bracket-mobile-matches {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
}

/* ─── Tournament Pages ─── */
.tournaments-page, .tournament-detail-page {
    min-height: 100vh;
    padding: 6rem 2rem 4rem;
    max-width: 1000px;
    margin: 0 auto;
}
.tournaments-page h1, .tournament-detail-page h1 {
    font-family: 'Bebas Neue', sans-serif;
    font-size: 3rem;
    color: var(--text-bright);
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
    background: var(--bg-card);
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
    font-family: 'Rajdhani', sans-serif;
    font-weight: 700;
    font-size: 1.2rem;
    color: var(--text-bright);
    margin-bottom: 0.4rem;
}
.tournament-card-meta {
    display: flex;
    justify-content: center;
    gap: 0.75rem;
    flex-wrap: wrap;
    font-size: 0.78rem;
    color: var(--text-muted);
}
.tournament-card-meta span {
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
}
.tournament-card-desc {
    color: var(--text-secondary);
    font-size: 0.82rem;
    margin-top: 0.4rem;
    line-height: 1.5;
}
.tournament-card-status {
    display: inline-block;
    padding: 0.15rem 0.5rem;
    border-radius: 999px;
    font-size: 0.65rem;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    margin-top: 0.5rem;
}
.tournament-card-status.registration { background: #7c3aed33; color: #a78bfa; }
.tournament-card-status.in_progress { background: #10b98133; color: #34d399; }
.tournament-card-status.completed { background: #6b728033; color: #9ca3af; }
.tournament-card-status.draft { background: #f59e0b33; color: #fbbf24; }

.tournament-header {
    margin-bottom: 2rem;
    text-align: center;
}
.tournament-header h1 {
    margin-bottom: 0.5rem;
}
.tournament-meta {
    display: flex;
    justify-content: center;
    gap: 1.5rem;
    flex-wrap: wrap;
    color: var(--text-secondary);
    font-size: 0.85rem;
    margin-bottom: 1rem;
}
.tournament-meta span {
    font-family: 'DM Mono', monospace;
    font-size: 0.8rem;
}
.tournament-description {
    color: var(--text-secondary);
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
    font-family: 'Rajdhani', sans-serif;
    font-weight: 700;
    font-size: 1rem;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-muted);
    margin: 2rem 0 1rem;
    padding-bottom: 0.5rem;
    border-bottom: 1px solid var(--border);
    text-align: center;
}

/* Empty-state / loading text on tournament pages */
.tournament-detail-page > p,
.tournaments-page > p {
    text-align: center;
    color: var(--text-muted);
    padding: 1rem 0;
}
"#;
