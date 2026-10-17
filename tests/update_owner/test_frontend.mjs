// Actual Settings controller, Node stdlib only. No React/browser/bootstrap.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { UpdateController, updateAuthHeaders } from '../../frontend/src/lib/updateProtocol.ts';

assert.equal(process.permission.has('net'), false);
assert.equal(process.permission.has('child'), false);
assert.throws(() => spawnSync('rtk', ['proxy', 'denied-by-permission']), { code: 'ERR_ACCESS_DENIED' });
console.log('GUARD_EVENT: network permission denied; child process spawn denied before execution');
function storage(values = {}) {
  return { getItem: key => values[key] ?? null, setItem: (key, value) => { values[key] = value; }, removeItem: key => { delete values[key]; } };
}
assert.deepEqual(updateAuthHeaders(storage({aim_data_access_token:'current-login',vectoraiz_api_key:'stale-key'})), {Authorization:'Bearer current-login'});
assert.deepEqual(updateAuthHeaders(storage({vectoraiz_api_key:'local-key'})), {'X-API-Key':'local-key'});
assert.deepEqual(updateAuthHeaders(storage()), {});
const response = (status, data) => ({status, ok: status >= 200 && status < 300, json: async () => data});
const requestStorage = 'vectoraiz_update_request';
function fixture(handler, saved = storage(), apiUrl = '') {
  const calls = []; const views = []; let keys = 0;
  const deps = {storage: saved, apiUrl: () => apiUrl, uuid: () => `original-stable-key-${++keys}`,
    changed: view => views.push(view), now: () => 0,
    fetch: async (url, options) => {
      calls.push({url, ...options});
      if (options.method === 'POST') assert.equal(JSON.parse(saved.getItem(requestStorage)).key, JSON.parse(options.body).idempotency_key, 'persist BEFORE delivery');
      return handler(url, options);
    }};
  return {controller: new UpdateController(deps), deps, calls, views, saved, keys: () => keys};
}
function pending(c) {
  assert.equal(c.view.status, 'updating'); assert.equal(c.view.retrySameKey, true);
  assert.match(c.view.message, /Current outcome unknown/);
}
let cases = 0;
for (const [code, body, message] of [
  [403, {detail:'Update permission required.'}, 'Update permission required.'],
  [401, {detail:'Not authenticated. Provide session.'}, 'Not authenticated'],
  [422, {detail:[{loc:['body','target'], msg:'Invalid release target'}]}, 'Invalid release target'],
  [503, {status:'owner_unavailable', message:'Adopt the host before submission.'}, 'owner_unavailable'],
]) {
  const f = fixture(async (_url, o) => o.method === 'POST' ? response(code, body) : response(404, {detail:'operation_not_found'}));
  await f.controller.submit('v2.0.0');
  assert.equal(f.controller.view.status, 'error');
  assert.equal(f.controller.view.retrySameKey, false);
  assert.match(f.controller.view.message, new RegExp(message));
  assert.match(f.controller.view.message, /retry/);
  const refusal = f.controller.view.message;
  await f.controller.observe();
  assert.equal(f.controller.view.message, refusal, 'lookup 404 erased confirmed refusal');
  const reloaded = new UpdateController(f.deps);
  await reloaded.observe();
  assert.equal(reloaded.view.status, 'error'); assert.equal(reloaded.view.message, refusal);
  await reloaded.submit('v2.0.0');
  assert.equal(f.keys(), 2, 'confirmed refusal must allow an actionable new attempt');
  cases++;
}

// Unknown delivery stays on the original key/target across 404, reload and a
// changed latest release. Explicit retry reaches the server for the first time.
{
  let delivered = false; let activation = 0; let op;
  const f = fixture(async (_url, o) => {
    if (o.method === 'POST') {
      if (!delivered) throw new Error('lost_before_delivery');
      activation++; const request = JSON.parse(o.body);
      op = {id:'original-op', target:request.target, phase:'activating', updated:1, deadline:20};
      return response(202, {operation_id:op.id});
    }
    return op ? response(200, {ready:true, current:op}) : response(404, {detail:'operation_not_found'});
  });
  await f.controller.submit('v2.0.0'); pending(f.controller);
  const original = JSON.parse(f.saved.getItem(requestStorage));
  await f.controller.observe(); pending(f.controller);
  const reload = new UpdateController(f.deps);
  await reload.observe(); pending(reload);
  await reload.submit('v3.0.0');
  assert.equal(f.calls.filter(c => c.method === 'POST').length, 1);
  delivered = true; await reload.retry();
  assert.equal(activation, 1); assert.equal(reload.view.correlation.id, 'original-op');
  assert.deepEqual(f.calls.filter(c => c.method === 'POST').map(c => JSON.parse(c.body)), [
    {target:original.target, idempotency_key:original.key}, {target:original.target, idempotency_key:original.key}]);
  assert.equal(f.keys(), 1); cases++;
}

// Lost ack after durable acceptance: a stale 404 does not replace the key;
// same-key repeat and lookup both recover the original operation, no activation.
for (const recover of ['retry', 'lookup']) {
  let posts = 0; let activation = 0; const durable = new Map(); let visible = false;
  const f = fixture(async (_url, o) => {
    if (o.method === 'POST') {
      posts++; const request = JSON.parse(o.body);
      if (!durable.has(request.idempotency_key)) {
        durable.set(request.idempotency_key, {id:'durable-original-op', target:request.target, phase:'activating', updated:1, deadline:20}); activation++;
      }
      if (posts === 1) throw new Error('lost_ack_after_commit');
      visible = true; return response(202, {operation_id:'durable-original-op'});
    }
    const [key, op] = [...durable][0];
    return visible ? response(200, {ready:true, current:op, keys:{[key]:{code:202, id:op.id}}}) : response(404, {detail:'operation_not_found'});
  });
  await f.controller.submit('v2.0.0'); await f.controller.observe(); pending(f.controller);
  const reload = new UpdateController(f.deps);
  if (recover === 'retry') await reload.retry();
  else { visible = true; await reload.observe(); }
  assert.equal(reload.view.correlation.id, 'durable-original-op'); assert.equal(f.keys(), 1); assert.equal(activation, 1);
  await reload.retry(); assert.equal(activation, 1); cases++;
}

