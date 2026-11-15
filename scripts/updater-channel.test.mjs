import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
const read = path => readFileSync(new URL('../' + path, import.meta.url), 'utf8');
const config = JSON.parse(read('apps/fontferry/src-tauri/tauri.conf.json'));
const source = read('apps/fontferry/src-tauri/src/commands.rs');
test('stable and opt-in beta update channels remain separate', () => {
  assert.match(source, /"stable"\s*=>\s*"https:\/\/github.com\/teckc\/fontferry\/releases\/latest\/download\/latest.json"/);
  assert.match(source, /"beta"\s*=>\s*"https:\/\/github.com\/teckc\/fontferry\/releases\/download\/beta\/latest.json"/);
  assert.deepEqual(config.plugins.updater.endpoints, ['https://github.com/teckc/fontferry/releases/latest/download/latest.json']);
});
test('candidate preserves the updater public key embedded in alpha.5', () => {
  // Public verifier, not a private key. Rotation requires a tested old-client bridge.
  assert.equal(config.plugins.updater.pubkey, 'dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEQ1M0FGMjlFQ0IyREY0MEQKUldRTjlDM0xudkk2MVpkZHU1WjlHTnhiTm5LSVFlU3ZNSytDS1cxcE9XSFE5NUg2dDg2M2IwNW0K');
});
