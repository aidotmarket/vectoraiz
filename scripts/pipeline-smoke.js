(async () => {
  const wait = async (predicate) => {
    const deadline = Date.now() + 240000;
    while (!predicate()) {
      if (document.querySelector('[role="alert"]')) throw new Error(document.querySelector('[role="alert"]').textContent);
      if (Date.now() > deadline) throw new Error('Pipeline UI timed out');
      await new Promise(resolve => setTimeout(resolve, 100));
    }
  };
  const button = label => [...document.querySelectorAll('button')].find(b => b.textContent === label);
  await wait(() => button('Index folder'));
  button('Index folder').click();
  await wait(() => document.querySelector('[role="status"]').textContent.includes('20/20') && !button('Search').disabled);
  button('Search').click();
  await wait(() => document.querySelector('li')?.textContent.includes('thirty days') && !button('Ask').disabled);
  button('Ask').click();
  await wait(() => document.querySelector('section').textContent.includes('[1] Document') && !button('Ask').disabled);
  button('Run self-test').click();
})();
