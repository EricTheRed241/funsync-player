// Desktop extras for the web remote (the Rust app's UI): the Electron
// player's keyboard shortcuts (README "Keyboard Shortcuts"), screenshot,
// picture-in-picture, aspect ratio cycling and a replay button. The server
// adds this script to the web remote's index.html; nothing else changes.
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
      case 'r': case 'R': v.style.objectFit = FITS[(FITS.indexOf(v.style.objectFit || 'contain') + 1) % FITS.length]; break;
      case 'g': case 'G': {
        const at = await actionTimes(v);
        const t = v.currentTime;
        const next = e.shiftKey ? [...at].reverse().find((x) => x < t - 0.5) : at.find((x) => x > t + 0.5);
        if (next !== undefined) v.currentTime = next;
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
})();
