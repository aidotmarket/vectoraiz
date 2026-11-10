import assert from 'node:assert/strict';

export async function assertLayout(browser, selectView) {
    for (const width of [1280, 680, 360]) {
      await browser.setWindowSize(width, 800);
      for (const view of ['settings', 'ai-apps']) {
        await selectView(view);
        const geometry = await browser.execute(() => {
          const content = document.querySelector('.content');
          const active = document.querySelector('.view:not([hidden])');
          const filler = document.createElement('div');
          filler.style.height = '2000px';
          active.append(filler);
          const bounds = element => { const r = element.getBoundingClientRect(); return {top:r.top,bottom:r.bottom,left:r.left,right:r.right}; };
          const before = Array.from(document.querySelectorAll('.sidebar nav a, .sidebar footer')).map(bounds);
          content.scrollTop = content.scrollHeight;
          const after = Array.from(document.querySelectorAll('.sidebar nav a, .sidebar footer')).map(bounds);
          const rows = Array.from(active.querySelectorAll('.actions')).filter(row => row.getBoundingClientRect().width > 0).map(row => {
            const buttons = Array.from(row.children).filter(child => child.tagName === 'BUTTON');
            return {gap:getComputedStyle(row).gap,display:getComputedStyle(row).display,wrap:getComputedStyle(row).flexWrap,width:row.clientWidth,buttons:buttons.map(bounds)};
          });
          const result = {before,after,rows,scroll:content.scrollTop,pageScroll:window.scrollY,height:innerHeight,width:innerWidth,sidebar:bounds(document.querySelector('.sidebar'))};
          filler.remove(); content.scrollTop = 0;
          return result;
        });
        assert.equal(geometry.width, width);
        assert.ok(geometry.scroll > 0, JSON.stringify(geometry));
        assert.equal(geometry.pageScroll, 0);
        assert.deepEqual(geometry.before, geometry.after);
        assert.equal(geometry.after.length, 5);
        for (const item of geometry.after) {
          assert.ok(item.top >= 0 && item.bottom <= geometry.height, JSON.stringify(geometry));
        }
        if (geometry.width >= 720) assert.equal(geometry.sidebar.bottom, geometry.height);
        else assert.equal(geometry.sidebar.top, 0);
        for (const row of geometry.rows) {
          assert.equal(row.display, 'flex'); assert.equal(row.gap, '12px'); assert.equal(row.wrap, 'wrap');
          if (row.buttons.length > 1 && row.buttons.reduce((sum, button) => sum + button.right - button.left, 0) + 12 * (row.buttons.length - 1) <= row.width) {
            assert.ok(row.buttons.every(button => button.top === row.buttons[0].top), JSON.stringify(row));
          }
          for (const [index, button] of row.buttons.entries()) {
            assert.ok(button.left >= 0 && button.right <= geometry.width);
            const previous = row.buttons[index - 1];
            if (previous && previous.top === button.top) assert.equal(button.left - previous.right, 12);
          }
        }
      }
    }
    await browser.setWindowSize(1280, 800);
}

export async function assertDialogActions(browser) {
  const row = await browser.execute(() => {
    const dialog = Array.from(document.querySelectorAll('[role="dialog"]')).find(element => element.getBoundingClientRect().width);
    const group = dialog.matches('.actions') ? dialog : dialog.querySelector('.setup-fields');
    const rect = group.getBoundingClientRect();
    return {display:getComputedStyle(group).display,gap:getComputedStyle(group).gap,wrap:getComputedStyle(group).flexWrap,width:rect.width,viewport:innerWidth,buttons:Array.from(group.children).filter(element => element.tagName === 'BUTTON').map(element => {
      const r = element.getBoundingClientRect(); return {top:r.top,left:r.left,right:r.right};
    })};
  });
  assert.equal(row.display, 'flex'); assert.equal(row.gap, '12px'); assert.equal(row.wrap, 'wrap');
  assert.equal(row.buttons.length, 2);
  const [first, second] = row.buttons;
  if (first.right - first.left + second.right - second.left + 12 <= row.width) {
    assert.equal(first.top, second.top); assert.equal(second.left - first.right, 12);
  } else assert.ok(second.top > first.top);
  for (const button of row.buttons) assert.ok(button.left >= 0 && button.right <= row.viewport);
}
