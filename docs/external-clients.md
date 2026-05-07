# External Nostr Clients — Connection Guide

Our clan chat runs on Nostr (NIP-29 relay-based groups). You can use external Nostr clients alongside the built-in web widget for full chat capabilities.

## Recommended Clients

### Flotilla (Web) — Primary

Discord-like interface built for NIP-29 groups. Best for desktop/browser use.

**Setup:**
1. Go to [flotilla.social](https://flotilla.social)
2. Log in with your Nostr key (NIP-07 extension like nos2x or Alby)
3. Click **Add Relay** and enter our relay URL: `wss://relay.scuffed.gg`
4. Our team channels will appear in the sidebar

### 0xChat (Mobile) — Primary

Best mobile Nostr client with full NIP-29 group support, push notifications, and NIP-44 encryption.

**Setup:**
1. Install 0xChat from [App Store](https://apps.apple.com/app/0xchat/id1675309861) or [Google Play](https://play.google.com/store/apps/details?id=com.oxchat.nostr)
2. Import your key or create a new one
3. Go to **Settings > Relays > Add Relay** and enter: `wss://relay.scuffed.gg`
4. Navigate to **Groups** to see our team channels

### Other Compatible Clients

| Client | Platform | Notes |
|--------|----------|-------|
| Groups (groups.nip29.com) | Web | Dedicated NIP-29 interface, good browser fallback |
| Chachi (chachi.chat) | Web | Lightweight, fast — still WIP |
| Amethyst | Android | NIP-29 supported but groups UX is buried |

## Linking Your Existing Nostr Key

If you already have a Nostr identity, link it to your clan account:

1. Install a NIP-07 browser extension (nos2x, Alby, or similar)
2. Go to `/settings` on the clan site
3. Click **Link Nostr Key**
4. Approve the signing request in your extension
5. Your Nostr pubkey is now linked — you're the same identity everywhere

## Exporting Your Server-Managed Key

If you started with a server-managed key and want to use it in external clients:

1. Go to `/settings` on the clan site
2. Click **Export Nostr Key**
3. Save your secret key (`nsec...`) securely — treat it like a password
4. Import it into your preferred Nostr client
5. Your key mode will change from "server-managed" to "external"

**Warning:** Once you export your key, the server can no longer sign events on your behalf. You'll need to use a NIP-07 extension for the web widget.

## Our Relay

- **URL:** `wss://relay.scuffed.gg`
- **Auth:** NIP-42 (your key must be registered with the clan)
- **Groups:** NIP-29 (team channels are auto-provisioned)
- **Encryption:** Officer channels use NIP-44 encrypted events
