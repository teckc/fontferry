import { readFile, appendFile } from 'node:fs/promises';
import { pathToFileURL } from 'node:url';

export const deadline = '2026-12-15T00:00:00Z';
export const started = '2026-11-15T00:00:00Z';
export const exceptions = [
  { id: 'RUSTSEC-2026-0370', name: 'zvariant', version: '5.13.1' },
  { id: 'RUSTSEC-2026-0371', name: 'zbus_names', version: '4.3.4' },
  { id: 'RUSTSEC-2026-0372', name: 'zvariant_derive', version: '5.13.1' },
];
const parents = {
  'zvariant@5.13.1': ['zbus@5.18.0', 'zbus_macros@5.18.0', 'zbus_names@4.3.4'],
  'zbus_names@4.3.4': ['zbus@5.18.0', 'zbus_macros@5.18.0'],
  'zvariant_derive@5.13.1': ['zvariant@5.13.1'],
  'zbus@5.18.0': ['tauri-plugin-opener@2.5.5', 'tauri-plugin-single-instance@2.4.4'],
  'zbus_macros@5.18.0': ['zbus@5.18.0'],
};
const fail = (message) => { throw new Error(message); };
const key = (p) => `${p.name}@${p.version}`;

export function validateScope(metadata, deny, audit, now = new Date()) {
  if (!Number.isFinite(now.getTime()) || now < new Date(started) || now >= new Date(deadline)) {
    fail(`Maintenance exceptions expired or clock invalid; deadline ${deadline}. No automatic renewal.`);
  }
  if (/unmaintained\s*=\s*"(?:none|workspace|transitive)"/.test(deny)) fail('Unmaintained scope was weakened');
  const packages = new Map(metadata.packages.map(p => [p.id, p]));
  if (!metadata.resolve?.nodes) fail('Cargo dependency resolution missing');
  for (const e of exceptions) {
    const ignored = deny.split('\n').filter(line => line.includes(e.id));
    if (ignored.length !== 1 || !ignored[0].includes('reason =') || !ignored[0].includes('2026-12-15')) {
      fail(`${e.id}: requires one documented, dated cargo-deny entry`);
    }
    if (audit.includes(e.id)) fail(`${e.id}: must remain visible to independent cargo-audit`);
    const matches = metadata.packages.filter(p => p.name === e.name);
    if (matches.length !== 1 || matches[0].version !== e.version) fail(`${e.name}: exception scope changed; review/remove it`);
  }
  for (const [dependency, allowed] of Object.entries(parents)) {
    const targets = metadata.packages.filter(p => key(p) === dependency);
    if (targets.length !== 1) fail(`${dependency}: missing or ambiguous dependency; remove/review exception`);
    const actual = metadata.resolve.nodes.filter(n => n.dependencies.includes(targets[0].id))
      .map(n => key(packages.get(n.id))).sort();
    if (JSON.stringify(actual) !== JSON.stringify([...allowed].sort())) fail(`${dependency}: dependency consumers changed`);
  }
}

export function validateAdvisory(text, expected) {
  const toml = text.match(/^```toml\r?\n([\s\S]*?)\r?\n```/);
  if (!toml) fail(`${expected.id}: malformed advisory`);
  const section = toml[1].match(/(?:^|\n)\[advisory\]\r?\n([\s\S]*?)(?=\n\[|$)/)?.[1];
  if (!section) fail(`${expected.id}: missing advisory section`);
  const field = name => {
    const lines = section.split('\n').filter(l => new RegExp(`^${name}\\s*=`).test(l));
    if (lines.length !== 1) fail(`${expected.id}: missing/duplicate ${name}`);
    return lines[0].match(/=\s*"([^"]*)"\s*$/)?.[1];
  };
  if (field('id') !== expected.id || field('package') !== expected.name || field('informational') !== 'unmaintained') {
    fail(`${expected.id}: not the approved unmaintained notice`);
  }
  if (/^\s*(cvss|withdrawn)\s*=/m.test(section)) fail(`${expected.id}: advisory changed; manual review required`);
}

async function main() {
  const [metadata, deny, audit] = await Promise.all([
    readFile(process.argv[2], 'utf8').then(JSON.parse),
    readFile('deny.toml', 'utf8'), readFile('.cargo/audit.toml', 'utf8'),
  ]);
  validateScope(metadata, deny, audit);
  for (const e of exceptions) {
    const url = `https://raw.githubusercontent.com/RustSec/advisory-db/main/crates/${e.name}/${e.id}.md`;
    const response = await fetch(url, { signal: AbortSignal.timeout(30000), redirect: 'error' });
    if (!response.ok) fail(`${e.id}: advisory retrieval failed (${response.status})`);
    validateAdvisory(await response.text(), e);
    const message = `${e.id} ${e.name}@${e.version}: temporary UNMAINTAINED exception only; expires ${deadline}; migrate Tauri opener/single-instance to zbus 6. Vulnerability audit remains mandatory.`;
    console.log(`::warning title=Temporary maintenance exception::${message}`);
    if (process.env.GITHUB_STEP_SUMMARY) await appendFile(process.env.GITHUB_STEP_SUMMARY, `- ${message}\n`);
  }
}
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch(error => { console.error(`::error::${error.message}`); process.exitCode = 1; });
}
