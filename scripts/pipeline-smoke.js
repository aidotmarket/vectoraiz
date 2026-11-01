const report = checkpoint => window.__TAURI_INTERNALS__.invoke('self_test', { checkpoint });
(async () => {
  await report('started');
  const button = label => [...document.querySelectorAll('button')].find(b => b.textContent === label);
  const downloadButton = () => [...document.querySelectorAll('button')].find(b => b.textContent.startsWith('Download search model ('));
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
  const ipc = (command, args) => window.__TAURI_INTERNALS__.invoke(command, args);
  const statusWait = async (predicate, budget = 8000) => {
    const deadline = Date.now() + budget;
    while (Date.now() < deadline) {
      const status = await ipc('models_status');
      if (predicate(status)) return status;
      await new Promise(resolve => setTimeout(resolve, 10));
    }
    throw new Error('Model status timed out: ' + predicate);
  };
  const query = 'What is the orbital archive retention policy?';
  const progress = '__CHANNEL__:' + window.__TAURI_INTERNALS__.transformCallback(() => {});
  if (window.__VZ_SMOKE__.migration) {
    if (window.__VZ_SMOKE__.migration === 'import') {
      const imported = await ipc('models_import',{folder:window.__VZ_SMOKE__.importFolder});
      if (imported.length !== 3 || imported.some(file => file.result !== 'imported')) throw new Error('Three-pin import failed: '+JSON.stringify(imported));
      const status = await ipc('models_status');
      if (!status.ready || status.answer.state !== 'none' || status.target) throw new Error('Import did not leave a search-ready optional answer selection');
      await ipc('pipeline_index',{folder:window.__VZ_SMOKE__.fixture,progress});
      const skipped = await ipc('models_import',{folder:window.__VZ_SMOKE__.importFolder});
      if (skipped.length !== 3 || skipped.some(file => file.result !== 'already installed')) throw new Error('Import rewrote verified models');
      if (!(await ipc('pipeline_search',{query}))[0]?.chunk.text.includes('thirty days')) throw new Error('Import changed Search');
      await ipc('answer_add',{tierIndex:0});
      if ((await ipc('pipeline_ask',{question:query,tokens:progress})).kind !== 'answer') throw new Error('Imported answer could not be activated');
      await report('import: all three distinct pins, verified skips, Search and Ask passed');
      await ipc('self_test');
      return;
    }
    const status = await ipc('models_status');
    if (status.offer.length !== 2 || status.offer[0].pin.sha256 === status.offer[1].pin.sha256 || status.offer[0].pin.file === status.offer[1].pin.file) throw new Error('Distinct answer fixture pins not offered');
    if (status.target || status.needs_reindex || status.answer.pin?.sha256 !== status.offer[0].pin.sha256) throw new Error('Migration changed the selection or index');
    const hits = await ipc('pipeline_search', {query});
    if (!hits[0]?.chunk.text.includes('thirty days')) throw new Error('Migration Search lost seeded hits');
    await report('migration: searched before indexing with adopted tier 0');
    if (window.__VZ_SMOKE__.migration === 'switch') {
      await report('hold-next-transfer');
      await report('hold-next-search');
      window.__VZ_SMOKE__.searchLocked = false;
      const heldSearch = ipc('pipeline_search', {query});
      await wait(() => window.__VZ_SMOKE__.searchLocked, 2000);
      const switching = ipc('answer_switch', {tierIndex:1});
      await statusWait(s => ['checking','downloading'].includes(s.target?.state));
      await report('release-transfer');
      await statusWait(s => s.target?.state === 'verified' && s.answer.pin.sha256 === s.offer[0].pin.sha256);
      let refused = false;
      try { await ipc('delete_unused_model', {sha256:status.offer[0].pin.sha256}); }
      catch (error) { if (String(error) !== 'A download is running') throw error; refused = true; }
      if (!refused) throw new Error('Delete raced pending activation');
      await report('release-search');
      await heldSearch; await switching;
      await statusWait(s => s.answer.state === 'ready' && s.answer.pin.sha256 === s.offer[1].pin.sha256 && !s.target);
      const answer = await ipc('pipeline_ask', {question:query,tokens:progress});
      if (answer.kind !== 'answer' || !answer.citations.length || !/(?:thirty|30) days/i.test(answer.text)) throw new Error('Switched answer lost citations');
      await report('migration: verified tier 1 answered');
      // Independent transfer exclusion, with no Search held and no activation wait.
      await report('hold-next-transfer');
      const adding = ipc('answer_add', {tierIndex:0});
      await statusWait(s => s.target?.state === 'downloading');
      if (!(await ipc('pipeline_search',{query}))[0]?.chunk.text.includes('thirty days')) throw new Error('Transfer blocked Search');
      const currentAnswer = await ipc('pipeline_ask',{question:query,tokens:progress});
      if (currentAnswer.kind !== 'answer' || !currentAnswer.citations.length) throw new Error('Transfer blocked the current answer');
      for (const [command,args] of [['answer_switch',{tierIndex:0}],['models_import',{folder:window.__VZ_SMOKE__.fixture}]]) {
        let refused = false;
        try { await ipc(command,args); } catch (error) { if (String(error) !== 'Another model or index operation is running') throw error; refused = true; }
        if (!refused) throw new Error('Concurrent transfer was accepted');
      }
      await report('release-transfer'); await adding;
      // Real IPC Index + transfer, cancelling each side independently.
      for (const kind of ['transfer','pipeline']) {
        await report('hold-next-index');
        window.__VZ_SMOKE__.indexLocked = false;
        const indexing = ipc('pipeline_index',{folder:window.__VZ_SMOKE__.fixture,progress}).then(() => 'complete', error => String(error));
        await wait(() => window.__VZ_SMOKE__.indexLocked,2000);
        await report('hold-next-transfer');
        const transfer = ipc('answer_add',{tierIndex:0}).then(() => 'complete',error => String(error));
        await statusWait(s => s.target?.state === 'downloading');
        await ipc('models_cancel',{kind});
        await report('release-index');
        await report('release-transfer');
        const [indexed,transferred] = await Promise.all([indexing,transfer]);
        if (kind === 'transfer' ? indexed !== 'complete' || !transferred.toLowerCase().includes('cancelled') : transferred !== 'complete' || !indexed.toLowerCase().includes('cancel')) throw new Error('Cancellation crossed operation kinds: '+JSON.stringify({kind,indexed,transferred}));
        if (!(await ipc('pipeline_search',{query}))[0]?.chunk.text.includes('thirty days')) throw new Error('Cancellation blocked Search');
        if ((await ipc('pipeline_ask',{question:query,tokens:progress})).kind !== 'answer') throw new Error('Cancellation blocked the current answer');
        await report('independent cancellation: '+kind);
      }
      const freed = await ipc('delete_unused_model',{sha256:status.offer[0].pin.sha256});
      if (freed !== status.offer[0].pin.size) throw new Error('Delete freed wrong bytes');
      if (!(await ipc('pipeline_search',{query}))[0]?.chunk.text.includes('thirty days')) throw new Error('Delete changed Search');
      for (const sha256 of [status.search.pin.sha256,status.offer[1].pin.sha256]) {
        let refused = false;
        try { await ipc('delete_unused_model',{sha256}); } catch { refused = true; }
        if (!refused) throw new Error('Deleted search or active answer model');
      }
      await ipc('answer_turn_off');
      if ((await ipc('pipeline_ask',{question:query,tokens:progress})).kind !== 'no_answer') throw new Error('Turn off did not remove answer selection');
      const removed = await ipc('delete_unused_model',{sha256:status.offer[1].pin.sha256});
      if (removed !== status.offer[1].pin.size) throw new Error('Turned-off model deletion freed wrong bytes');
      if (!(await ipc('pipeline_search',{query}))[0]?.chunk.text.includes('thirty days')) throw new Error('Turning off changed Search');
      // Cancel an actual acquisition after bytes arrive, then remove only its
      // resumable partial through the same unused-model command.
      const cancelledAdd = ipc('answer_add',{tierIndex:0}).then(() => 'complete',error => String(error));
      await statusWait(s => (s.target?.progress?.downloaded ?? 0) > 0 || s.target?.progress?.stage === 'checking',240000);
      await ipc('models_cancel',{kind:'transfer'});
      if (!(await cancelledAdd).toLowerCase().includes('cancelled')) throw new Error('Acquisition did not cancel');
      const partial = (await ipc('models_status')).files.find(file => file.sha256 === status.offer[0].pin.sha256)?.partial_bytes;
      if (!(partial > 0)) throw new Error('Cancellation did not preserve resumable bytes');
      if (await ipc('delete_unused_model',{sha256:status.offer[0].pin.sha256}) !== partial) throw new Error('Partial delete freed wrong bytes');
      if (!(await ipc('pipeline_search',{query}))[0]?.chunk.text.includes('thirty days')) throw new Error('Partial deletion changed Search');
      await report('migration: transfer exclusion, delete refusals, turn off and unused model deletion passed');
    }
    await ipc('self_test');
    return;
  }
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
  const repeatedHits = await window.__TAURI_INTERNALS__.invoke('pipeline_search', { query });
  if (!repeatedHits[0]?.chunk.text.includes('thirty days')) throw new Error('Repeated Search lost its result');
  await report('searched again');
  await report('guard-before');
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
  ]);
  await report('release-search');
  for (const result of overlaps) {
    if (result.status !== 'rejected' || String(result.reason) !== 'Another model or index operation is running') throw new Error('Overlapping operation was not refused');
  }
  const nextHits = await secondSearch;
  const rendered = nextHits.map(hit => `${hit.chunk.text} · ${hit.file_name || `Document ${hit.chunk.doc_id.slice(0, 8)}`} · Page ${hit.chunk.page}`);
  if (JSON.stringify(rendered) !== JSON.stringify(previousSearch)) throw new Error('Invalid Folder changed search results');
  await report('searched; empty/blank/missing-folder IPC preserved seeded index');
  await report('inference lock refused Search and Ask overlaps');
  if (window.__VZ_SMOKE__.acquisition) {
    const searchOnly = await ipc('models_status');
    if (!searchOnly.ready || searchOnly.answer.state !== 'none' || searchOnly.files.filter(file => file.installed_bytes > 0).length !== 1) throw new Error('Fresh install acquired more than the search model');
    const noAnswer = await ipc('pipeline_ask',{question:query,tokens:progress});
    if (noAnswer.kind !== 'no_answer') throw new Error('Search-only Ask must return no_answer');
    await report('search-only: Index and Search passed; Ask returned no_answer');
  }
  let changed = false;
  const eventHandler = window.__TAURI_INTERNALS__.transformCallback(() => {changed = true;});
  const eventId = await ipc('plugin:event|listen',{event:'models_changed',target:{kind:'Any'},handler:eventHandler});
  await ipc('answer_add',{tierIndex:0});
  await statusWait(status => status.answer.state === 'ready',240000);
  if (window.__VZ_SMOKE__.acquisition && !changed) throw new Error('answer_add emitted no models_changed');
  await ipc('plugin:event|unlisten',{event:'models_changed',eventId});
  button('Ask').click();
  await pipelineWait(() => document.querySelector('section ol:last-of-type li')?.textContent.startsWith('[1] ') && !button('Ask').disabled);
  await report('answered');
  const secondAnswer = await window.__TAURI_INTERNALS__.invoke('pipeline_ask', { question: query, tokens: progress });
  if (secondAnswer.kind !== 'answer' || !secondAnswer.text.toLowerCase().includes('thirty days') || !secondAnswer.citations.length) throw new Error('Repeated Ask lost its answer or citations');
  await report('answered again');
  button('Run self-test').click();
})().catch(error => report('ERROR: ' + String(error)));
