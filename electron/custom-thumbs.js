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
const { toGray } = require('./preview-match');

const MAX_WIDTH = 640;
const JPEG_QUALITY = 82;

// Formats nativeImage can't decode (webp on every platform, animated
// images, avif) go through Chromium instead. Bounded so a hostile or broken
// file can't hold the import open.
const CHROMIUM_DECODE_TIMEOUT_MS = 15_000;
// The decoder window is kept between decodes (a batch decodes hundreds) and
// closed once idle this long.
const DECODER_IDLE_MS = 20_000;

// One hidden, sandboxed window decodes everything, one job at a time.
// Creating a window per decode broke: the second window's load failed
// (ERR_FAILED) and the next one crashed Electron. It loads a local page
// rather than about:blank because ImageDecoder (WebCodecs, needed for the
// frames of an animation) exists only in a secure context.
let _decoder = null;
let _decoderIdle = null;
let _decodeChain = Promise.resolve();

function decoderWindow() {
  if (_decoder && !_decoder.win.isDestroyed()) return _decoder.ready;
  const { BrowserWindow } = require('electron');
  const win = new BrowserWindow({
    show: false,
    width: 64,
    height: 64,
    paintWhenInitiallyHidden: false,
    webPreferences: { sandbox: true, contextIsolation: true, nodeIntegration: false },
  });
  const ready = win.loadFile(path.join(__dirname, 'decoder.html')).then(() => win);
  _decoder = { win, ready };
  ready.catch(() => closeDecoder());
  return ready;
}

function closeDecoder() {
  clearTimeout(_decoderIdle);
  if (_decoder && !_decoder.win.isDestroyed()) _decoder.win.destroy();
  _decoder = null;
}

/** Run `script` in the decoder window; its result, or null on error or timeout. */
function runInDecoder(script) {
  const job = _decodeChain.then(async () => {
    clearTimeout(_decoderIdle);
    let timer;
    try {
      const win = await decoderWindow();
      const timeout = new Promise((resolve) => { timer = setTimeout(() => resolve(runInDecoder.TIMEOUT), CHROMIUM_DECODE_TIMEOUT_MS); });
      const result = await Promise.race([win.webContents.executeJavaScript(script), timeout]);
      // A decode still running in there would block the next one.
      if (result === runInDecoder.TIMEOUT) { closeDecoder(); return null; }
      return result;
    } catch {
      return null;
    } finally {
      clearTimeout(timer);
      _decoderIdle = setTimeout(closeDecoder, DECODER_IDLE_MS);
    }
  });
  _decodeChain = job;
  return job;
}
runInDecoder.TIMEOUT = Symbol('timeout');

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
 * Decode an image with Chromium (the hidden decoder window) and return its
 * first frame as PNG bytes (≤ MAX_WIDTH wide), or null.
 *
 * nativeImage only reads PNG/JPEG (plus a few OS formats); EroScripts
 * previews are mostly webp, often animated, and a raw copy of those can be
 * several MB per tile.
 *
 * @param {Buffer} bytes
 * @returns {Promise<Buffer|null>}
 */
async function decodeWithChromium(bytes) {
  const b64 = await runInDecoder(`(async () => {
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
  })()`);
  return typeof b64 === 'string' && b64 ? Buffer.from(b64, 'base64') : null;
}

// ---- Grayscale frames for preview matching (electron/preview-match.js) ----

/** Width images are decoded at for hashing (the hash input is 32×32). */
const MATCH_WIDTH = 128;
/** Frames hashed from an animated preview, spread over the animation. */
const MATCH_MAX_FRAMES = 6;

// GIF and WebP may be animated: those go to Chromium's ImageDecoder, which
// can read every frame. nativeImage reads only the first.
function maybeAnimated(bytes) {
  if (bytes.length < 12) return false;
  if (bytes.toString('latin1', 0, 4) === 'GIF8') return true;
  return bytes.toString('latin1', 0, 4) === 'RIFF' && bytes.toString('latin1', 8, 12) === 'WEBP';
}

