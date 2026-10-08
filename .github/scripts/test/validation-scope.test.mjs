import assert from 'node:assert/strict';
import { execFileSync, spawnSync } from 'node:child_process';
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { needsFullValidation } from '../validation-scope.mjs';

test('only explicitly allowlisted documentation PRs skip code validation', () => {
  for (const paths of [['README.md'], ['CONTRIBUTING.md', 'AGENTS.md'], ['README.md', 'CONTRIBUTING.md', 'AGENTS.md']]) {
    assert.equal(needsFullValidation('pull_request', paths), false);
    for (const event of ['push', 'workflow_dispatch', '', undefined]) {
      assert.equal(needsFullValidation(event, paths), true);
    }
  }
  for (const path of ['src/main.rs', 'frontend/src/app.tsx', 'Cargo.lock', 'LICENSE.md', 'NOTICE.md',
    'CHANGELOG.md', '.github/workflows/validation.yml', '.github/scripts/validation-scope.mjs',
    'frontend/public/assets/licenses/manrope.txt', 'README.md.old', 'readme.md', 'README.md\nsrc/main.rs']) {
    assert.equal(needsFullValidation('pull_request', ['README.md', path]), true, path);
  }
  assert.equal(needsFullValidation('pull_request', []), true);
});

test('scope CLI compares the tested merge and cannot hide a code-to-doc rename or missing base', () => {
  const directory = mkdtempSync(join(tmpdir(), 'tdm-scope-'));
  const script = fileURLToPath(new URL('../validation-scope.mjs', import.meta.url));
  const env = { ...process.env, GITHUB_EVENT_NAME: 'pull_request',
    GIT_AUTHOR_NAME: 'Test', GIT_AUTHOR_EMAIL: 'test@example.com',
    GIT_COMMITTER_NAME: 'Test', GIT_COMMITTER_EMAIL: 'test@example.com' };
  const git = (...args) => execFileSync('git', args, { cwd: directory, env, stdio: 'pipe' });
  const scope = () => execFileSync(process.execPath, [script], { cwd: directory, env, encoding: 'utf8', stdio: 'pipe' }).trim();
  try {
    git('init', '-b', 'main');
    writeFileSync(join(directory, 'README.md'), 'guide\n');
    writeFileSync(join(directory, 'code.rs'), 'fn main() {}\n');
    git('add', '.');
    git('commit', '-m', 'base');
    assert.throws(scope);
    git('switch', '-c', 'docs');
    writeFileSync(join(directory, 'README.md'), 'updated guide\n');
    git('commit', '-am', 'docs');
    git('switch', 'main');
    git('merge', '--no-ff', 'docs', '-m', 'merge');
    assert.equal(scope(), 'full=false');
    env.GITHUB_EVENT_NAME = 'push';
    assert.equal(scope(), 'full=true');
    env.GITHUB_EVENT_NAME = 'pull_request';
    git('mv', '-f', 'code.rs', 'README.md');
    git('commit', '-am', 'rename');
    assert.equal(scope(), 'full=true');
  } finally { rmSync(directory, { recursive: true, force: true }); }
});

test('the Linux workflow completion gate rejects failed or unexpectedly skipped checks', { skip: process.platform === 'win32' }, () => {
  const workflow = readFileSync(new URL('../../workflows/validation.yml', import.meta.url), 'utf8');
  const script = workflow.split('  complete:\n')[1].split('        run: |\n')[1];
  const run = values => spawnSync('bash', ['-e', '-o', 'pipefail', '-c', script], {
    env: { ...process.env, GITHUB_STEP_SUMMARY: '/dev/null', ...values },
  }).status;
  for (const full of ['true', 'false']) {
    const expected = full === 'true' ? 'success' : 'skipped';
    const values = { SCOPE_RESULT: 'success', FULL: full, TEST_RESULT: expected, BROWSER_RESULT: expected, DOCKER_RESULT: expected, DASHBOARD_RESULT: expected, DESKTOP_RESULT: expected };
    assert.equal(run(values), 0);
    for (const name of ['SCOPE_RESULT', 'TEST_RESULT', 'BROWSER_RESULT', 'DOCKER_RESULT', 'DASHBOARD_RESULT', 'DESKTOP_RESULT']) {
      for (const result of ['success', 'skipped', 'failure', 'cancelled', '']) {
        if (result === values[name]) continue;
        assert.notEqual(run({ ...values, [name]: result }), 0, `${full}: ${name}=${result}`);
      }
    }
    assert.notEqual(run({ ...values, FULL: '' }), 0);
  }
});
