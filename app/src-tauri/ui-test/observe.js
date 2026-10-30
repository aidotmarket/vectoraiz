// Injected before application scripts, only by the debug ui-test feature.
(() => {
  const evidence = { errors: [], network: [] };
  window.__VZ_UI_TEST__ = evidence;
  const record = (kind, message) => evidence.errors.push({ kind, message: String(message) });
  window.addEventListener('error', event => record('error', event.error?.stack ?? event.message));
  window.addEventListener('unhandledrejection', event => record('unhandledrejection', event.reason?.stack ?? event.reason));
  const originalError = console.error.bind(console);
  console.error = (...args) => {
    record('console.error', args.join(' '));
    originalError(...args);
  };
  // Tauri IPC uses ipc: / ipc.localhost; deny external frontend requests.
  const external = url => /^https?:/.test(String(url)) && !String(url).startsWith('http://ipc.localhost');
  const fetch = window.fetch.bind(window);
  window.fetch = (input, init) => {
    const url = input instanceof Request ? input.url : String(input);
    if (external(url)) {
      evidence.network.push(url);
      return Promise.reject(new Error('UI smoke test forbids external network: ' + url));
    }
    return fetch(input, init);
  };
})();