/**
 * Decode an image into grayscale frames for hashing: one for a still,
 * up to MATCH_MAX_FRAMES spread over an animation.
 *
 * @param {Buffer} bytes
 * @param {{nativeImage: object, chromium?: (bytes: Buffer) => Promise<Array<object>|null>}} deps
 * @returns {Promise<Array<{gray: Uint8Array, width: number, height: number,
 *   srcWidth: number, srcHeight: number}>>} [] when undecodable
 */
async function decodeGrayFrames(bytes, { nativeImage, chromium = decodeGrayFramesWithChromium }) {
  if (!maybeAnimated(bytes)) {
    let img = nativeImage.createFromBuffer(bytes);
    if (!img.isEmpty()) {
      const { width: srcWidth, height: srcHeight } = img.getSize();
      if (srcWidth > MATCH_WIDTH) img = img.resize({ width: MATCH_WIDTH, quality: 'good' });
      const { width, height } = img.getSize();
      // toBitmap() is BGRA on every platform Electron ships.
      return [{ gray: toGray(img.toBitmap(), width, height, 'bgra'), width, height, srcWidth, srcHeight }];
    }
  }
  return (await chromium(bytes)) || [];
}

/**
 * Chromium half of decodeGrayFrames: ImageDecoder for every frame of an
 * animation (createImageBitmap, first frame only, where it's missing).
 *
 * @param {Buffer} bytes
 * @returns {Promise<Array<object>|null>}
 */
async function decodeGrayFramesWithChromium(bytes) {
  const frames = await runInDecoder(`(async () => {
    const raw = Uint8Array.from(atob(${JSON.stringify(bytes.toString('base64'))}), (c) => c.charCodeAt(0));
    const sources = [];
    let srcWidth = 0, srcHeight = 0;
    const type = (raw[0] === 0x47 ? 'image/gif' : raw[8] === 0x57 ? 'image/webp' : '');
    if (type && typeof ImageDecoder === 'function' && await ImageDecoder.isTypeSupported(type)) {
      const dec = new ImageDecoder({ data: raw, type });
      await dec.tracks.ready;
      const count = Math.max(1, dec.tracks.selectedTrack?.frameCount || 1);
      const n = Math.min(${MATCH_MAX_FRAMES}, count);
      for (let i = 0; i < n; i++) {
        const { image } = await dec.decode({ frameIndex: Math.floor(i * count / n) });
        srcWidth = image.displayWidth; srcHeight = image.displayHeight;
        sources.push(image);
      }
      dec.close();
    } else {
      const bmp = await createImageBitmap(new Blob([raw]));
      srcWidth = bmp.width; srcHeight = bmp.height;
      sources.push(bmp);
    }
    const scale = Math.min(1, ${MATCH_WIDTH} / srcWidth);
    const w = Math.max(1, Math.round(srcWidth * scale));
    const h = Math.max(1, Math.round(srcHeight * scale));
    const canvas = new OffscreenCanvas(w, h);
    const ctx = canvas.getContext('2d', { willReadFrequently: true });
    return sources.map((src) => {
      ctx.clearRect(0, 0, w, h);
      ctx.drawImage(src, 0, 0, w, h);
      src.close?.();
      const px = ctx.getImageData(0, 0, w, h).data;
      let s = '';
      for (let i = 0; i < px.length; i += 4) {
        s += String.fromCharCode((px[i] * 299 + px[i + 1] * 587 + px[i + 2] * 114) / 1000);
      }
      return { gray: btoa(s), width: w, height: h, srcWidth, srcHeight };
    });
  })()`);
  if (!Array.isArray(frames) || frames.length === 0) return null;
  return frames.map((f) => ({ ...f, gray: new Uint8Array(Buffer.from(f.gray, 'base64')) }));
}

module.exports = {
  MAX_WIDTH,
  thumbStem,
  importThumbnail,
  decodeWithChromium,
  decodeGrayFrames,
  closeDecoder,
};
