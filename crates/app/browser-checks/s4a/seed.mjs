// Seeds the S4a test stack through the app's own API. Safe to rerun: every
// step checks what is already there. Test data only, fake names.
import fs from 'node:fs';
import { chromium } from 'playwright';
import { B, PW, STATE_DIR, Jar, rows, sleep, ensureEvent } from './lib.mjs';

const notes = [];
const note = (what, r) => { if (r.status >= 300) notes.push(`${what}: ${r.status} ${JSON.stringify(r.j).slice(0, 200)}`); return r; };

// --- accounts -------------------------------------------------------------
const admin = new Jar();
const st = (await admin.req('GET', '/api/auth/setup-status')).j;
if (st && (st.needs_setup || st.setup_required || st.setup_needed)) {
  note('setup', await admin.req('POST', '/api/auth/setup', { username: 'TestAdmin', password: PW, org_name: 'Test Crew' }));
} else {
  const l = await admin.req('POST', '/api/auth/local/login', { username: 'TestAdmin', password: PW });
  if (l.status !== 200) note('setup', await admin.req('POST', '/api/auth/setup', { username: 'TestAdmin', password: PW, org_name: 'Test Crew' }));
}
await admin.req('GET', '/api/auth/me');

// Games first: the Apply form needs at least one game to pick.
let games = rows((await admin.req('GET', '/api/games')).j);
for (const [name, abbreviation] of [['Overwatch', 'OW'], ['Marvel Rivals', 'MR']]) {
  if (!games.some(g => g.name === name)) note('game ' + name, await admin.req('POST', '/api/games', { name, abbreviation }));
}
games = rows((await admin.req('GET', '/api/games')).j);
const ow = games.find(g => g.name === 'Overwatch');

const users = {};
for (const name of ['Player01', 'Player02', 'Applicant01', 'Visitor01']) {
  const j = new Jar(); users[name] = j;
  let r = await j.req('POST', '/api/auth/local/login', { username: name, password: PW });
  if (r.status !== 200) {
    await sleep(2500);
    note('register ' + name, await j.req('POST', '/api/auth/local/register', { username: name, password: PW, confirm_min_age: true }));
  }
  const me = (await j.req('GET', '/api/auth/me')).j;
  if (name === 'Visitor01') continue; // account only, so the Apply form can be shot
  const mine = await j.req('GET', '/api/applications/mine');
  const has = mine.status === 200 && mine.j && (mine.j.id || (mine.j.data && mine.j.data.id));
  if (!has && !(me && me.member && me.member.is_active)) {
    const a = await j.req('POST', '/api/applications', {
      preferred_games: ow ? [ow.id] : [], preferred_roles: [name === 'Player02' ? 'tank' : 'support'],
      message: `Hi, I am ${name}. I play Overwatch most evenings, mostly ${name === 'Player02' ? 'tank' : 'support'}, and I am looking for a steady scrim team. Test application.`,
    });
    if (a.status >= 300 && a.status !== 409) note('apply ' + name, a);
  }
}
const apps = rows((await admin.req('GET', '/api/applications')).j);
const members0 = rows((await admin.req('GET', '/api/members')).j);
for (const a of apps) {
  if (a.status !== 'pending') continue;
  const who = JSON.stringify(a).toLowerCase();
  if (who.includes('applicant01')) continue;
  note('accept ' + a.id, await admin.req('PATCH', '/api/applications/' + a.id, { status: 'accepted', review_notes: 'seed' }));
}
const members = rows((await admin.req('GET', '/api/members')).j);
const pretty = { testadmin: 'TestAdmin', player01: 'Player01', player02: 'Player02' };
for (const m of members) {
  const want = pretty[String(m.display_name).toLowerCase()];
  if (want && want !== 'TestAdmin' && m.org_role === 'recruit') note('role ' + want, await admin.req('PATCH', `/api/members/${m.id}/role`, { role: 'member' }));
  if (want && m.display_name !== want) note('rename ' + want, await admin.req('PUT', `/api/members/${m.id}`, { display_name: want }));
}

