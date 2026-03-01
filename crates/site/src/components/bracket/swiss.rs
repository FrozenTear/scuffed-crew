use leptos::prelude::*;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct SwissStanding {
    pub participant_id: String,
    pub participant_name: String,
    pub wins: u32,
    pub losses: u32,
    pub draws: u32,
    pub game_wins: u32,
    pub game_losses: u32,
    pub buchholz: f64,
    pub rank: u32,
}

/// Swiss format standings table.
#[component]
pub fn SwissStandings(standings: Vec<SwissStanding>) -> impl IntoView {
    view! {
        <table class="swiss-standings">
            <thead>
                <tr>
                    <th>"#"</th>
                    <th>"Team"</th>
                    <th>"W"</th>
                    <th>"L"</th>
                    <th>"D"</th>
                    <th>"GW"</th>
                    <th>"GL"</th>
                    <th>"Buch."</th>
                </tr>
            </thead>
            <tbody>
                {standings.into_iter().map(|s| {
                    view! {
                        <tr>
                            <td class="rank-col">{s.rank}</td>
                            <td>{s.participant_name}</td>
                            <td class="record-col">{s.wins}</td>
                            <td class="record-col">{s.losses}</td>
                            <td class="record-col">{s.draws}</td>
                            <td>{s.game_wins}</td>
                            <td>{s.game_losses}</td>
                            <td>{format!("{:.1}", s.buchholz)}</td>
                        </tr>
                    }
                }).collect_view()}
            </tbody>
        </table>
    }
}
