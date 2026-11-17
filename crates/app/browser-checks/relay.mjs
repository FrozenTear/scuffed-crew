// Step 4: two extra relay URLs (on .invalid hosts) so /admin/relay shows both
// tables with rows. Pair with NOSTR_RELAY_URL on a dead local port.
import { Jar } from './lib.mjs';

const admin = await new Jar().login('TestAdmin');
const s = (await admin.req('GET', '/api/settings')).j;
s.extra_relay_urls = 'wss://relay-one.example.invalid/a-long-path-for-width-testing\nwss://relay-two.example.invalid';
await admin.req('PUT', '/api/settings', s);
await admin.req('GET', '/api/nostr/health');
