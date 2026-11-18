// Gives Applicant01 a pending application again after the withdraw check.
import { Jar, rows, adminJar } from './lib.mjs';
const admin = await adminJar(true);
const ow = rows((await admin.req('GET', '/api/games')).j).find(g => g.name === 'Overwatch');
const j = new Jar(false); await j.login('Applicant01');
await j.req('POST', '/api/applications', { preferred_games: [ow.id], preferred_roles: ['Support'], message: 'Test application, support main.' });
await j.req('GET', '/api/applications/mine');
