import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
const root=new URL('../',import.meta.url);
const config=JSON.parse(readFileSync(new URL('apps/fontferry/src-tauri/tauri.conf.json',root),'utf8'));
test('Linux packages explicitly require the fontconfig command package',()=>{
  assert.ok(config.bundle.linux.deb.depends.includes('fontconfig'));
  assert.ok(config.bundle.linux.rpm.depends.includes('fontconfig'));
});
test('locked Tauri JavaScript API and Rust library have matching major/minor',()=>{
  const cargo=readFileSync(new URL('Cargo.lock',root),'utf8');
  const rust=cargo.match(/\[\[package\]\]\nname = "tauri"\nversion = "(\d+\.\d+)\./)?.[1];
  const lock=readFileSync(new URL('pnpm-lock.yaml',root),'utf8');
  const api=lock.match(/'@tauri-apps\/api':\n\s+specifier: [^\n]+\n\s+version: (\d+\.\d+)\./)?.[1];
  assert.ok(rust && api,'both locked versions must be found');
  assert.equal(api,rust,'Tauri build checks require API and Rust major/minor alignment');
});
