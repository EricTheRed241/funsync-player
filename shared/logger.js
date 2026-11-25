// Logger for the shared modules (eroscripts-api.js …): console output, which a
// CLI turns off by setting `transports.console.level = false` (the electron-log
// shape the CLIs already use; `transports.file` is accepted and ignored).
const transports = { console: { level: 'info' }, file: { level: false } };

const emit = (method) => (...args) => {
  if (transports.console.level !== false) console[method](...args);
};

module.exports = {
  transports,
  error: emit('error'),
  warn: emit('warn'),
  info: emit('info'),
  verbose: emit('debug'),
  debug: emit('debug'),
  log: emit('log'),
};
