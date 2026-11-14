import { readdirSync, readFileSync, existsSync, writeFileSync } from 'node:fs';
import { join, basename } from 'node:path';
import { pathToFileURL } from 'node:url';

export function updaterManifest(root, repository, tag, now = new Date()) {
  if (!/^[\w.-]+\/[\w.-]+$/.test(repository) || !/^v\d+\.\d+\.\d+(?:-[\w.-]+)?$/.test(tag)) {
    throw new Error('Invalid release repository or version tag');
  }
  const platforms = {};
  function visit(dir) {
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      const file = join(dir, entry.name);
      if (entry.isDirectory()) { visit(file); continue; }
      if (!file.endsWith('.sig')) continue;
      const artifact = file.slice(0, -4);
      if (!existsSync(artifact)) throw new Error(`Missing signed artifact: ${artifact}`);
      const name = basename(artifact);
      const keys = name.endsWith('setup.exe') ? ['windows-x86_64']
        : name.endsWith('.app.tar.gz') ? ['darwin-x86_64', 'darwin-aarch64']
        : name.endsWith('.AppImage') ? ['linux-x86_64'] : [];
      if (!keys.length) continue;
      const signature = readFileSync(file, 'utf8').trim();
      if (!signature) throw new Error(`Empty signature: ${file}`);
      for (const key of keys) {
        if (platforms[key]) throw new Error(`Duplicate updater artifact for ${key}`);
        platforms[key] = { signature, url: `https://github.com/${repository}/releases/download/${tag}/${encodeURIComponent(name)}` };
      }
    }
  }
  visit(root);
  for (const key of ['windows-x86_64', 'darwin-x86_64', 'darwin-aarch64', 'linux-x86_64']) {
    if (!platforms[key]) throw new Error(`Missing updater platform: ${key}`);
  }
  return { version: tag.slice(1), notes: `See the GitHub release notes for ${tag}.`, pub_date: now.toISOString(), platforms };
}
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const [root, repository, tag] = process.argv.slice(2);
  writeFileSync(join(root, 'latest.json'), JSON.stringify(updaterManifest(root, repository, tag), null, 2) + '\n');
}
