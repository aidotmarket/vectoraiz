// Exercise the diagnostics control through the one-window shell.
(async () => {
  const report = checkpoint => window.__TAURI_INTERNALS__.invoke('self_test', { checkpoint });
  const wait = async predicate => {
    const deadline = Date.now() + 10000;
    while (!predicate()) {
      if (Date.now() > deadline) throw new Error('Self-test UI timed out: ' + predicate);
      await new Promise(resolve => setTimeout(resolve, 50));
    }
  };
  await report('self-test: started');
  const status = await window.__TAURI_INTERNALS__.invoke('listing_status');
  const marker = 'Test switch on — listing review preview';
  if (status.test_switch_on) await wait(() => document.body.textContent.includes(marker));
  await report('listing-probe:' + JSON.stringify({
    status, marker: document.body.textContent.includes(marker) ? marker : null,
  }));
  await wait(() => document.querySelector('nav a[href="#settings"]'));
  document.querySelector('nav a[href="#settings"]').click();
  await wait(() => document.querySelector('nav a[href="#settings"]').getAttribute('aria-current') === 'page'
    && document.querySelector('main > .view[aria-label="Settings"] details'));
  const diagnostics = document.querySelector('main details');
  diagnostics.querySelector('summary').click();
  const button = () => [...diagnostics.querySelectorAll('button')].find(b => b.textContent === 'Run self-test');
  await wait(() => diagnostics.open && button() && !button().disabled);
  await report('self-test: Settings diagnostics ready');
  button().click();
})().catch(error => window.__TAURI_INTERNALS__.invoke('self_test', { checkpoint: 'ERROR: ' + String(error) }));
