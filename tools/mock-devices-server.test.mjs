import { test } from 'node:test';
import assert from 'node:assert/strict';
import { validateDevices, fixtureText } from './mock-devices-server.mjs';

const device = { family: 'mouse', model: '测试鼠标', level: 78, charging: false, connected: true };
test('valid rows preserve offline and charging state', () => {
  const rows = validateDevices([device, { ...device, family: 'keyboard', level: 100, charging: true, connected: false }]);
  assert.equal(fixtureText(rows), 'mouse|测试鼠标|78|0|online\nkeyboard|测试鼠标|100|1|offline\n');
  assert.equal(fixtureText(validateDevices([])), '\n');
});
test('malformed edits cannot introduce extra fixture rows', () => {
  for (const update of [{ model: 'x|9|1' }, { model: 'x\nmouse|other|10|0' }, { model: '' },
    { level: 101 }, { level: -1 }, { level: 7.8 }, { family: 'bad' }, { charging: 1 }]) {
    assert.throws(() => validateDevices([{ ...device, ...update }]));
  }
  assert.throws(() => validateDevices(Array(33).fill(device)));
});
