'use strict';

/**
 * Read a response body, giving up (null) once it passes `limit` bytes:
 * a missing or wrong Content-Length can't make it buffer more.
 *
 * @param {Response} resp
 * @param {number} limit
 * @returns {Promise<Buffer|null>}
 */
async function readCapped(resp, limit) {
  if (!resp.body?.getReader) {
    const buf = Buffer.from(await resp.arrayBuffer());
    return buf.length > limit ? null : buf;
  }
  const reader = resp.body.getReader();
  const chunks = [];
  let total = 0;
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    total += value.byteLength;
    if (total > limit) {
      try { await reader.cancel(); } catch { /* already closed */ }
      return null;
    }
    chunks.push(value);
  }
  return Buffer.concat(chunks.map((c) => Buffer.from(c.buffer, c.byteOffset, c.byteLength)), total);
}

module.exports = { readCapped };
