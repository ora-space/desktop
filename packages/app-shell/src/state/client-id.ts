const CLIENT_ID_STORAGE_KEY = "ora.clientId";

let cached: string | null = null;

/**
 * Identifies this client surface to the backend for the lifetime of the tab.
 *
 * Warm sessions are keyed partly by this value. One backend can serve several
 * clients — browser tabs against the Web server — and two tabs showing the same
 * selection must not be handed the same provider session, or whichever attaches
 * first takes the other tab's conversation. The Desktop app has a single window,
 * so this is effectively constant there.
 *
 * `sessionStorage` is deliberate: it is per-tab and survives a reload, so a
 * refreshed tab reclaims the warm session it already had instead of stranding it
 * until the backend's idle timeout.
 */
export function clientId(): string {
  if (cached !== null) return cached;
  const stored = window.sessionStorage.getItem(CLIENT_ID_STORAGE_KEY);
  if (stored !== null && stored !== "") {
    cached = stored;
    return cached;
  }
  const created = crypto.randomUUID();
  window.sessionStorage.setItem(CLIENT_ID_STORAGE_KEY, created);
  cached = created;
  return created;
}
