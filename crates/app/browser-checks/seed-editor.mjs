// Step 5: a saved public strategy with a map, so /strategy/editor/:id opens
// with no map picker (the Tab walk from page load runs there). Fake data only.
import { Jar, writeState } from './lib.mjs';

const admin = await new Jar().login('TestAdmin');
const r = await admin.req('POST', '/api/strategy/strategies', { name: 'Test push on Dorado', map_id: 'dorado', game_mode: 'escort', visibility: 'public' });
if (r.status >= 300 || !r.j.id) throw new Error('strategy ' + r.status);
writeState('strategy.json', { id: r.j.id });
