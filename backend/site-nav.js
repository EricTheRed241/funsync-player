// The nav bar on every page the backend serves (routes/sites.py): the web remote and each
// local site of deploy/sites.json, the current one marked. window.FUNSYNC_NAV
// ([{name, href}]) is prepended by the backend. The bar is the body's first child; it sets
// `--site-nav-h` on <html> (its height) for pages that size themselves to the viewport.
(() => {
  const links = window.FUNSYNC_NAV || [];
  if (!links.length || document.getElementById('funsync-site-nav')) return;
  const style = document.createElement('style');
  style.textContent = `
#funsync-site-nav { flex: none; display: flex; gap: 2px; align-items: center; overflow-x: auto;
  padding: calc(4px + env(safe-area-inset-top)) 10px 4px; background: #111216; border-bottom: 1px solid #2a2b31;
  font: 600 13px/1.2 -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif; position: relative; z-index: 1000; }
#funsync-site-nav b { color: #e0529c; margin-right: 8px; white-space: nowrap; }
#funsync-site-nav a { color: #b8b8c0; text-decoration: none; padding: 5px 10px; border-radius: 6px; white-space: nowrap; }
#funsync-site-nav a:hover { background: #23242a; color: #fff; }
#funsync-site-nav a[aria-current="page"] { background: #2f6fdf; color: #fff; }
html.has-site-nav { --safe-top: 0px; }`;
  document.head.append(style);
  const nav = document.createElement('nav');
  nav.id = 'funsync-site-nav';
  nav.setAttribute('aria-label', 'FunSync pages');
  const brand = document.createElement('b');
  brand.textContent = 'FunSync';
  nav.append(brand);
  // The longest link the page is under: /swipe/downloads marks Downloads, not Swipe.
  const here = location.pathname;
  const under = (href) => here === href || here.startsWith(href) || here === href.replace(/\/$/, '');
  const current = links.filter((l) => under(l.href)).sort((a, b) => b.href.length - a.href.length)[0];
  for (const link of links) {
    const a = document.createElement('a');
    a.href = link.href;
    a.textContent = link.name;
    if (link === current) a.setAttribute('aria-current', 'page');
    nav.append(a);
  }
  document.body.prepend(nav);
  document.documentElement.classList.add('has-site-nav');
  const size = () => document.documentElement.style.setProperty('--site-nav-h', `${nav.offsetHeight}px`);
  size();
  new ResizeObserver(size).observe(nav);
})();
