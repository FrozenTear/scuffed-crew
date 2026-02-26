# SCUFFED CREW

**Gaming Organization**
**Project Brief & Planning Document**
*Draft for Review — February 2026*

---

## About this document

This is a working draft of everything planned so far for Scuffed Crew. Nothing is final.
Looking for honest feedback on the concept, structure, messaging, and technical decisions.
Feel free to challenge anything — that's the whole point of sharing this early.

---

## 1. What Is Scuffed Crew?

Scuffed Crew is a multi-game competitive gaming organization modeled on old-school clan culture — the kind of structured, tight-knit gaming communities that were common in the 2000s and early 2010s but have largely disappeared.

**The core idea:** a small, intentional group of adults who play together on a schedule, know each other by name, and actually care about showing up. Not a 500-member Discord server where nobody knows anyone. A real clan.

### The Problem It Solves

The clan scene is functionally dead. Matchmaking replaced community servers. Forums died. Everything consolidated into Discord, which is great for communication but terrible for the kind of committed community a clan needs. There's no friction to joining or leaving, so people just drift.

Meanwhile, thousands of players are actively looking for teams on sites like LookingForClan, BattleStandards, and TeamTavern — but every listing just funnels into another Discord server. Nobody's offering the old-school package: a proper website, a roster, scheduled play nights, a group that actually feels like a clan.

### What Makes It Different

| Traditional Discord Groups | Scuffed Crew |
|---|---|
| Open invite, anyone joins | Application process, small roster |
| No schedule, play whenever | Scheduled play nights (Tue/Thu/Sat) |
| Ghost members tolerated | Communicate or lose your slot |
| No web presence | Dedicated website with roster, schedule, branding |
| Flat structure | Org hierarchy: Founder > Council > Captains > Members |

---

## 2. Organization Structure

Scuffed Crew operates as an umbrella organization. The org handles branding, recruitment, infrastructure, and cross-team events. Day-to-day play happens within small, game-specific teams of 6–12 members. Your team is your clan experience — the org is the scaffold.

### Hierarchy

| Role | Level | Responsibilities |
|---|---|---|
| Founder | Org | Final say on direction, branding, platform decisions. Sets the culture. |
| Council | Org | Senior leadership across all teams. Org-wide policy, inter-team issues, new team approval. 2–4 people max. |
| Community Lead | Org | Manages public Discord, recruitment pipeline, website. Doesn't need to be a player. |
| Team Captain | Team | Runs their team. Sets schedule, calls strats, manages roster. Seat at org-level decisions. |
| Lieutenant | Team | Captain's backup. Runs sessions when captain can't. Handles onboarding. |
| Member | Team | Active roster. Shows up, communicates absences, participates in scrims. |
| Recruit | Team | 2–4 week trial. Plays with the team. No private channel access until promoted. |

### Team Naming Convention

Each game division names its teams after in-game lore factions. This gives teams identity and ties them to the game they play. **Note:** this becomes relevant when there are multiple teams in the same game. A single team doesn't need a faction name — it's just "the Overwatch team."

Examples for Overwatch:

| Team Name | Lore Reference | Vibe |
|---|---|---|
| Blackwatch | Covert ops division (Reyes, Genji, Moira) | Competitive/tryhard |
| Null Sector | Omnic extremist group | Aggressive comps |
| Deadlock | Ashe's gang, outlaw rebels | Casual/flex |
| Shimada | The Shimada clan, disciplined legacy | Fundamentals, clean play |
| Talon | The big bad of OW lore | Chaos and aggression |

This model extends to other games — Counter-Strike factions, Valorant organizations, Destiny 2 lore groups, etc.

---

## 3. Policies & Culture

### The One Rule

Communicate. That's it. Show up when you can, let your team know when you can't. A quick message in the team chat is all anyone needs. Ghost for weeks without a word and you lose your slot — not as punishment, but because someone else deserves it.

### What We're NOT

Not a job. No activity tracking, no mandatory hours, no attendance spreadsheets. The expectation is adults who understand mutual respect, not employees clocking in. The super-strict clans imploded from burnout. The super-casual ones dissolved from apathy. We're aiming for the middle ground.

### Adults Only (18+)

Not because we're doing anything inappropriate, but because the humor, the schedule expectations, and the social dynamic all assume adults with lives, jobs, and families. Computer time is limited and that's fine.

### No Drama / Zero Tolerance

Competitive banter is fine. Toxicity isn't. If someone is making the team worse to be around, the Captain handles it. If the Captain can't resolve it, Council steps in. Zero tolerance for bigotry, harassment, or targeted abuse.

### Multi-Gaming

Members can be on teams in multiple games. No exclusivity required. Play Overwatch with Blackwatch on Tuesday and Destiny with Dredgen on Thursday.

### Privacy

We don't harvest data or require personal information. Members are never asked to share anything beyond a display name and timezone.

**What we'll never ask for:**
Real name, email address, phone number, social media accounts, physical location, government ID, contacts access, or permission to scan your running processes.

---

## 4. Communication Stack

Two platforms. Discord for text, TeamSpeak for voice.

| Layer | Platform | Who | Purpose |
|---|---|---|---|
| Text | Discord | Everyone | Public: recruitment, announcements, casual chat, LFG. Private team channels: scheduling, strat discussion, coordination (unlocked on roster). |
| Website | Static site (Zola) | Public | Org identity. Roster, schedule, recruitment info, news. |
| Voice | TeamSpeak (self-hosted) | Rostered members | Play night voice comms. Low latency, no process scanning, no telemetry. |

### Why TeamSpeak Instead of Discord Voice?

