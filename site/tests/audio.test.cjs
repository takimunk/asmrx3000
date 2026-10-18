const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

function page({ failDownload = false, failProcessor = false } = {}) {
  const elements = new Map();
  const drawing = new Proxy({}, { get: () => () => {} });
  const element = id => {
    if (!elements.has(id)) elements.set(id, {
      value: '0.5', width: 320, height: 320, disabled: false,
      setAttribute() {}, addEventListener() {}, getContext: () => drawing,
    });
    return elements.get(id);
  };
  const contexts = [], sources = [];
  let fetches = 0;
  class AudioContext {
    constructor() {
      contexts.push(this);
      this.destination = {};
      this.audioWorklet = { addModule: async () => {} };
    }
    async close() { this.closed = true; }
    async resume() { this.running = true; }
    async suspend() { this.running = false; }
  }
  class AudioWorkletNode {
    constructor() {
      sources.push(this);
      this.port = { postMessage() {} };
      queueMicrotask(() => {
        if (failProcessor && sources.length === 1) this.onprocessorerror();
        else this.port.onmessage({ data: { type: 'ready' } });
      });
    }
    connect() {}
    disconnect() {}
  }
  const context = vm.createContext({
    document: { getElementById: element }, AudioContext, AudioWorkletNode,
    fetch: async () => {
      fetches++;
      return { ok: !(failDownload && fetches === 1), status: 503, arrayBuffer: async () => new ArrayBuffer(0) };
    },
    addEventListener() {}, requestAnimationFrame() {}, setTimeout, clearTimeout,
  });
  const html = fs.readFileSync(path.join(__dirname, '../index.html'), 'utf8');
  vm.runInContext(html.match(/<script>([\s\S]*?)<\/script>/)[1], context);
  return { element, contexts, sources };
}

test('a failed download closes its context and the next Play retries successfully', async () => {
  const p = page({ failDownload: true });
  await p.element('play').onclick();
  assert.equal(p.contexts[0].closed, true);
  assert.equal(p.element('play').disabled, false);
  await p.element('play').onclick();
  assert.equal(p.contexts.length, 2);
  assert.equal(p.contexts[1].running, true);
  assert.match(p.element('play').textContent, /PAUSE/);
});

test('Play cannot start another context while initialization is in progress', async () => {
  const p = page();
  await Promise.all([p.element('play').onclick(), p.element('play').onclick()]);
  assert.equal(p.contexts.length, 1);
  assert.equal(p.contexts[0].running, true);
});

test('a WASM processor initialization error is retryable', async () => {
  const p = page({ failProcessor: true });
  await p.element('play').onclick();
  assert.equal(p.contexts[0].closed, true);
  await p.element('play').onclick();
  assert.equal(p.contexts[1].running, true);
});

test('a processor failure during playback resets Play and permits a fresh engine', async () => {
  const p = page();
  await p.element('play').onclick();
  p.sources[0].onprocessorerror();
  assert.match(p.element('play').textContent, /PLAY/);
  await p.element('play').onclick();
  assert.equal(p.contexts.length, 2);
  assert.equal(p.contexts[1].running, true);
});
