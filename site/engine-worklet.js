// AudioWorklet shell around asmr-core (wasm). Mirrors what the macOS shell does through the C ABI.
const P = { VOLUME: 0, WIND: 1, BIRDS: 2, WATER: 3, ORBIT: 4, PLAYING: 5, YAW: 6, PITCH: 7, ROLL: 8 };
const FRAMES = 128, MAX_SRC = 64, SRC_BYTES = 36, BANDS = 32;

class AsmrProcessor extends AudioWorkletProcessor {
  constructor(options) {
    super();
    const mod = new WebAssembly.Module(options.processorOptions.wasm);
    this.x = new WebAssembly.Instance(mod, {}).exports;
    this.eng = this.x.asmr_engine_new(sampleRate);
    this.pL = this.x.asmr_alloc(FRAMES * 4);
    this.pR = this.x.asmr_alloc(FRAMES * 4);
    this.pSrc = this.x.asmr_alloc(MAX_SRC * SRC_BYTES);
    this.pSpec = this.x.asmr_alloc(BANDS * 4);
    this.spatial = true;
    this.yaw = 0; this.yawTarget = 0;
    this.pitch = 0;
    this.tick = 0;
    this.reportEvery = Math.max(1, Math.round(sampleRate / FRAMES / 20));
    this.memory = null;
    this.x.asmr_set_param(this.eng, P.PLAYING, 1);
    this.port.onmessage = ({ data }) => {
      if (data.type === 'param') this.x.asmr_set_param(this.eng, P[data.name], data.value);
      else if (data.type === 'mode') this.spatial = data.spatial;
      else if (data.type === 'yaw') this.yawTarget = data.value;
    };
    this.port.postMessage({ type: 'ready' });
  }

  process(_in, outputs) {
    const out = outputs[0], l = out[0], r = out[1] || out[0];
    const x = this.x;
    // A flat recording cannot react to the head: lock the pose. Spatial mode follows the pointer smoothly.
    const target = this.spatial ? this.yawTarget : 0;
    this.yaw += (target - this.yaw) * 0.12;
    x.asmr_set_param(this.eng, P.YAW, this.yaw);
    x.asmr_render(this.eng, this.pL, this.pR, FRAMES);
    const mem = x.memory.buffer;
    if (this.memory !== mem) {
      this.memory = mem;
      this.left = new Float32Array(mem, this.pL, FRAMES);
      this.right = new Float32Array(mem, this.pR, FRAMES);
    }
    const L = this.left, R = this.right;
    if (this.spatial) {
      l.set(L); r.set(R);
    } else {
      // Typical stereo ASMR mix: fixed image, mostly mid with a little width, no head-related cues.
      for (let i = 0; i < FRAMES; i++) {
        const m = (L[i] + R[i]) * 0.5, s = (L[i] - R[i]) * 0.5 * 0.3;
        l[i] = m + s; r[i] = m - s;
      }
    }
    if (this.tick++ % this.reportEvery === 0) this.report();
    return true;
  }

  report() {
    const x = this.x;
    const n = x.asmr_get_sources(this.eng, this.pSrc, MAX_SRC);
    const dv = new DataView(x.memory.buffer, this.pSrc, MAX_SRC * SRC_BYTES);
    const sources = [];
    for (let i = 0; i < n; i++) {
      const o = i * SRC_BYTES;
      sources.push({
        kind: dv.getUint32(o, true),
        rx: dv.getFloat32(o + 16, true), ry: dv.getFloat32(o + 20, true),
        level: dv.getFloat32(o + 28, true), freq: dv.getFloat32(o + 32, true),
      });
    }
    x.asmr_get_spectrum(this.eng, this.pSpec, BANDS);
    this.port.postMessage({
      sources,
      spectrum: new Float32Array(x.memory.buffer, this.pSpec, BANDS).slice(),
      gust: x.asmr_get_telemetry(this.eng, 1),
      bubbles: x.asmr_get_telemetry(this.eng, 3),
    });
  }
}
registerProcessor('asmr', AsmrProcessor);