// --- an image for the long post (uploaded, so it is same-origin) ------------
let imageUrl = null;
try {
  const cache = STATE_DIR + '/seed-image.json';
  if (fs.existsSync(cache)) {
    const u = JSON.parse(fs.readFileSync(cache, 'utf8')).url;
    if ((await fetch(B + u)).status === 200) imageUrl = u;
  }
  if (!imageUrl) {
    const browser = await chromium.launch({ headless: true });
    const page = await browser.newPage({ viewport: { width: 1200, height: 630 } });
    await page.setContent(`<body style="margin:0;height:100vh;display:grid;place-items:center;background:linear-gradient(135deg,#1b1630,#3a2a7a 55%,#8f73ff);font:700 64px system-ui;color:#fff"><div>Test Crew scrim review<div style="font:400 28px system-ui;opacity:.8;margin-top:12px">Fake image for test data</div></div></body>`);
    const png = await page.screenshot({ type: 'png' });
    await browser.close();
    const fd = new FormData();
    fd.append('file', new Blob([png], { type: 'image/png' }), 'scrim-review.png');
    const up = note('upload image', await admin.req('POST', '/api/upload/image', undefined, fd));
    if (up.status === 200 && up.j.url) { imageUrl = up.j.url; fs.writeFileSync(cache, JSON.stringify({ url: imageUrl })); }
  }
} catch (e) { notes.push('image: ' + e.message); }

// --- blog -------------------------------------------------------------------
const longPost = `Three weeks of scrims are done, and the review notes were getting too long for chat. This post collects what worked, what did not, and what changes before the next block.

## What we looked at

We reviewed 14 maps from the Tuesday and Thursday blocks. Every map got one pass for team fights and one pass for rotations. The full notes are on the [Tank basics wiki page](/wiki/tank-basics), and the outside reading we used is at [example.com/scrim-reviews](https://example.com/scrim-reviews).

${imageUrl ? `![Scoreboard overview from the Thursday review session](${imageUrl} "Thursday review: the last fight on the second point.")\n` : ''}
### Team fights

Most lost fights were lost before they started. The common causes:

- One support was out of position when the fight began
- Ultimates were stacked on a fight that was already won
- Nobody called the target, so damage was split across three heroes
- The tank pushed past a corner the supports could not see

### Rotations

Rotations were better than last block, mainly because Player02 started calling them early. A few maps still went wrong when the team split between high ground and main.

## The plan for the next block

1. Start every scrim with a five minute comp talk
2. One shot caller per fight, named before the round
3. Track ultimates out loud, both ours and theirs
4. Regroup after two deaths, do not trickle
5. Review one lost fight per map, the same evening

> Winning the next fight matters more than winning the argument about the last one.

## Tracking it

Player01 wrote a small note template for the review channel. Copy it and fill it in after each map:

\`\`\`text
map:        King's Row (attack)
fights:     7 won / 4 lost
first pick: ours 6, theirs 5
ults used:  14 (3 wasted on won fights)
one fix:    regroup before the last corner, do not stagger through the choke one at a time
\`\`\`

Inline code such as \`!review start\` works in the bot channel too.

| Map | Fights won | Fights lost | Note |
| --- | --- | --- | --- |
| King's Row | 7 | 4 | Good third point hold |
| Ilios | 5 | 6 | Lost Well twice to boops |
| Dorado | 6 | 3 | Clean rotations |

## Thanks

Thanks to everyone who showed up on time, and to TestAdmin for recording every map. See you on Tuesday.`;

