// Access PIN (backend/routes/access.py). Once a PIN is set, the backend
// answers 401 (WebSockets: close code 4401) when this device's 30-day login
// has run out or the PIN was changed; the remote then goes to the login page,
// which comes back to /remote/ after the PIN.

export const LOGIN_URL = '/login?next=%2Fremote%2F';
export const PIN_REQUIRED_CLOSE_CODE = 4401;

export function redirectToLogin(loc = globalThis.location) {
  if (!loc) return;
  if (typeof loc.assign === 'function') loc.assign(LOGIN_URL);
  else loc.href = LOGIN_URL;
}

/** Wrap `win.fetch` so any 401 answer sends the page to the login page. */
export function installLoginRedirect(win = globalThis) {
  const nativeFetch = win.fetch.bind(win);
  win.fetch = async (...args) => {
    const resp = await nativeFetch(...args);
    if (resp.status === 401) redirectToLogin(win.location);
    return resp;
  };
}
