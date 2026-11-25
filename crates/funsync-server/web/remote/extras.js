// Desktop extras for the web remote: keyboard shortcuts (README "Keyboard
// Shortcuts"), screenshot, picture-in-picture, aspect ratio cycling, a replay
// button, the A-B loop, the seek bar's frame preview and the library cards'
// hover preview (mouse only). The server adds this script to the pages.
(() => {
  const FITS = ['contain', 'cover', 'fill'];
  const video = () => [...document.querySelectorAll('video')].find((v) => v.offsetParent !== null || v.currentSrc);
  const idOf = (v) => (v?.currentSrc.match(/\/api\/media\/stream\/([0-9a-f]+)/) || [])[1];
  let actions = { id: null, at: [] };

  async function actionTimes(v) {
    const id = idOf(v);
    if (!id) return [];
    if (actions.id !== id) {
      actions = { id, at: [] };
      try {
        const s = await (await fetch(`/api/media/script/${id}`)).json();
        actions.at = (s.actions || []).map((a) => a.at / 1000).sort((a, b) => a - b);
      } catch { /* no script */ }
    }
    return actions.at;
  }

  function screenshot(v) {
    const c = document.createElement('canvas');
    c.width = v.videoWidth;
    c.height = v.videoHeight;
    c.getContext('2d').drawImage(v, 0, 0);
    const a = document.createElement('a');
    a.download = `screenshot-${Math.round(v.currentTime)}s.png`;
    a.href = c.toDataURL('image/png');
    a.click();
  }

  const replay = document.createElement('button');
  replay.textContent = '↻ Replay';
  replay.style.cssText = 'position:fixed;left:50%;top:40%;transform:translateX(-50%);z-index:2000;display:none;'
    + 'font:600 18px system-ui;padding:12px 22px;border-radius:24px;border:0;background:#e94560;color:#fff;cursor:pointer';
  document.addEventListener('DOMContentLoaded', () => document.body.append(replay));
  replay.onclick = () => { const v = video(); if (v) { v.currentTime = 0; v.play(); } replay.style.display = 'none'; };
  document.addEventListener('ended', () => { replay.style.display = 'block'; }, true);
  document.addEventListener('play', () => { replay.style.display = 'none'; }, true);

  // A-B loop (A / B set the points, Escape clears) and a small status toast.
  const loop = { a: null, b: null, v: null };
  const toast = document.createElement('div');
  toast.style.cssText = 'position:fixed;left:12px;bottom:12px;z-index:2000;display:none;white-space:pre;'
    + 'font:13px/1.4 system-ui;padding:8px 12px;border-radius:8px;background:#000c;color:#fff;pointer-events:none';
  document.addEventListener('DOMContentLoaded', () => document.body.append(toast));
  let toastTimer = 0;
  const say = (text, ms = 2500) => {
    toast.textContent = text;
    toast.style.display = 'block';
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => { toast.style.display = 'none'; }, ms);
  };
  const clock = (s) => `${Math.floor(s / 60)}:${String(Math.floor(s % 60)).padStart(2, '0')}`;
  document.addEventListener('timeupdate', (e) => {
    const v = e.target;
    if (v === loop.v && loop.a !== null && loop.b !== null && v.currentTime >= loop.b) v.currentTime = loop.a;
  }, true);
  document.addEventListener('emptied', (e) => { if (e.target === loop.v) loop.a = loop.b = null; }, true);

  document.addEventListener('keydown', async (e) => {
    if (e.metaKey || e.ctrlKey || e.altKey || /^(INPUT|TEXTAREA|SELECT)$/.test(e.target.tagName) || e.target.isContentEditable) return;
    const v = video();
    if (!v) return;
    const seek = (d) => { v.currentTime = Math.max(0, Math.min(v.duration || Infinity, v.currentTime + d)); };
    switch (e.key) {
      case ' ': case 'k': case 'K':
        if (v.ended) v.currentTime = 0;
        if (v.paused) v.play(); else v.pause();
        break;
      case 'ArrowLeft': seek(-5); break;
      case 'ArrowRight': seek(5); break;
      case 'j': case 'J': seek(-10); break;
      case 'l': case 'L': seek(10); break;
      case 'ArrowUp': v.volume = Math.min(1, v.volume + 0.1); break;
      case 'ArrowDown': v.volume = Math.max(0, v.volume - 0.1); break;
      case 'm': case 'M': v.muted = !v.muted; break;
      case 'f': case 'F': case 'F11':
        if (document.fullscreenElement) document.exitFullscreen(); else (v.parentElement || v).requestFullscreen?.();
        break;
      case 's': case 'S': screenshot(v); break;
      case 'p': case 'P':
        if (document.pictureInPictureElement) document.exitPictureInPicture(); else v.requestPictureInPicture?.();
        break;
      case 'a': case 'A': loop.v = v; loop.a = v.currentTime; if (loop.b !== null && loop.b <= loop.a) loop.b = null; say(`Loop A ${clock(loop.a)}`); break;
      case 'b': case 'B':
        if (loop.a === null || v.currentTime <= loop.a) { say('Set A first (before B)'); break; }
        loop.b = v.currentTime;
        say(`Loop ${clock(loop.a)} → ${clock(loop.b)}`);
        break;
      case 'Escape': if (loop.a === null) return; loop.a = loop.b = null; say('Loop cleared'); break;
      case 'i': case 'I':
        say(`${decodeURIComponent((document.title || '').trim())}\n${clock(v.currentTime)} / ${clock(v.duration || 0)}`
          + ` · ${v.videoWidth}×${v.videoHeight} · ${v.playbackRate}×${loop.a !== null ? `\nLoop ${clock(loop.a)} → ${loop.b !== null ? clock(loop.b) : '…'}` : ''}`, 4000);
        break;
      case 'h': case 'H': window.open('/remote/devices.html', 'funsync-devices'); break;
      case 'r': case 'R': v.style.objectFit = FITS[(FITS.indexOf(v.style.objectFit || 'contain') + 1) % FITS.length]; break;
      case 'v': case 'V': {
        // The player's variant menu (its radio rows); V next, Shift+V previous.
        const rows = [...document.querySelectorAll('.player__variant-row-input')];
        if (rows.length < 2) return;
        const i = rows.findIndex((r) => r.checked);
        rows[(i + (e.shiftKey ? rows.length - 1 : 1)) % rows.length].click();
        break;
      }
      case 'g': case 'G': {
        const at = await actionTimes(v);
        const t = v.currentTime;
        const next = e.shiftKey ? [...at].reverse().find((x) => x < t - 0.5) : at.find((x) => x > t + 0.5);
        if (next !== undefined) v.currentTime = next;
        break;
      }
      case 't': case 'T': {
        // Pin this frame as the video's thumbnail; Shift+T takes the pin away.
        const id = idOf(v);
        const pct = e.shiftKey ? null : v.currentTime / (v.duration || Infinity);
        if (!id || (!e.shiftKey && !(pct >= 0))) return;
        const r = await fetch(`/api/media/pin-thumb/${id}`, {
          method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ pct }),
        });
        say(!r.ok ? 'Thumbnail not changed' : e.shiftKey ? 'Thumbnail pin removed' : `Thumbnail pinned at ${clock(v.currentTime)}`);
        break;
      }
      case 'z': case 'Z':
        if (!e.repeat) edge(true);
        break;
      default: return;
    }
    e.preventDefault();
  });

  // Edge hold (Z): the devices idle while it is held (toggle mode: each
  // press flips it; the server ignores the release then).
  const edge = (active) => fetch('/api/remote/edge', {
    method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ active }),
  }).catch(() => {});
  document.addEventListener('keyup', (e) => {
    if ((e.key === 'z' || e.key === 'Z') && !/^(INPUT|TEXTAREA|SELECT)$/.test(e.target.tagName)) edge(false);
  });
  // Seek bar preview: a hidden second <video> of the same stream seeks to the
  // hovered time; its frame is drawn above the cursor with the time.
  const hoverable = matchMedia('(hover: hover) and (pointer: fine)').matches;
  const tip = document.createElement('div');
  tip.style.cssText = 'position:fixed;z-index:2001;display:none;pointer-events:none;background:#000;'
    + 'border:1px solid #fff4;border-radius:6px;overflow:hidden;font:600 12px system-ui;color:#fff;text-align:center';
  const canvas = document.createElement('canvas');
  canvas.width = 160;
  canvas.height = 90;
  canvas.style.display = 'block';
  const label = document.createElement('div');
  tip.append(canvas, label);
  const probe = document.createElement('video');
  probe.muted = true;
  probe.preload = 'auto';
  let want = null;
  probe.addEventListener('seeked', () => {
    canvas.height = Math.round(160 * (probe.videoHeight / probe.videoWidth || 9 / 16));
    canvas.getContext('2d').drawImage(probe, 0, 0, canvas.width, canvas.height);
    if (want !== null && Math.abs(want - probe.currentTime) > 0.5) { probe.currentTime = want; }
    want = null;
  });
  const fmt = (s) => `${Math.floor(s / 60)}:${String(Math.floor(s % 60)).padStart(2, '0')}`;
  document.addEventListener('DOMContentLoaded', () => document.body.append(tip));
  document.addEventListener('pointermove', (e) => {
    const bar = hoverable && e.target.closest?.('.pcx__seek');
    const v = bar && video();
    if (!v || !v.duration || !v.currentSrc) { tip.style.display = 'none'; return; }
    if (probe.src !== v.currentSrc) probe.src = v.currentSrc;
    const r = bar.getBoundingClientRect();
    const t = Math.max(0, Math.min(1, (e.clientX - r.left) / r.width)) * v.duration;
    label.textContent = fmt(t);
    if (probe.seeking) want = t; else probe.currentTime = t;
    tip.style.display = 'block';
    tip.style.left = `${Math.max(4, Math.min(innerWidth - 168, e.clientX - 80))}px`;
    tip.style.top = `${r.top - canvas.height - 28}px`;
  });

  // Card hover preview: after a short hover a library card plays its video
  // muted over the thumbnail, from 10 % in.
  let hover = null;
  const stopHover = () => {
    if (!hover) return;
    clearTimeout(hover.timer);
    hover.video?.remove();
    hover = null;
  };
  document.addEventListener('mouseover', (e) => {
    const card = hoverable && e.target.closest?.('.card');
    if (!card || hover?.card === card) return;
    stopHover();
    const img = card.querySelector('img.card__thumb');
    const id = (img?.src.match(/\/api\/media\/thumb\/([0-9a-f]+)/) || [])[1];
    if (!id) return;
    hover = { card };
    hover.timer = setTimeout(() => {
      const p = document.createElement('video');
      Object.assign(p, { muted: true, loop: true, autoplay: true, playsInline: true, src: `/api/media/stream/${id}` });
      p.style.cssText = `position:absolute;left:${img.offsetLeft}px;top:${img.offsetTop}px;width:${img.offsetWidth}px;`
        + `height:${img.offsetHeight}px;object-fit:cover;pointer-events:none;border-radius:inherit`;
      p.addEventListener('loadedmetadata', () => { p.currentTime = p.duration * 0.1 || 0; }, { once: true });
      if (getComputedStyle(card).position === 'static') card.style.position = 'relative';
      card.append(p);
      hover.video = p;
    }, 400);
  });
  document.addEventListener('mouseout', (e) => {
    if (hover && !hover.card.contains(e.relatedTarget)) stopHover();
  });
  addEventListener('hashchange', stopHover);
})();