const posts = [
  ['scrim-review-three-weeks', 'Scrim review: what three weeks taught us', 'Notes from 14 scrim maps: lost fights, better rotations and the plan for the next block.', longPost],
  ['season-kickoff', 'Season kickoff: what changes this month', 'New scrim nights, tryouts and review sessions for the month.', '## Welcome back\n\nScrim nights move to Tuesday and Thursday. Tryouts open next week.\n\n- New scrim schedule\n- Review sessions on Sunday\n- Bring a friend night\n\nSee you in the lobby.'],
  ['tryouts-open', 'Tryouts open for the second roster', 'We are adding a second roster. Here is how the tryouts work and what we look for.', '## How it works\n\nTryouts run over two evenings. You play three maps with the current roster and one review session.\n\n## What we look for\n\n- Clear comms\n- Showing up on time\n- Taking feedback well\n\nApply through the site and an officer will get back to you.'],
  ['new-scrim-partners', 'Two new scrim partners this month', 'We have booked regular scrims with two more teams at our level.', 'Starting next week we scrim two new teams, both close to our level.\n\nThe Tuesday block stays as it is. Thursday moves half an hour later to fit their schedule.'],
  ['vod-review-guide', 'How to review your own VODs', 'A short method for reviewing your own games without spending the whole evening on it.', '## Pick one thing\n\nDo not try to fix everything. Pick one habit per session.\n\n## Watch deaths first\n\nFor each death ask: where was I, what did I know, what could I have done.\n\n1. Note the time\n2. Note the cause\n3. Write one fix'],
  ['comms-guidelines', 'Comms guidelines for scrims', 'Short calls, one shot caller, and no backseat coaching in the middle of a fight.', 'Keep calls short. Name the target, name the ultimate, name the plan.\n\n> Say what you see, then what you want.\n\nSave the feedback for after the round.'],
  ['bring-a-friend-night', 'Bring a friend night is back', 'Open lobbies on Saturday. Bring someone who might want to join.', 'Saturday evening we run open lobbies. Anyone can join voice, no application needed.\n\nMembers: bring one friend each. We will mix the teams.'],
  ['support-duo-notes', 'Notes for support duos', 'Positioning, cooldown trading and who heals whom when everything goes wrong.', '## Positioning\n\nStay where both of you can see each other.\n\n## Cooldowns\n\nTrade them, do not stack them. If one of you used the save, the other one holds theirs.'],
  ['map-pool-update', 'Map pool update for the next block', 'Which maps we practise for the next four weeks and why.', 'The next block focuses on four maps:\n\n- King\'s Row\n- Ilios\n- Dorado\n- Lijiang Tower\n\nWe drop the maps we already play well and add the ones we keep losing.'],
  ['tournament-recap', 'Community cup recap', 'We finished third in the community cup. Here is how the day went.', 'Third place out of sixteen teams. We lost the semi final 2 to 1 after a long third map.\n\nPlayer01 and Player02 both played every map. Thank you to everyone who watched.'],
  ['discord-cleanup', 'Voice channel cleanup', 'Fewer channels, clearer names. Here is the new layout.', 'We removed the channels nobody used and renamed the rest.\n\n- Lobby\n- Scrim A\n- Scrim B\n- Review room'],
  ['officer-team-update', 'Officer team update', 'Who does what on the officer team from this month.', 'TestAdmin keeps running the site and the schedule. Reviews move to a rotating pair of members each week.'],
  ['code-of-conduct-refresh', 'Code of conduct refresh', 'A shorter code of conduct with the same rules. Please read it once.', 'The rules did not change, the text did. It is now one page.\n\n1. Respect your team mates\n2. No drama in voice\n3. Tell your squad if you cannot make it'],
  ['welcome-to-test-crew', 'Welcome to Test Crew', 'What this site is for and where to find things.', 'This is the home of Test Crew. You can find the schedule under Events, guides in the Wiki and updates here on the blog.'],
];
const haveArticles = rows((await admin.req('GET', '/api/articles/admin/all?limit=100')).j);
// Oldest first, so the long post ends up newest and on page 1.
for (const [slug, title, summary, content_markdown] of [...posts].reverse()) {
  const cur = haveArticles.find(a => a.slug === slug);
  if (!cur) {
    note('article ' + slug, await admin.req('POST', '/api/articles', { slug, title, content_markdown, summary, cover_image_url: null }));
    note('publish ' + slug, await admin.req('POST', `/api/articles/${slug}/publish`, {}));
    await sleep(1100); // distinct published_at, stable order
  } else if (slug === 'scrim-review-three-weeks') {
    note('update ' + slug, await admin.req('PUT', `/api/articles/${slug}`, { title, content_markdown, summary }));
  }
}

// --- news -------------------------------------------------------------------
const news = [
  ['Scrim nights move to Tuesday and Thursday', 'From next week the scrim blocks are Tuesday and Thursday at 19:30. The Sunday review session stays where it is. Check Events for the full schedule and RSVP so we can plan the lobbies.', true],
  ['Recruitment is open for support and tank', 'We are looking for one main support and one tank for the second roster. If you know someone who fits, send them to the Apply page.', false],
  ['Site maintenance on Sunday morning', 'The site will be down for about ten minutes on Sunday morning while we update the database. Nothing changes for you afterwards.', false],
  ['Community cup sign-up closes Friday', 'If you want to play the community cup, tell an officer before Friday evening. We need five players and one substitute.', false],
];
const haveNews = rows((await admin.req('GET', '/api/announcements')).j);
for (const [title, content, pinned] of [...news].reverse()) {
  if (!haveNews.some(n => n.title === title)) { note('news ' + title, await admin.req('POST', '/api/announcements', { title, content, pinned })); await sleep(1100); }
}

