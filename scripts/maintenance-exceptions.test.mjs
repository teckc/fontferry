import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { exceptions, validateScope, validateAdvisory } from './maintenance-exceptions.mjs';

function fixture() {
  const graph = {
    'zvariant@5.13.1': ['zvariant_derive@5.13.1'],
    'zvariant_derive@5.13.1': [],
    'zbus_names@4.3.4': ['zvariant@5.13.1'],
    'zbus_macros@5.18.0': ['zvariant@5.13.1', 'zbus_names@4.3.4'],
    'zbus@5.18.0': ['zvariant@5.13.1', 'zbus_names@4.3.4', 'zbus_macros@5.18.0'],
    'tauri-plugin-opener@2.5.5': ['zbus@5.18.0'],
    'tauri-plugin-single-instance@2.4.4': ['zbus@5.18.0'],
  };
  return {
    packages: Object.keys(graph).map(id => ({ id, name: id.split('@')[0], version: id.split('@')[1] })),
    resolve: { nodes: Object.entries(graph).map(([id, dependencies]) => ({ id, dependencies })) },
  };
}
const deny = readFileSync(new URL('../deny.toml', import.meta.url), 'utf8');
const audit = readFileSync(new URL('../.cargo/audit.toml', import.meta.url), 'utf8');
const date = new Date('2026-11-20T00:00:00Z');
const validate = (metadata = fixture(), config = deny, auditConfig = audit) => validateScope(metadata, config, auditConfig, date);
const advisory = (e, kind = 'unmaintained') => `\x60\x60\x60toml\n[advisory]\nid = "${e.id}"\npackage = "${e.name}"\ninformational = "${kind}"\n[versions]\npatched = []\n\x60\x60\x60`;

test('approved dependency scope and three notices pass before expiry', () => {
  validate();
  for (const e of exceptions) validateAdvisory(advisory(e), e);
});
test('exact expiry, after expiry and invalid clocks fail closed', () => {
  for (const time of ['2026-12-15T00:00:00Z', '2027-01-01', '2026-11-14', 'invalid']) {
    assert.throws(() => validateScope(fixture(), deny, audit, new Date(time)), /expired|clock/);
  }
});
test('new consumers and changed versions require a fresh review', () => {
  const m = fixture();
  m.packages.push({ id: 'new@1', name: 'new', version: '1' });
  m.resolve.nodes.push({ id: 'new@1', dependencies: ['zvariant@5.13.1'] });
  assert.throws(() => validate(m), /consumers/);
  const n = fixture();
  n.packages.find(p => p.name === 'zvariant').version = '5.14.0';
  assert.throws(() => validate(n), /scope changed/);
});
test('resolved dependencies force exception removal, not silent renewal', () => {
  const m = fixture();
  m.packages = m.packages.filter(p => p.name !== 'zvariant');
  assert.throws(() => validate(m), /scope changed/);
});
test('audit ignore, broad scope and undated reasons fail', () => {
  assert.throws(() => validate(fixture(), deny, exceptions[0].id), /cargo-audit/);
  assert.throws(() => validate(fixture(), `${deny}\nunmaintained = "none"`), /configuration changed/);
  assert.throws(() => validate(fixture(), deny.replaceAll('2026-12-15', 'later')), /configuration changed/);
});
test('alternative TOML quoting and extra advisory exemptions cannot bypass review', () => {
  for (const line of ["unmaintained = 'none'", '"unmaintained" = "none"',
    "'unmaintained' = 'workspace'", 'unmaintained = "none" # broaden',
    'unmaintained = "all"\nignore-expiry = "P999D"']) {
    assert.throws(() => validate(fixture(), deny.replace('unmaintained = "all"', line)), /configuration changed/);
  }
  assert.throws(() => validate(fixture(), deny.replace('ignore = [', 'ignore = ["RUSTSEC-2026-9999",')), /configuration changed/);
  validate(fixture(), deny.replace(/\r?\n/g, '\r\n'), audit.replace(/\r?\n/g, '\r\n'));
});
test('vulnerability, unsound, withdrawn, mismatched and malformed advisories fail', () => {
  const e = exceptions[0];
  for (const s of [advisory(e, 'unsound'), advisory(e, 'vulnerability'),
    advisory(e).replace('informational = "unmaintained"', 'cvss = "CVSS:3.1"'),
    advisory(e).replace('[versions]', 'withdrawn = "2026-11-01"\n[versions]'),
    advisory(e).replace(e.id, 'RUSTSEC-2026-9999'), 'unavailable']) {
    assert.throws(() => validateAdvisory(s, e));
  }
});
