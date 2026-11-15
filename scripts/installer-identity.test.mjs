import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
const config = JSON.parse(readFileSync(new URL('../apps/fontferry/src-tauri/tauri.conf.json', import.meta.url), 'utf8'));
// alpha.5 identity, interpreted by tauri-cli-v2.11.4 nsis/mod.rs and installer.nsi.
// A deliberate migration must update these checks AND supply migration tests.
test('Windows installer retains alpha.5 identity and previous-location registry key', () => {
  assert.equal(config.productName, 'FontFerry');
  assert.equal(config.identifier, 'io.github.teckc.fontferry');
  assert.equal(config.bundle.windows.nsis.installMode, 'currentUser');
  const manufacturer = config.bundle.publisher ?? config.identifier.split('.')[1];
  assert.equal(manufacturer, 'github');
  assert.equal(`Software\\${manufacturer}\\${config.productName}`, 'Software\\github\\FontFerry');
  assert.equal(config.bundle.windows.nsis.startMenuFolder, undefined);
  assert.equal(config.bundle.windows.nsis.template, undefined);
});
test('Windows installer retains standard opt-in data deletion without custom cleanup hooks', () => {
  assert.equal(config.bundle.windows.nsis.installerHooks, undefined);
  assert.equal(config.bundle.windows.nsis.installMode, 'currentUser');
});
