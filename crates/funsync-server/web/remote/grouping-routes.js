// Hash routes for the grouping detail views: `#collection/<id>`,
// `#playlist/<id>`, `#category/<id>`, `#artist/<id>`. The lists are keyed by
// the plural kind (`collections`, the `/api/remote/<kind>` endpoint), the
// routes by the singular, and the two don't derive from each other by adding
// or dropping an "s" (categories → category), so both directions are spelled
// out. Ids are URI-encoded in the hash (an artist's id is its folded name,
// spaces and all) and decoded back.

export const GROUPING_SINGULAR = Object.freeze({
  collections: 'collection',
  playlists: 'playlist',
  categories: 'category',
  artists: 'artist',
});

export const GROUPING_PLURAL = Object.freeze({
  collection: 'collections',
  playlist: 'playlists',
  category: 'categories',
  artist: 'artists',
});

const DETAIL_RE = /^(collection|playlist|category|artist)\/(.+)$/;

/** `#…` hash (without `#`) opening grouping `id` of plural `kind`. */
export function groupingDetailHash(kind, id) {
  const singular = GROUPING_SINGULAR[kind];
  if (!singular) throw new Error(`Unknown grouping kind: ${kind}`);
  return `${singular}/${encodeURIComponent(id)}`;
}

/** `{kind, id}` (plural kind) for a grouping detail hash, else null. */
export function parseGroupingDetailHash(hash) {
  const m = String(hash || '').match(DETAIL_RE);
  if (!m) return null;
  let id = m[2];
  try { id = decodeURIComponent(id); } catch { /* not encoded: keep as is */ }
  return { kind: GROUPING_PLURAL[m[1]], id };
}
