// Isolated Node stdlib check; run under --permission without network/process grants.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { updateAuthHeaders } from '../../frontend/src/lib/updateProtocol.ts';

assert.equal(process.permission.has('net'), false);
assert.equal(process.permission.has('child'), false);
assert.throws(() => spawnSync('rtk', ['proxy', 'denied-by-permission']), { code: 'ERR_ACCESS_DENIED' });
console.log('GUARD_EVENT: network permission denied; child process spawn denied before execution');
const storage = values => ({ getItem: key => values[key] ?? null });
assert.deepEqual(updateAuthHeaders(storage({aim_data_access_token:'current-login',vectoraiz_api_key:'stale-key'})), {Authorization:'Bearer current-login'});
assert.deepEqual(updateAuthHeaders(storage({vectoraiz_api_key:'local-key'})), {'X-API-Key':'local-key'});
assert.deepEqual(updateAuthHeaders(storage({})), {});
const page = readFileSync(new URL('../../frontend/src/pages/SettingsPage.tsx', import.meta.url), 'utf8');
assert.match(page, /import \{ updateAuthHeaders \} from "@\/lib\/updateProtocol"/);
assert.match(page, /JSON\.stringify\(\{ target, idempotency_key: correlation\.key \}\)/);
assert.ok(page.indexOf('localStorage.setItem("vectoraiz_update_request", JSON.stringify(correlation));',page.indexOf('const triggerUpdate')) < page.indexOf('method: "POST"',page.indexOf('const triggerUpdate')));
assert.doesNotMatch(page.slice(page.indexOf('const triggerUpdate'), page.indexOf('const createLocalKey')), /setTimeout|location\.reload/);
assert.match(page, /if \(data\.stale\)/);
assert.match(page, /op\.phase === "succeeded"/);
console.log('PASS: actual bearer/local-key helper and updater HTTP/persistence/truthful observation source contracts');
