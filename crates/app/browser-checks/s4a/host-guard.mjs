// The checks seed accounts, delete events and rewrite site settings, so they
// only run against this machine. A BASE whose host is not loopback is refused
// unless ALLOW_REMOTE=1 is set. No imports: `node --test` loads this alone.
const LOOPBACK = new Set(['localhost', '127.0.0.1', '[::1]']);

export function isLoopbackHost(hostname) {
  const h = String(hostname).toLowerCase();
  return LOOPBACK.has(h) || /^127(\.\d{1,3}){3}$/.test(h);
}

// Returns the BASE to use (no trailing slash) or throws with the reason.
export function checkedBase(base, allowRemote) {
  const raw = (base || 'http://127.0.0.1:3031').replace(/\/$/, '');
  let url;
  try { url = new URL(raw); } catch { throw new Error(`BASE is not a URL: ${raw}`); }
  if (url.protocol !== 'http:' && url.protocol !== 'https:') {
    throw new Error(`BASE must be http or https: ${raw}`);
  }
  if (!isLoopbackHost(url.hostname) && allowRemote !== '1') {
    throw new Error(
      `BASE host ${url.hostname} is not loopback. These checks seed accounts, delete events and rewrite settings; ` +
      'point them at a local stack, or set ALLOW_REMOTE=1 if this host is a throwaway stack.'
    );
  }
  return raw;
}
