# External Nostr Clients — Connection Guide

Our clan chat runs on Nostr (NIP-29 relay-based groups). You can use external Nostr clients alongside the built-in web widget for full chat capabilities.

## Recommended Clients

### Flotilla (Web) — Primary

Discord-like interface built for NIP-29 groups. Best for desktop/browser use.

**Setup:**
1. Go to [flotilla.social](https://flotilla.social)
2. Log in with your Nostr key (NIP-07 extension like nos2x or Alby)
3. Click **Add Relay** and enter our relay URL (see [Our Relay](#our-relay) below for how to find it)
4. Our team channels will appear in the sidebar

### 0xChat (Mobile) — Primary

Best mobile Nostr client with full NIP-29 group support, push notifications, and NIP-44 encryption.

**Setup:**
1. Install 0xChat from [App Store](https://apps.apple.com/app/0xchat/id1675309861) or [Google Play](https://play.google.com/store/apps/details?id=com.oxchat.nostr)
2. Import your key or create a new one
3. Go to **Settings > Relays > Add Relay** and enter our relay URL (see [Our Relay](#our-relay) below)
4. Navigate to **Groups** to see our team channels

### Other Compatible Clients

| Client | Platform | Notes |
|--------|----------|-------|
| Groups (groups.nip29.com) | Web | Dedicated NIP-29 interface, good browser fallback |
| Chachi (chachi.chat) | Web | Lightweight, fast — still WIP |
| Amethyst | Android | NIP-29 supported but groups UX is buried |

## Linking Your Existing Nostr Key

If you already have a Nostr identity, link it to your clan account.

Most members start with a server-managed key, and `/identity` offers **Link & Verify** only when no key is linked and a NIP-07 extension is installed. So with a server-managed key, unlink it first:

1. Install a NIP-07 browser extension (nos2x, Alby, or similar)
2. Go to `/identity` on the clan site
3. If the page shows a server-managed key, back it up first if you want to keep it (see below). Then open **Advanced**, click **Unlink key** and confirm. Chat and direct messages stop working until a key is linked again
4. Under **Link a key**, click **Detect** to read your public key from the extension (or paste it)
5. Click **Link & Verify** and approve the signing request in your extension
6. Your Nostr pubkey is now linked, and you're the same identity everywhere

## Backing Up Your Server-Managed Key

If you started with a server-managed key, you can export an encrypted backup of it:

1. Go to `/identity` on the clan site
2. Under **Back up your key**, enter a backup password (12 characters or more)
3. Click **Export Backup**
4. Copy the backup (`ncryptsec1...`) right away. It is shown only once
5. Store the backup and its password securely. You need both to use the key

The backup is your key locked with that password (NIP-49). It is not a plain `nsec`.

Exporting does not change your key mode. Your key stays server-managed, and the server keeps signing for you on the clan site.

To use the key in an external client, import the `ncryptsec` there if the client supports NIP-49 backups, and enter the backup password when asked.

## Our Relay

- **URL:** the relay this instance advertises. Fetch
  <https://ow.scuffedcrew.no/.well-known/nostr.json?name=YOURNAME> and read the
  `relays` entry for your pubkey — that is the authoritative value, and it
  follows the deploy's `NOSTR_RELAY_URL` setting. Ask an officer if unsure.
  (Earlier revisions of this guide named `wss://relay.scuffed.gg`, a domain the
  org does **not** control — do not use it.)
- **Auth:** NIP-42 (your key must be registered with the clan)
- **Groups:** NIP-29 (team channels are auto-provisioned)
- **Encryption:** Officer channels use NIP-44 encrypted events
