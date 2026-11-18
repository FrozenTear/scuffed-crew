// Second seed for the fix pass: one team with a roster and one tournament in
// registration, so Home shows the Teams table, the third stat tile and the
// Tournaments panel. Safe to rerun. Test data only, fake names.
import { adminJar, rows } from './lib.mjs';
const admin = await adminJar(false);
const games = rows((await admin.req('GET', '/api/games')).j);
const ow = games.find(g => g.name === 'Overwatch');
let teams = rows((await admin.req('GET', '/api/teams')).j);
for (const [name, division] of [['Test Squad A', 'Division 1'], ['Test Squad B', 'Division 3']]) {
  if (!teams.some(t => t.name === name)) await admin.req('POST', '/api/teams', { name, game_id: ow.id, division });
}
teams = rows((await admin.req('GET', '/api/teams')).j);
const members = rows((await admin.req('GET', '/api/members')).j);
const byName = n => members.find(m => String(m.display_name).toLowerCase() === n.toLowerCase());
const a = teams.find(t => t.name === 'Test Squad A');
if (a) {
  const roster = rows((await admin.req('GET', `/api/teams/${a.id}/roster`)).j);
  for (const [n, team_role] of [['Player01', 'captain'], ['Player02', 'player']]) {
    const m = byName(n); if (!m) continue;
    if (!JSON.stringify(roster).includes(m.id)) await admin.req('POST', `/api/teams/${a.id}/roster`, { member_id: m.id, team_role });
  }
}
const tours = rows((await admin.req('GET', '/api/tournaments')).j);
if (!tours.some(t => t.name === 'Test Cup')) {
  const r = await admin.req('POST', '/api/tournaments', { name: 'Test Cup', format: 'single_elim', game_id: ow.id, max_teams: 8, is_open: true });
  const id = r.j && (r.j.id || (r.j.data && r.j.data.id));
  if (id) await admin.req('PATCH', `/api/tournaments/${id}/status`, { status: 'registration' });
}
console.log('overview', JSON.stringify((await admin.req('GET', '/api/public/overview')).j).slice(0, 400));