// --- wiki -------------------------------------------------------------------
const tankBasics = `Tank is the role that decides where the fight happens. This page covers the basics we expect from every tank player in Test Crew.

## Your job

You make space, you hold space, and you make the other team look at you. Damage and kills come second.

### Before the fight

- Check where both of your supports are
- Know which cooldowns you have and which ones you are waiting for
- Pick the corner you will fall back to
- Say out loud what you want to do

### During the fight

1. Take the space your team can follow you into
2. Use cover between cooldowns
3. Call the target your damage players can reach
4. Fall back before your resources are gone, not after

## Cooldown tracking

Write it down for a few scrims until it is a habit. A simple format:

\`\`\`text
fight 1: shield broke at 0:42, backed up to choke, won
fight 2: used both cooldowns on the engage, nothing left for their ult, lost
fight 3: held one cooldown for the enemy dive, won
\`\`\`

The short version is \`one out, one held\`.

## Common mistakes

| Mistake | What to do instead |
| --- | --- |
| Pushing past a corner alone | Stop at the corner and wait for one support |
| Using everything on the engage | Keep one defensive cooldown |
| Staying after two deaths | Call the reset and regroup |

> If your supports cannot see you, they cannot help you.

## Hero picks

### Brawl

Good when your team wants to walk in together. Stay close, keep the pace steady.

### Dive

Good when the other team sits far back. Count down the engage so everybody goes at once.

### Poke

Good on long sight lines. Do not take fights you do not need.

## More reading

See the blog post [Scrim review: what three weeks taught us](/blog/scrim-review-three-weeks) and the [Comms guidelines](/wiki/comms-guidelines) page.`;
const wiki = [
  ['tank-basics', 'Tank basics', tankBasics],
  ['comms-guidelines', 'Comms guidelines', '## Short calls\n\nName the target, name the ultimate, name the plan.\n\n## One shot caller\n\nPick the shot caller before the round starts.\n\n- No backseat coaching during a fight\n- Feedback after the round'],
  ['support-positioning', 'Support positioning', '## Stay in sight\n\nStand where your other support can see you.\n\n## Use cover\n\nHeal from behind a corner when you can.'],
  ['scrim-etiquette', 'Scrim etiquette', 'Be on time, be in voice five minutes early, and tell an officer if you cannot make it.\n\n1. Join the lobby\n2. Check your settings\n3. Say good game at the end'],
  ['new-member-guide', 'New member guide', '## First week\n\nSay hello in chat, pick your roles on your profile and RSVP to one scrim night.\n\n## Where things are\n\n- Events: the schedule\n- Wiki: guides\n- Blog: longer updates'],
  ['map-callouts', 'Map callouts', 'Callouts we use for the maps in the current pool.\n\n## King\'s Row\n\n- Hotel\n- Statue\n- Bridge\n\n## Ilios\n\n- Well\n- Lighthouse\n- Ruins'],
];
const p1 = users.Player01;
const haveWiki = rows((await admin.req('GET', '/api/wiki?limit=100')).j);
for (const [topic, title, content_markdown] of wiki) {
  if (!haveWiki.some(w => w.topic === topic)) note('wiki ' + topic, await (topic === 'tank-basics' ? admin : p1).req('POST', '/api/wiki', { topic, title, content_markdown }));
}

