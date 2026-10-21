const report = checkpoint => window.__TAURI_INTERNALS__.invoke('self_test', { checkpoint });
(async () => {
  await report('started');
  const wait = async (predicate) => {
    const deadline = Date.now() + 240000;
    let logged = Date.now();
    while (!predicate()) {
      if (Date.now() - logged > 10000) {
        await report('waiting ' + predicate.toString() + ' DOM: ' + document.querySelector('section')?.textContent.slice(0, 1200));
        logged = Date.now();
      }
      if (document.querySelector('[role="alert"]')) throw new Error(document.querySelector('[role="alert"]').textContent);
      if (Date.now() > deadline) throw new Error('Pipeline UI timed out');
      await new Promise(resolve => setTimeout(resolve, 100));
    }
  };
  const button = label => [...document.querySelectorAll('button')].find(b => b.textContent === label);
  await wait(() => button('Index folder'));
  await report('indexing');
  button('Index folder').click();
  await wait(() => document.querySelector('[role="status"]').textContent.includes('20/20') && !button('Search').disabled);
  await report('indexed');
  button('Search').click();
  await wait(() => document.querySelector('li')?.textContent.includes('thirty days') && !button('Ask').disabled);
  await report('searched');
  button('Ask').click();
  await wait(() => document.querySelector('section').textContent.includes('[1] Document') && !button('Ask').disabled);
  await report('answered');
  button('Run self-test').click();
})().catch(error => report('ERROR: ' + String(error)));