// Ambiguous server/ack responses are never classified as a safe refusal.
for (const [code, body] of [[504, {status:'acceptance_unknown'}], [503, {status:'acceptance_unknown'}], [500, {}], [202, {}]]) {
  const f = fixture(async (_url, o) => o.method === 'POST' ? response(code, body) : response(404, {}));
  await f.controller.submit('v2.0.0'); pending(f.controller);
  const key = f.controller.view.correlation.key;
  await f.controller.observe(); assert.equal(f.controller.view.correlation.key, key); pending(f.controller);
  assert.equal(f.keys(), 1); cases++;
}

// A lookup already in flight cannot overwrite a subsequent confirmed refusal.
{
  let release;
  const f = fixture(async (_url, o) => o.method === 'POST' ? response(403, {detail:'Update permission required'}) :
    new Promise(resolve => { release = resolve; }));
  const lookup = f.controller.observe();
  await f.controller.submit('v2.0.0');
  release(response(404, {})); await lookup;
  assert.equal(f.controller.view.status, 'error'); assert.match(f.controller.view.message, /Update permission required/); cases++;
}

// An auth refusal of a retry does not disprove the ORIGINAL unknown delivery.
{
  let posts = 0;
  const f = fixture(async (_url, o) => {
    if (o.method !== 'POST') return response(404, {});
    if (++posts === 1) throw new Error('lost');
    return response(401, {detail:'Session expired; sign in again'});
  });
  await f.controller.submit('v2.0.0'); const key = f.controller.view.correlation.key;
  await f.controller.retry(); pending(f.controller);
  assert.match(f.controller.view.message, /Session expired/); assert.equal(f.controller.view.correlation.key, key);
  await f.controller.observe(); pending(f.controller); cases++;
}

// Fake browser/server authentication boundary: cookies travel to the configured
// API origin via include, headers stay supported, and the server decides scopes.
for (const origin of ['', 'https://configured-api.invalid']) {
  for (const role of ['cookie-admin', 'cookie-user', 'absent', 'bearer', 'api-key']) {
    const saved = storage(role === 'bearer' ? {aim_data_access_token:'token'} : role === 'api-key' ? {vectoraiz_api_key:'key'} : {});
    let posts = 0; let lookups = 0;
    const f = fixture(async (url, o) => {
      assert.equal(o.credentials, 'include'); assert.ok(url.startsWith(origin + '/api/version/update'));
      const authenticated = role !== 'absent';
      const admin = role !== 'cookie-user' && authenticated;
      if (role === 'bearer') assert.equal(o.headers.Authorization, 'Bearer token');
      if (role === 'api-key') assert.equal(o.headers['X-API-Key'], 'key');
      if (role.startsWith('cookie') || role === 'absent') assert.equal(o.headers.Authorization, undefined);
      if (o.method === 'POST') {
        posts++;
        return !authenticated ? response(401, {detail:'Not authenticated'}) : !admin ? response(403, {detail:'Update permission required'}) : response(202, {operation_id:'cookie-or-header-op'});
      }
      lookups++;
      return authenticated ? response(200, {ready:true, current:null}) : response(401, {detail:'Not authenticated'});
    }, saved, origin);
    await f.controller.observe(); await f.controller.submit('v2.0.0');
    assert.equal(posts, 1, 'empty local headers suppressed POST'); assert.ok(lookups >= 1, 'empty local headers suppressed status');
    if (role === 'absent' || role === 'cookie-user') assert.equal(f.controller.view.status, 'error');
    else assert.equal(f.controller.view.correlation.id, 'cookie-or-header-op');
    cases++;
  }
}

// Stale, unrelated and failed operations cannot become a successful update.
{
  let data;
  const f = fixture(async (_url, o) => o.method === 'POST' ? response(202, {operation_id:'op'}) : response(200, data));
  const op = {id:'op', target:'v2.0.0', phase:'succeeded', updated:1, deadline:20};
  data = {ready:true, current:{...op, id:'historical'}};
  await f.controller.submit('v2.0.0'); assert.notEqual(f.controller.view.status, 'done');
  data = {ready:true, stale:true, current:op}; await f.controller.observe(); assert.equal(f.controller.view.status, 'updating');
  data = {ready:true, current:{...op, phase:'recovery_required', code:'migration_failed'}};
  await f.controller.observe(); assert.equal(f.controller.view.status, 'error'); assert.match(f.controller.view.message, /migration_failed/);
  data = {ready:true, current:op}; await f.controller.observe(); assert.equal(f.controller.view.status, 'done');
  assert.equal(f.controller.view.retrySameKey, false); cases++;
}
const page = readFileSync(new URL('../../frontend/src/pages/SettingsPage.tsx', import.meta.url), 'utf8');
assert.match(page, /new UpdateController/);
assert.match(page, /updateController.current\?\.retry\(\)/);
assert.match(page, /Check update status/);
assert.doesNotMatch(page.slice(page.indexOf('const updateController'), page.indexOf('const createLocalKey')), /setTimeout|location\.reload|Object\.keys\(headers\)/);
console.log(`PASS: ${cases} actual-controller scenarios plus auth header checks; fake server scopes only; real browser/FastAPI NOT_RUN`);
