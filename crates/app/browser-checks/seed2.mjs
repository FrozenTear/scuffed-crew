// Step 2: roles and display names, a blog post, a patch note, a forum board
// and thread. Fake data only.
import { Jar, writeState } from './lib.mjs';

const admin = await new Jar().login('TestAdmin');
const members = (await admin.req('GET', '/api/members')).j.data;
const names = { testadmin: ['TestAdmin', null], testofficer: ['TestOfficer', 'officer'], player01: ['Player01', 'member'], player02: ['Player02', 'member'], recruit01: ['Recruit01', null] };
for (const m of members) {
  const n = names[m.display_name]; if (!n) continue;
  if (n[1]) await admin.req('PATCH', `/api/members/${m.id}/role`, { role: n[1] });
  await admin.req('PUT', `/api/members/${m.id}`, { display_name: n[0] });
}
await admin.req('POST', '/api/articles', { slug: 'season-kickoff', title: 'Season kickoff: what changes this month', content_markdown: '## Welcome back\n\nScrim nights move to Tuesday and Thursday. Tryouts open next week.\n\n- New scrim schedule\n- Review sessions on Sunday\n- Bring a friend night\n\nSee you in the lobby.', summary: 'New scrim nights, tryouts and review sessions for the month.', cover_image_url: null });
await admin.req('POST', '/api/articles/season-kickoff/publish', {});
await admin.req('POST', '/api/strategy/patch-notes', { version: '9.9.1', date: '2026-11-10', title: 'Test balance patch', url: 'https://example.com/patch-notes/9-9-1', hero_updates: [{ hero_id: 'ana', hero_name: 'Ana', change_type: 'buff', changes: [{ ability: 'Biotic Rifle', description: 'Healing per shot increased from 75 to 80.', change_type: 'buff' }], dev_comment: 'Test data only.' }, { hero_id: 'reinhardt', hero_name: 'Reinhardt', change_type: 'nerf', changes: [{ ability: 'Barrier Field', description: 'Barrier health reduced from 1500 to 1400.', change_type: 'nerf' }], dev_comment: null }], sections: [{ category: 'Bug Fixes', items: ['Fixed a test bug with spawn doors.', 'Fixed a test bug with the scoreboard.'] }] });
const cat = (await admin.req('POST', '/api/forum/categories', { name: 'General', slug: 'general', description: 'Everything else', sort_order: 0 })).j;
const board = (await admin.req('POST', '/api/forum/boards', { category_id: cat.id, name: 'Team talk', slug: 'team-talk', description: 'Comps, scrims and reviews', sort_order: 0 })).j;
const p1 = await new Jar().login('Player01');
const thread = (await p1.req('POST', '/api/forum/threads', { title: 'Best comp for the next scrim?', content: 'We lost the last two maps on defense. Should we try a dive comp or stay with brawl? Post your ideas here.', board_id: board.id })).j;
writeState('seed.json', { thread, board, cat });
