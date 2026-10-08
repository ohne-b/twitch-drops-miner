import { run, validationSource } from './validated-artifacts.mjs';
export { validatedRun } from './validated-artifacts.mjs';
import { mkdtempSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';
import { releaseImages, validateVersion } from './release.mjs';

const repository = 'ohne-b/twitch-drops-miner';
const image = `ghcr.io/${repository}`;
const architectures = ['amd64', 'arm64'];

export function validatedImage(info, arch, version, sha) {
  if (info.Os !== 'linux' || info.Architecture !== arch ||
      info.Labels?.['org.opencontainers.image.version'] !== version ||
      info.Labels?.['org.opencontainers.image.revision'] !== sha ||
      !/^sha256:[a-f0-9]{64}$/.test(info.Digest)) {
    throw new Error(`The ${arch} artifact does not match this release's platform, version and commit.`);
  }
  return `${image}@${info.Digest}`;
}

export function publishImages(version, target) {
  validateVersion(version);
  if (![releaseImages(version)[0], `${image}:edge`].includes(target)) {
    throw new Error('Only the requested GHCR version or edge tag may be published.');
  }
  const source = validationSource();
  const { id, sha } = source;
  const checkCurrent = source.check;
  const directory = mkdtempSync(join(tmpdir(), 'tdm-images-'));
  try {
    source.download(architectures.map(arch => `image-${arch}`), directory);
  } catch {
    throw new Error(`Image artifacts for validation run ${id} are unavailable. Rerun validation on main; publishing never rebuilds them.`);
  }
  // Validate both artifacts before authenticating to the registry or publishing either one.
  const images = architectures.map(arch => {
    const source = `oci-archive:${join(directory, `image-${arch}`, 'image.tar')}`;
    const info = JSON.parse(run('skopeo', ['--override-arch', arch, 'inspect', source]));
    return { source, destination: validatedImage(info, arch, version, sha) };
  });
  checkCurrent();
  run('docker', ['login', 'ghcr.io', '-u', process.env.GITHUB_ACTOR, '--password-stdin'], {
    input: process.env.GH_TOKEN, stdio: ['pipe', 'inherit', 'inherit'],
  });
  for (const { source, destination } of images) {
    run('skopeo', ['copy', '--preserve-digests', '--retry-times', '3', source, `docker://${destination}`], { stdio: 'inherit' });
  }
  checkCurrent();
  run('docker', ['buildx', 'imagetools', 'create', '--tag', target,
    ...images.map(({ destination }) => destination)], { stdio: 'inherit' });
  console.log(`Published ${target} from validation run ${id} (${sha}).`);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  publishImages(...process.argv.slice(2));
}
