import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';

test('only the local main window can open HTTP(S) links in the default browser', () => {
  const capability = JSON.parse(readFileSync(new URL('../../../desktop/capabilities/main.json', import.meta.url), 'utf8'));
  assert.deepEqual(capability.windows, ['main']);
  assert.equal(capability.remote, undefined);
  assert.notEqual(capability.local, false);
  const opener = capability.permissions.filter(permission =>
    (typeof permission === 'string' ? permission : permission.identifier).startsWith('opener:'));
  assert.deepEqual(opener, [{
    identifier: 'opener:allow-open-url',
    allow: [{ url: 'https://*' }, { url: 'http://*' }],
  }]);
});