Discord voice works fine for casual play — DAVE encryption is legit, audited by Trail of Bits. But for competitive play nights we want dedicated voice: self-hosted, lower latency, no process scanning, no accidental screen-share popups mid-match. TeamSpeak has been the standard for competitive teams for over two decades. It does one thing and does it well.

We're waiting on TS6 licensing details before committing, but the plan is to apply for a Non-Profit License (free, up to 512 slots) once the org has an active website and community.

### Future: Encrypted Text (Matrix)

The infrastructure for self-hosted encrypted text chat (Matrix via Continuwuity, using the Cinny client) is built and ready to deploy. This is not part of the launch stack — adding a third platform creates too much friction for new recruits. Once the community is established and people are invested, migrating team coordination to Matrix becomes a natural upgrade rather than an onboarding barrier.

### Technical Note: Hosting

All services (TeamSpeak, and eventually Continuwuity) can run on the same VPS.
Add a reverse proxy (Caddy) for automatic HTTPS, point a subdomain at future services.
Total resource usage stays well within a 2GB VPS.

---

## 5. Recruitment Pipeline

### The Funnel

1. Prospect finds Scuffed Crew via team finder sites, Reddit, word of mouth, or the website.
2. Joins the public Discord. Gets a feel for the vibe. No pressure.
3. Fills out a short application on the website (name, game, role/main, availability, timezone).
4. Gets matched with a team that has an open slot and compatible schedule.
5. Plays 3–5 sessions with the team as a Recruit. No formal tryout — just playing together.
6. Team Captain and members decide if it's a fit.
7. If yes: promoted to Member, gets TeamSpeak access, added to private team channels and website roster.
8. If no: no hard feelings, stays welcome in the public Discord.

### Requirements for Members

- **18+** — adults with lives, humor reflects that
- **Communicate** — let us know if you'll be away
- **No toxicity** — competitive is fine, being a jerk isn't
- **Mic required** for play nights
- **Willing to install TeamSpeak** when you make a roster

---

## 6. Website

The website is the org's identity — what separates "we're a real group" from "we're a Discord invite link." A working HTML prototype already exists.

### Current Sections

| Section | Content |
|---|---|
| Hero / Landing | Clan name, tagline, CTAs for Apply and Meet the Crew. |
| About / Ethos | Four pillars: Squad structure, Compete on FACEIT, Dedicated voice, One rule. |
| Teams | Cards for each active team with roster size, W-L, Elo, division info. |
| FACEIT / Compete | Competitive pipeline from Open to Master division, OWCS qualification path. |
| Comms | Two-layer model: Discord (text, everyone) → TeamSpeak (voice, rostered). Why TeamSpeak explainer. |
| Schedule | Weekly grid: Tue (Comp), Thu (Scrims), Sat (Game Night). |
| Recruitment | Split layout: pitch copy + Apply button, requirements list, "never ask for" box. |
| Footer | "No tracking. No cookies. Just games." |

### Design Direction

Dark theme, red accent (#d63031), display headers (Archivo Black), monospace UI text (DM Mono), clean sans-serif body (Source Sans 3). Noise texture overlay, subtle grid animation on the hero. Modern esports meets old-school clan site — polished but not corporate.

Built as static HTML currently. Production version would use Zola (Rust-based static site generator) for maintainability, hosted on GitHub Pages or similar for zero cost.

---

## 7. Schedule

| Day | Event | Time |
|---|---|---|
| Tuesday | Comp Night | 21:00 CET |
| Thursday | Scrims | 21:00 CET |
| Saturday | Game Night | 20:00 CET |

No obligation to hit every session. The expectation is that most members make 2 out of 3 consistently, and communicate when they can't make it.

---

## 8. Growth Path

| Phase | Milestone | Details |
|---|---|---|
| Phase 1: Foundation | One OW team | Get the website up, comms running, recruit 6–10 solid members. Play together for a month before thinking about expansion. |
| Phase 2: Second Team | 10–20 members | Second OW team or first team in a second game. Promote trusted member to Captain. Start cross-team events. Introduce lore-based team naming. |
| Phase 3: Infrastructure | 20–50 members | Formalize Council. Build out website with team pages, match history, proper application system. Evaluate migrating team text chat to Matrix. |
| Phase 4: Community | 50+ members | Multiple teams across multiple games. Regular org-wide events, internal tournaments. Reputation as a real, structured community. |

---

## 9. Open Questions & Feedback Wanted

These are the things that aren't locked in yet. Would especially appreciate input on:

1. Team naming from game lore — cool identity builder or unnecessarily complex for a small org? (Probably hold off until there are multiple teams in a game.)
2. Website messaging: does the current tone come across as confident or try-hard?
3. Recruitment copy: does "Play 3–5 sessions with us, if we're having fun together you're in" sound welcoming or vague?
4. Schedule: Tue/Thu/Sat realistic for adults with families? Is 21:00 CET late enough for people who work evenings?
5. Org model: should we start as a single clan and scale up, or build the multi-team structure from day one? (Leaning single team first.)
6. Branding: does "Scuffed Crew" read as self-aware and fun, or does it undermine credibility?
7. When is the right time to introduce Matrix for team text chat? Phase 2? Phase 3? Never?

---

## Current Status

- **Website:** Working HTML prototype (v2, updated comms model)
- **Structure doc:** Complete org hierarchy, team naming, policies, recruitment flow
- **Server:** VPS running, TeamSpeak operational, Matrix server ready to deploy when needed
- **Members:** Pre-launch. Building the foundation before recruiting.
- **Timeline:** Launch when TS6 licensing is finalized + infrastructure is tested

---

*End of Draft*
