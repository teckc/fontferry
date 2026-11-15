import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { updaterManifest } from './updater-manifest.mjs';
function fixture(t) {
  const dir = mkdtempSync(join(tmpdir(), 'fontferry-release-'));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  for (const name of ['FontFerry_setup.exe', 'FontFerry.app.tar.gz', 'FontFerry.AppImage']) {
    writeFileSync(join(dir, name), 'test artifact');
    writeFileSync(join(dir, name + '.sig'), 'test signature\n');
  }
  return dir;
}
test('universal macOS bundle serves both supported updater architectures', t => {
  const result = updaterManifest(fixture(t), 'teckc/fontferry', 'v0.2.0-alpha.6');
  assert.equal(Object.keys(result.platforms).length, 4);
  assert.deepEqual(result.platforms['darwin-x86_64'], result.platforms['darwin-aarch64']);
  assert.equal(result.version, '0.2.0-alpha.6');
  assert.equal(result.platforms['darwin-universal'], undefined);
});
test('missing artifact fails closed', t => {
  const dir = fixture(t); rmSync(join(dir, 'FontFerry.AppImage'));
  assert.throws(() => updaterManifest(dir, 'teckc/fontferry', 'v0.2.0'), /Missing signed artifact/);
});
test('empty signature fails closed', t => {
  const dir = fixture(t); writeFileSync(join(dir, 'FontFerry.AppImage.sig'), '');
  assert.throws(() => updaterManifest(dir, 'teckc/fontferry', 'v0.2.0'), /Empty signature/);
});
test('duplicate platform fails closed', t => {
  const dir = fixture(t); writeFileSync(join(dir, 'extra_setup.exe'), ''); writeFileSync(join(dir, 'extra_setup.exe.sig'), 'sig');
  assert.throws(() => updaterManifest(dir, 'teckc/fontferry', 'v0.2.0'), /Duplicate/);
});