// --- patch notes --------------------------------------------------------------
const ch = (ability, description, change_type) => ({ ability, description, change_type });
const hero = (hero_id, hero_name, change_type, changes, dev_comment = null) => ({ hero_id, hero_name, change_type, changes, dev_comment });
const patches = [
  { version: '9.9.3', date: '2026-11-10', title: 'Test balance patch', url: 'https://example.com/patch-notes/9-9-3',
    hero_updates: [
      hero('ana', 'Ana', 'buff', [ch('Biotic Rifle', 'Healing per shot increased from 75 to 80.', 'buff'), ch('Sleep Dart', 'Cooldown reduced from 14 to 13 seconds.', 'buff')], 'Ana has been falling behind other supports at long range. Test data only.'),
      hero('reinhardt', 'Reinhardt', 'nerf', [ch('Barrier Field', 'Barrier health reduced from 1500 to 1400.', 'nerf'), ch('Charge', 'Cooldown increased from 7 to 8 seconds.', 'nerf')]),
      hero('tracer', 'Tracer', 'adjustment', [ch('Pulse Pistols', 'Damage falloff range increased from 12 to 13 meters.', 'buff'), ch('Recall', 'Cooldown increased from 12 to 13 seconds.', 'nerf'), ch(null, 'Base health changed from 175 to 150 with 25 shields.', 'adjustment')], 'A small shift from survivability to range.'),
      hero('mercy', 'Mercy', 'bugfix', [ch('Guardian Angel', 'Fixed a test bug where the cooldown started late.', 'bugfix')]),
      hero('winston', 'Winston', 'buff', [ch('Tesla Cannon', 'Secondary fire charge time reduced from 1.2 to 1 second.', 'buff')]),
      hero('sombra', 'Sombra', 'nerf', [ch('Hack', 'Ability lockout duration reduced from 1.5 to 1.25 seconds.', 'nerf')]),
    ],
    sections: [
      { category: 'General', items: ['Competitive rank resets apply at the start of the test season.', 'Added a new option to hide the kill feed in replays.'] },
      { category: 'Bug Fixes', items: ['Fixed a test bug with spawn doors.', 'Fixed a test bug with the scoreboard.', 'Fixed a test bug where the payload could stop moving on Dorado.'] },
      { category: 'Maps', items: ['Ilios Well: adjusted cover near the point.'] },
    ] },
  { version: '9.9.2', date: '2026-10-21', title: 'Test hotfix', url: 'https://example.com/patch-notes/9-9-2',
    hero_updates: [
      hero('kiriko', 'Kiriko', 'nerf', [ch('Protection Suzu', 'Cooldown increased from 15 to 16 seconds.', 'nerf')]),
      hero('zarya', 'Zarya', 'buff', [ch('Particle Barrier', 'Duration increased from 2 to 2.25 seconds.', 'buff')], 'Test data only.'),
    ],
    sections: [{ category: 'Bug Fixes', items: ['Fixed a test crash when opening the hero gallery.'] }] },
  { version: '9.9.1', date: '2026-09-30', title: 'Test season launch', url: 'https://example.com/patch-notes/9-9-1',
    hero_updates: [
      hero('genji', 'Genji', 'adjustment', [ch('Shuriken', 'Damage reduced from 27 to 26.', 'nerf'), ch('Deflect', 'Duration increased from 2 to 2.25 seconds.', 'buff')]),
      hero('lucio', 'Lucio', 'bugfix', [ch('Wall Ride', 'Fixed a test bug that kept the speed bonus after landing.', 'bugfix')]),
    ],
    sections: [
      { category: 'General', items: ['New test season starts.', 'Updated the practice range layout.'] },
      { category: 'Competitive', items: ['Placement matches reduced from 10 to 5.'] },
      { category: 'Bug Fixes', items: ['Fixed a test bug with voice lines in the hero select screen.'] },
    ] },
  { version: '9.8.9', date: '2025-12-09', title: 'Winter test patch', url: 'https://example.com/patch-notes/9-8-9',
    hero_updates: [hero('mei', 'Mei', 'buff', [ch('Ice Wall', 'Cooldown reduced from 12 to 11 seconds.', 'buff')])],
    sections: [{ category: 'General', items: ['Winter test event begins.'] }] },
];
const havePatches = rows((await admin.req('GET', '/api/strategy/patch-notes')).j);
for (const p of patches) {
  if (!havePatches.some(h => h.version === p.version)) note('patch ' + p.version, await admin.req('POST', '/api/strategy/patch-notes', p));
}

// --- home page copy (defaults say "Your Clan"; the frames use a centred hero) ----
{
  const cur = (await admin.req('GET', '/api/settings')).j;
  const hp = cur.homepage || {};
  if (hp.hero_title !== 'Play' || hp.content_align !== 'center') {
    const homepage = { ...hp, content_align: 'center', hero_badge: 'Community · all skill levels', hero_title: 'Play', hero_title_accent: 'together',
      hero_sub: 'Regular Overwatch scrims without the grind. Pull up a chair.', cta_primary: 'Join us', cta_secondary: 'Meet the teams',
      ethos_body: 'We play to improve and to have a good evening. Show up, talk, and take feedback well.', seeking_tags: ['Tank', 'Support'] };
    note('settings homepage', await admin.req('PUT', '/api/settings', { homepage }));
  }
}

// --- one event ----------------------------------------------------------------
await ensureEvent(admin);

// --- summary ------------------------------------------------------------------
const q = new Jar(true);
const count = async path => rows((await q.req('GET', path)).j).length;
const summary = {
  members: rows((await admin.req('GET', '/api/members')).j).map(m => `${m.display_name}:${m.org_role}`),
  applications: rows((await admin.req('GET', '/api/applications')).j).map(a => a.status),
  games: games.map(g => g.name),
  articles_public: await count('/api/articles?limit=100'),
  announcements: await count('/api/announcements'),
  wiki: await count('/api/wiki?limit=100'),
  patch_notes: await count('/api/strategy/patch-notes'),
  events_public: await count('/api/events'),
  image: imageUrl,
  problems: notes,
};
fs.writeFileSync(STATE_DIR + '/seed.json', JSON.stringify(summary, null, 1));
console.log('\nSEED SUMMARY\n' + JSON.stringify(summary, null, 1));
