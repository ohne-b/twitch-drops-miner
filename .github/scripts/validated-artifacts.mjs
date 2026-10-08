import childProcess from 'node:child_process';
export const repository = 'ohne-b/twitch-drops-miner';
export const run = (command, args, options = {}) => childProcess.execFileSync(command, args, {
  encoding: 'utf8', stdio: ['ignore', 'pipe', 'inherit'], ...options,
});

export function validatedRun(runs, sha) {
  const run = runs.find(run => run.headSha === sha && run.headBranch === 'main' &&
    ['push', 'workflow_dispatch'].includes(run.event));
  if (!run || run.status !== 'completed' || run.conclusion !== 'success' ||
      !Number.isSafeInteger(run.databaseId) || run.databaseId <= 0 ||
      !Number.isSafeInteger(run.attempt) || run.attempt <= 0) {
    throw new Error('Complete validation on the exact current main commit before publishing.');
  }
  return { id: run.databaseId, attempt: run.attempt };
}

export function validationSource() {
  const sha = process.env.GITHUB_SHA;
  if (process.env.GITHUB_REPOSITORY !== repository || process.env.GITHUB_REF !== 'refs/heads/main' ||
      !/^[a-f0-9]{40}$/.test(sha ?? '') || !process.env.GH_TOKEN || !process.env.GITHUB_ACTOR) {
    throw new Error('Publishing requires the canonical main workflow and its scoped token.');
  }
  const checkMain = () => {
    if (run('git', ['ls-remote', 'origin', 'refs/heads/main']).split('\t')[0] !== sha) {
      throw new Error('Main advanced. Validate and publish the current main commit.');
    }
  };
  checkMain();
  const currentValidation = () => validatedRun(JSON.parse(run('gh', ['run', 'list', '--repo', repository,
    '--workflow', 'validation.yml', '--branch', 'main', '--commit', sha, '--limit', '20',
    '--json', 'databaseId,attempt,headSha,headBranch,event,status,conclusion'])), sha);
  const { id, attempt } = currentValidation();
  const check = () => {
    checkMain();
    const current = currentValidation();
    if (current.id !== id || current.attempt !== attempt) {
      throw new Error('Validation changed during publication. Restart with its tested artifacts.');
    }
  };
  return { id, attempt, sha, check, download(names, directory) {
    run('gh', ['run', 'download', String(id), '--repo', repository, '--dir', directory,
      ...names.flatMap(name => ['--name', name])]);
  }};
}
