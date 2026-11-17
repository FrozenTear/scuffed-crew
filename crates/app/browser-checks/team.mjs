// Step 3: a game and a team, so a team chat channel exists. Fake data only.
import { Jar } from './lib.mjs';

const admin = await new Jar().login('TestAdmin');
const g = (await admin.req('POST', '/api/games', { name: 'Overwatch', abbreviation: 'OW' })).j;
const t = (await admin.req('POST', '/api/teams', { name: 'Test Team', game_id: g.id })).j;
await admin.req('GET', `/api/teams/${t.id}/channels`);
