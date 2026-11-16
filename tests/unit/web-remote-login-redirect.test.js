/**
 * @vitest-environment node
 */
// Access PIN on the web remote (backend/web-remote/login-redirect.js): a 401
// from the backend sends the page to the login page; other answers pass
// through untouched.

import { describe, it, expect } from 'vitest';
import { installLoginRedirect, redirectToLogin, LOGIN_URL } from '../../backend/web-remote/login-redirect.js';

function fakeWindow(status) {
  const assigned = [];
  const win = {
    location: { assign: (url) => assigned.push(url) },
    fetch: async () => ({ status }),
  };
  return { win, assigned };
}

describe('installLoginRedirect', () => {
  it('goes to the login page on a 401 and still returns the response', async () => {
    const { win, assigned } = fakeWindow(401);
    installLoginRedirect(win);
    const resp = await win.fetch('/api/remote/videos');
    expect(resp.status).toBe(401);
    expect(assigned).toEqual([LOGIN_URL]);
    expect(LOGIN_URL).toBe('/login?next=%2Fremote%2F');
  });

  it('leaves other answers alone', async () => {
    for (const status of [200, 403, 404, 429, 500]) {
      const { win, assigned } = fakeWindow(status);
      installLoginRedirect(win);
      expect((await win.fetch('/x')).status).toBe(status);
      expect(assigned).toEqual([]);
    }
  });
});

describe('redirectToLogin', () => {
  it('sets href on a location without assign', () => {
    const loc = {};
    redirectToLogin(loc);
    expect(loc.href).toBe(LOGIN_URL);
  });
});
