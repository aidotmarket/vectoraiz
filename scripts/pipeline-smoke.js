const report = checkpoint => window.__TAURI_INTERNALS__.invoke('self_test', { checkpoint });
(async () => {
  await report('started');
  const button = label => [...document.querySelectorAll('button')].find(b => b.textContent === label);
  const downloadButton = () => [...document.querySelectorAll('button')].find(b => b.textContent.startsWith('Download models ('));
  const wait = async (predicate, budget = 240000) => {
    const deadline = Date.now() + budget;
    let logged = Date.now();
    while (!predicate()) {
      if (Date.now() - logged > 10000) {
        await report('waiting ' + predicate.toString() + ' DOM: ' + document.querySelector('main')?.textContent.slice(-2400));
        logged = Date.now();
      }
      if (document.querySelector('[role="alert"]')) throw new Error(document.querySelector('[role="alert"]').textContent);
      if (Date.now() > deadline) throw new Error('Pipeline UI timed out');
      await new Promise(resolve => setTimeout(resolve, 100));
    }
  };
  await wait(() => button('Index folder'));
  if (window.__VZ_SMOKE__.acquisition) {
    await report('acquisition: empty model directory, awaiting model status');
    await wait(() => downloadButton() && !downloadButton().disabled);
    // Acquisition owns a 900s total budget and at most three UI attempts, in
    // addition to the downloader's bounded retries. Pipeline gets its own 300s.
    const acquisitionDeadline = Date.now() + 900000;
    for (let attempt = 1; ; attempt++) {
      await report('acquisition attempt ' + attempt);
      (attempt === 1 ? downloadButton() : button('Retry')).click();
      await new Promise(resolve => setTimeout(resolve, 0));
      await wait(() => !document.querySelector('[aria-label="Models"]') || (button('Retry') && !button('Retry').disabled), Math.max(1, acquisitionDeadline - Date.now()));
      if (!document.querySelector('[aria-label="Models"]')) break;
      await report('acquisition attempt failed: ' + document.querySelector('[aria-label="Models"]').textContent);
      if (attempt === 3 || Date.now() >= acquisitionDeadline) throw new Error('Model acquisition retries exhausted');
    }
    await report('acquisition: verified downloads; beginning separate pipeline budget');
  }
  const pipelineDeadline = Date.now() + 300000;
  const pipelineWait = predicate => wait(predicate, Math.max(1, pipelineDeadline - Date.now()));
  const folder = [...document.querySelectorAll('label')].find(label => label.textContent.startsWith('Folder')).querySelector('input');
  if (!window.__VZ_SMOKE__.fixture) throw new Error('Absolute fixture path is required');
  Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set.call(folder, window.__VZ_SMOKE__.fixture);
  folder.dispatchEvent(new Event('input', { bubbles: true }));
  await pipelineWait(() => !button('Index folder').disabled);
  await report('indexing ' + folder.value);
  button('Index folder').click();
  await pipelineWait(() => document.querySelector('[role="status"]').textContent.includes('20/20') && !button('Search').disabled);
  await report('indexed');
  button('Search').click();
  await pipelineWait(() => document.querySelector('section li')?.textContent.includes('thirty days') && !button('Ask').disabled);
  const previousSearch = [...document.querySelectorAll('section ol:first-of-type li')].map(li => li.textContent);
  const query = 'What is the orbital archive retention policy?';
  const repeatedHits = await window.__TAURI_INTERNALS__.invoke('pipeline_search', { query });
  if (!repeatedHits[0]?.chunk.text.includes('thirty days')) throw new Error('Repeated Search lost its result');
  await report('searched again');
  await report('guard-before');
  const progress = '__CHANNEL__:' + window.__TAURI_INTERNALS__.transformCallback(() => {});
  for (const invalid of ['', '   ', window.__VZ_SMOKE__.fixture + '/directory-that-does-not-exist']) {
    let refused = false;
    try { await window.__TAURI_INTERNALS__.invoke('pipeline_index', { folder: invalid, progress }); }
    catch (error) {
      const expected = invalid.trim() ? 'Folder not found' : 'Choose a folder to index';
      if (String(error) !== expected) throw error;
      refused = true;
    }
    if (!refused) throw new Error('Invalid Folder reached index: ' + JSON.stringify(invalid));
  }
  await report('guard-after');
  await report('hold-next-search');
  window.__VZ_SMOKE__.searchLocked = false;
  const secondSearch = window.__TAURI_INTERNALS__.invoke('pipeline_search', { query });
  // Observe the backend's post-acquire signal before dispatching competitors.
  await pipelineWait(() => window.__VZ_SMOKE__.searchLocked);
  const overlaps = await Promise.allSettled([
    window.__TAURI_INTERNALS__.invoke('pipeline_search', { query }),
    window.__TAURI_INTERNALS__.invoke('pipeline_ask', { question: query, tokens: progress }),
    window.__TAURI_INTERNALS__.invoke('models_download', { progress }),
    window.__TAURI_INTERNALS__.invoke('models_import', { folder: window.__VZ_SMOKE__.fixture }),
  ]);
  await report('release-search');
  for (const result of overlaps) {
    if (result.status !== 'rejected' || String(result.reason) !== 'Another model or index operation is running') throw new Error('Overlapping operation was not refused');
  }
  const nextHits = await secondSearch;
  const rendered = nextHits.map(hit => `${hit.chunk.text} · ${hit.file_name || `Document ${hit.chunk.doc_id.slice(0, 8)}`} · Page ${hit.chunk.page}`);
  if (JSON.stringify(rendered) !== JSON.stringify(previousSearch)) throw new Error('Invalid Folder changed search results');
  await report('searched; empty/blank/missing-folder IPC preserved seeded index');
  await report('shared lock refused pipeline and mutation IPC overlaps');
  button('Ask').click();
  await pipelineWait(() => document.querySelector('section ol:last-of-type li')?.textContent.startsWith('[1] ') && !button('Ask').disabled);
  await report('answered');
  const secondAnswer = await window.__TAURI_INTERNALS__.invoke('pipeline_ask', { question: query, tokens: progress });
  if (!secondAnswer.text.toLowerCase().includes('thirty days') || !secondAnswer.citations.length) throw new Error('Repeated Ask lost its answer or citations');
  await report('answered again');
  button('Run self-test').click();
})().catch(error => report('ERROR: ' + String(error)));
