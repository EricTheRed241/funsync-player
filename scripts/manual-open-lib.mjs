// Opening the download list's pages in the browser, a few posts at a time
// (scripts/manual-open.mjs). The list is the text `eroscripts:scout
// --manual-list` writes (manualListText in eroscripts-scout-lib.mjs):
//
//   1. <post title>
//      Post: <post url>  (id <topic id>)
//      - <file name> (<size>)
//        <file page url>
//
// Pure: no file system, no browser.

/**
 * The posts of a download list, in order: `[{n, title, url, id, files:
 * [{name, page}]}]`. Lines that aren't part of a post (the instructions)
 * are ignored.
 *
 * @param {string} text
 */
export function parseManualList(text) {
  const posts = [];
  let post = null;
  let file = null;
  for (const line of String(text || '').split(/\r?\n/)) {
    let m;
    if ((m = line.match(/^(\d+)\. (.+)$/))) {
      post = { n: Number(m[1]), title: m[2].trim(), url: null, id: null, files: [] };
      posts.push(post);
      file = null;
    } else if (post && (m = line.match(/^\s+Post: (\S+)\s+\(id (\d+)\)/))) {
      post.url = m[1];
      post.id = Number(m[2]);
    } else if (post && (m = line.match(/^\s+- (.+?)(?: \([\d.]+ [KMG]B\))?$/))) {
      file = { name: m[1], page: null };
      post.files.push(file);
    } else if (file && !file.page && (m = line.match(/^\s+(https?:\/\/\S+)$/))) {
      file.page = m[1];
    }
  }
  return posts.filter((p) => p.url);
}

/** The post ids of a download list (its `--fetch --ids a,b,c` line), so it can be made again; [] when it has none. */
export function listIds(text) {
  const m = String(text || '').match(/--fetch --ids ([\d,]+)/);
  return m ? m[1].split(',').map(Number).filter((n) => n > 0) : [];
}

/**
 * A browser's name for a second download of the same file: `name (1).mp4`
 * → `name.mp4`. Other names come back unchanged.
 */
export function withoutCopySuffix(name) {
  return String(name || '').replace(/ \(\d+\)(\.[^.]+)$/, '$1');
}

/**
 * The posts with files still to download: a file counts as saved when
 * `saved` (names in the download folders, NFC) holds it, under its own name
 * or with a browser's ` (1)` suffix. Each post keeps only its missing files.
 *
 * @param {ReturnType<typeof parseManualList>} posts
 * @param {Iterable<string>} saved file names in the download folders
 */
export function postsToOpen(posts, saved) {
  const have = new Set([...saved].map((n) => withoutCopySuffix(String(n).normalize('NFC'))));
  return posts
    .map((p) => ({ ...p, files: p.files.filter((f) => !have.has(f.name.normalize('NFC'))) }))
    .filter((p) => p.files.length);
}

/**
 * The URLs to open for a batch, in pairs: each post's page, then its files'
 * pages.
 *
 * @param {ReturnType<typeof postsToOpen>} batch
 */
export function batchUrls(batch) {
  return batch.flatMap((p) => [p.url, ...p.files.map((f) => f.page).filter(Boolean)]);
}
