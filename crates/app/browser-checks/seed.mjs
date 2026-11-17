// Step 1 on an empty database: first-run setup (TestAdmin), four sign-ups
// with applications, three accepted, one left pending (Applicant01) and one
// in trial (Recruit01, an OrgRole::Recruit member). Fake data only.
import { Jar, writeState, sleep } from './lib.mjs';

const admin = new Jar();
await admin.req('POST', '/api/auth/setup', { username: 'TestAdmin', password: 'Test-password-12345', org_name: 'Test Crew' });
await admin.req('GET', '/api/auth/me');
const users = {};
for (const name of ['TestOfficer', 'Player01', 'Player02', 'Applicant01', 'Recruit01']) {
  await sleep(2500); // the register endpoint is rate limited
  const j = new Jar(); users[name] = j;
  await j.req('POST', '/api/auth/local/register', { username: name, password: 'Test-password-12345', confirm_min_age: true });
  await j.req('GET', '/api/auth/me');
  const a = await j.req('POST', '/api/applications', { preferred_games: ['overwatch'], preferred_roles: ['support'], message: 'Test application from ' + name });
  j.app = a.j;
}
for (const name of ['TestOfficer', 'Player01', 'Player02']) {
  await admin.req('PATCH', '/api/applications/' + users[name].app.id, { status: 'accepted', review_notes: 'seed' });
}
const trial = await admin.req('PATCH', '/api/applications/' + users.Recruit01.app.id, { status: 'trial', review_notes: 'seed' });
if (trial.status !== 200) throw new Error('Recruit01 trial ' + trial.status);
writeState('members.json', (await admin.req('GET', '/api/members')).j);
