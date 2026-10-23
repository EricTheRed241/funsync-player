// Custom thumbnail images (user-uploaded posters, community #217, and
// EroScripts post previews). Image I/O stays in the main process (the
// renderer is sandboxed). Every import goes through `importThumbnail`: the
// image is decoded, downscaled to tile width, re-encoded as JPEG and stored
// under userData/custom-thumbs keyed by a hash of the video path. The
// renderer displays thumbnails as data URLs (CSP img-src allows data:, not
// file:), so the import returns one.

const fs = require('fs');
const path = require('path');
const crypto = require('crypto');

const MAX_WIDTH = 640;
const JPEG_QUALITY = 82;

// Formats nativeImage can't decode (webp on every platform, animated
// images, avif) go through Chromium instead. Bounded so a hostile or broken
// file can't hold the import open.
const CHROMIUM_DECODE_TIMEOUT_MS = 15_000;

/**
 * Cache file stem for a video. User uploads and EroScripts previews use
 * separate files, so a preview download that finishes late can never
 * overwrite a poster the user picked in the meantime.
 *
 * @param {string} videoPath
 * @param {'user'|'eroscripts'} [source]
 */
function thumbStem(videoPath, source = 'user') {
  const hash = crypto.createHash('md5').update(String(videoPath)).digest('hex');
  return source === 'eroscripts' ? `${hash}-eroscripts` : hash;
}

/**
 * Decode `bytes`, downscale to MAX_WIDTH and write `<stem>.jpg` into `dir`.
 *
 * @param {Buffer} bytes encoded image
 * @param {object} opts
 * @param {string} opts.dir custom-thumbs directory
 * @param {string} opts.stem file name without extension (see thumbStem)
 * @param {{createFromBuffer: Function}} opts.nativeImage Electron's nativeImage
 * @param {(bytes: Buffer) => Promise<Buffer|null>} [opts.decodeFallback]
 *   turns bytes nativeImage can't read into PNG bytes
 * @returns {Promise<{path: string, dataUrl: string}|null>} null when the
 *   image can't be decoded; an existing thumbnail is left untouched then
 */
async function importThumbnail(bytes, { dir, stem, nativeImage, decodeFallback }) {
  let img = nativeImage.createFromBuffer(bytes);
  if (img.isEmpty() && decodeFallback) {
    const png = await decodeFallback(bytes);
    if (png) img = nativeImage.createFromBuffer(png);
  }
  if (img.isEmpty()) return null;

  const size = img.getSize();
  if (size.width > MAX_WIDTH) img = img.resize({ width: MAX_WIDTH });
  const jpeg = img.toJPEG(JPEG_QUALITY);

  fs.mkdirSync(dir, { recursive: true });
  // Older versions copied undecodable files raw (`<stem>.webp` etc.); drop
  // any such sibling so only the fresh JPEG remains.
  for (const f of fs.readdirSync(dir)) {
    if (f.startsWith(`${stem}.`)) {
      try { fs.unlinkSync(path.join(dir, f)); } catch { /* best-effort */ }
    }
  }
  const outPath = path.join(dir, `${stem}.jpg`);
  fs.writeFileSync(outPath, jpeg);
  return { path: outPath, dataUrl: `data:image/jpeg;base64,${jpeg.toString('base64')}` };
}

/**
 * Decode an image with Chromium in a hidden, sandboxed window and return
 * its first frame as PNG bytes (≤ MAX_WIDTH wide), or null.
 *
 * nativeImage only reads PNG/JPEG (plus a few OS formats); EroScripts
 * previews are mostly webp, often animated, and a raw copy of those can be
 * several MB per tile.
 *
 * @param {Buffer} bytes
 * @returns {Promise<Buffer|null>}
 */
async function decodeWithChromium(bytes) {
  const { BrowserWindow } = require('electron');
  const win = new BrowserWindow({
    show: false,
    width: 64,
    height: 64,
    paintWhenInitiallyHidden: false,
    webPreferences: { sandbox: true, contextIsolation: true, nodeIntegration: false },
  });
  let timer;
  try {
    await win.loadURL('about:blank');
    const script = `(async () => {
      const b64 = ${JSON.stringify(bytes.toString('base64'))};
      const raw = Uint8Array.from(atob(b64), (c) => c.charCodeAt(0));
      const bmp = await createImageBitmap(new Blob([raw]));
      const scale = Math.min(1, ${MAX_WIDTH} / bmp.width);
      const w = Math.max(1, Math.round(bmp.width * scale));
      const h = Math.max(1, Math.round(bmp.height * scale));
      const canvas = new OffscreenCanvas(w, h);
      canvas.getContext('2d').drawImage(bmp, 0, 0, w, h);
      const blob = await canvas.convertToBlob({ type: 'image/png' });
      const out = new Uint8Array(await blob.arrayBuffer());
      let s = '';
      for (let i = 0; i < out.length; i += 0x8000) s += String.fromCharCode(...out.subarray(i, i + 0x8000));
      return btoa(s);
    })()`;
    const timeout = new Promise((resolve) => { timer = setTimeout(() => resolve(null), CHROMIUM_DECODE_TIMEOUT_MS); });
    const b64 = await Promise.race([win.webContents.executeJavaScript(script), timeout]);
    return typeof b64 === 'string' && b64 ? Buffer.from(b64, 'base64') : null;
  } catch {
    return null;
  } finally {
    clearTimeout(timer);
    if (!win.isDestroyed()) win.destroy();
  }
}

module.exports = {
  MAX_WIDTH,
  thumbStem,
  importThumbnail,
  decodeWithChromium,
};
