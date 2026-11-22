// Picture lookahead, shared by the swipe deck (scout-report) and the Creators site
// (creator-report): each page has this file inlined where its script says `// @preload.js`
// (server::with_preloader), so both pages stay one self-contained file.
//
// The page names the pictures coming up, nearest first (plan); they're fetched in the
// background `parallel` at a time at low priority, so a card's picture is in the browser's
// cache by the time it shows. Card GIFs run 2–8 MB, so pages plan only what's close (the
// shown card's next picture and the first of the next few cards): fetching every picture of
// every card at once made swiping lag on a phone. `soon` puts one picture ahead of the plan
// (the one a tap shows next).
function picturePreloader(parallel) {
  const done = new Set();     // fetched (or failed): in the browser's cache
  const loading = new Map();  // url → Image on its way (held so the fetch isn't dropped)
  let first = [];             // soon(): ahead of the plan
  let planned = [];           // plan(), nearest first
  function pump() {
    while (loading.size < parallel) {
      const url = first.shift() || planned.shift();
      if (!url) return;
      if (done.has(url) || loading.has(url)) continue;
      const img = new Image();
      img.decoding = 'async';
      img.fetchPriority = 'low';
      img.onload = img.onerror = () => {
        loading.delete(url);
        done.add(url);
        pump();
      };
      loading.set(url, img);
      img.src = url;
    }
  }
  return {
    /** Fetch `url` next. */
    soon(url) {
      if (!url || done.has(url) || loading.has(url)) return;
      first.unshift(url);
      pump();
    },
    /** Fetch `urls` (nearest first; empty entries skipped) instead of the earlier plan. */
    plan(urls) {
      planned = urls.filter((u) => u && !done.has(u));
      pump();
    },
  };
}
