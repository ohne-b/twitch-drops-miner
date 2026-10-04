import childProcess from 'node:child_process';
import { createHash } from 'node:crypto';
import { pathToFileURL } from 'node:url';
import { isPrerelease, releaseImages } from './release.mjs';

const repository = 'ohne-b/twitch-drops-miner';
const image = `ghcr.io/${repository}`;

export function mirrorImage(version) {
  const source = version === 'edge' ? `${image}:edge` : releaseImages(version)[0];
  const { DOCKERHUB_IMAGE, DOCKERHUB_USERNAME, DOCKERHUB_TOKEN } = process.env;
  if (!/^[a-z0-9][a-z0-9_-]*\/[a-z0-9]+(?:[._-][a-z0-9]+)*$/.test(DOCKERHUB_IMAGE ?? '') ||
      !DOCKERHUB_USERNAME || !DOCKERHUB_TOKEN) {
    throw new Error('Configure DOCKERHUB_IMAGE and DOCKERHUB_USERNAME variables and the DOCKERHUB_TOKEN secret.');
  }
  if (process.env.GITHUB_REPOSITORY !== repository || process.env.GITHUB_REF !== 'refs/heads/main') {
    throw new Error('Mirroring requires the canonical main workflow.');
  }
  const run = (command, args, options = {}) => childProcess.execFileSync(command, args, {
    encoding: 'utf8', stdio: ['ignore', 'pipe', 'inherit'], ...options,
  });
  const release = path => JSON.parse(run('gh', ['api', `repos/${repository}/releases/${path}`]));
  if (version !== 'edge') {
    const published = release(`tags/v${version}`);
    if (published.draft || published.tag_name !== `v${version}` || published.prerelease !== isPrerelease(version)) {
      throw new Error('Mirror only an existing published release or edge image.');
    }
  }
  const manifest = ref => run('skopeo', ['inspect', '--raw', `docker://${ref}`]);
  const digest = raw => `sha256:${createHash('sha256').update(raw).digest('hex')}`;
  const raw = manifest(source);
  const platforms = JSON.parse(raw).manifests?.map(item => `${item.platform?.os}/${item.platform?.architecture}`) ?? [];
  if (!['linux/amd64', 'linux/arm64'].every(platform => platforms.includes(platform))) {
    throw new Error('The source image must contain both linux/amd64 and linux/arm64.');
  }
  const sourceDigest = digest(raw);
  const target = `docker.io/${DOCKERHUB_IMAGE}`;
  run('skopeo', ['login', '--username', DOCKERHUB_USERNAME, '--password-stdin', 'docker.io'], {
    input: DOCKERHUB_TOKEN, stdio: ['pipe', 'inherit', 'inherit'],
  });
  const copy = tag => {
    run('skopeo', ['copy', '--all', '--preserve-digests', '--retry-times', '3',
      `docker://${image}@${sourceDigest}`, `docker://${target}:${tag}`], { stdio: 'inherit' });
    if (digest(manifest(`${target}:${tag}`)) !== sourceDigest) {
      throw new Error(`The mirrored ${tag} image differs from GHCR.`);
    }
  };
  copy(source.slice(source.lastIndexOf(':') + 1));
  // Backfilling an older release must not move latest backwards.
  if (version !== 'edge' && !isPrerelease(version) && release('latest').tag_name === `v${version}` &&
      digest(manifest(`${image}:latest`)) === sourceDigest) {
    copy('latest');
  }
  console.log(`Mirrored ${source} to ${target} (${sourceDigest}).`);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  mirrorImage(process.argv[2]);
}
