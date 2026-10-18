const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

test('the shipped WASM renders finite stereo and reports at about 20 Hz', () => {
  const wasm = fs.readFileSync(path.join(__dirname, '../asmr_core.wasm'));
  const script = fs.readFileSync(path.join(__dirname, '../engine-worklet.js'), 'utf8');
  for (const sampleRate of [44100, 48000]) {
    let Processor;
    const messages = [];
    const context = vm.createContext({
      WebAssembly, sampleRate,
      AudioWorkletProcessor: class {
        constructor() { this.port = { postMessage: data => messages.push(data) }; }
      },
      registerProcessor: (_, type) => { Processor = type; },
    });
    vm.runInContext(script, context);
    const processor = new Processor({ processorOptions: { wasm } });
    assert.equal(messages[0].type, 'ready');
    const left = new Float32Array(128), right = new Float32Array(128);
    let energy = 0;
    for (let i = 0; i < Math.ceil(sampleRate / 128); i++) {
      assert.equal(processor.process([], [[left, right]]), true);
      assert.ok(left.every(Number.isFinite));
      assert.ok(right.every(Number.isFinite));
      energy += left.reduce((sum, v) => sum + v * v, 0);
    }
    assert.ok(energy > 0);
    const reports = messages.filter(m => m.spectrum);
    assert.ok(reports.length >= 19 && reports.length <= 22, `reports: ${reports.length}`);
    assert.equal(reports[0].spectrum.length, 32);
    assert.ok(reports[0].spectrum.every(Number.isFinite));
    const cached = processor.left;
    processor.process([], [[left, right]]);
    assert.equal(processor.left, cached);
    processor.x.asmr_engine_free(processor.eng);
  }
});
