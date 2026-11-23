// Step 5: a saved public strategy with a map, so /strategy/editor/:id opens
// with no map picker (the Tab walk from page load runs there). Plus two
// strategies whose stored map id is not the picker's exact id (a display
// name, and an id the picker does not list), so keys.mjs can check that a
// saved map is fixed whatever id is stored. Fake data only.
import { Jar, writeState } from './lib.mjs';

const admin = await new Jar().login('TestAdmin');
const make = async (name, mapId) => {
  const r = await admin.req('POST', '/api/strategy/strategies', { name, map_id: mapId, game_mode: 'escort', visibility: 'public' });
  if (r.status >= 300 || !r.j.id) throw new Error('strategy ' + r.status);
  return r.j.id;
};
writeState('strategy.json', {
  id: await make('Test push on Dorado', 'dorado'),
  byName: await make('Test map stored by name', 'Dorado'),
  unknown: await make('Test map not in the picker', 'not-a-map'),
});
