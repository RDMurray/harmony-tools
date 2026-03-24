import createModule from "./harmony32_wasm.js";

class Harmony32Processor extends AudioWorkletProcessor {
  constructor(options) {
    super();

    this.wasmUrl = options.processorOptions.wasmUrl;
    this.wasmBinary = options.processorOptions.wasmBinary || null;
    this.romBuffer = options.processorOptions.romBuffer;
    this.sampleRateOverride = options.processorOptions.sampleRate || sampleRate;
    this.cpuHz = options.processorOptions.cpuHz || 2000000;

    this.ctrlSab = options.processorOptions.controlSab;
    this.ctrl = this.ctrlSab ? new Int32Array(this.ctrlSab) : null;

    this.ready = false;
    this.failed = false;
    this.engine = 0;
    this.mod = null;
    this.outPtr = 0;
    this.stemPtr = 0;
    this.outFrames = 128;
    this.statusPtr = 0;
    this.statusStride = 28;
    this.statusTicker = 0;
    this.satLastIn = [0.0, 0.0, 0.0];
    this.satLastOut = [0.0, 0.0, 0.0];

    this.port.onmessage = (evt) => {
      if (!evt || !evt.data) {
        return;
      }
      if (evt.data.type === "cpu-reset") {
        if (this.mod && this.engine) {
          this.mod._h32_reset_cpu(this.engine);
        }
      }
      if (evt.data.type === "full-reset") {
        this.applyControls(true);
      }
      if (evt.data.type === "set-rom" && evt.data.romBuffer) {
        this.romBuffer = evt.data.romBuffer;
        this.reloadRom();
      }
    };

    this.initialize();
  }

  async initialize() {
    try {
      this.mod = await createModule({
        wasmBinary: this.wasmBinary ? new Uint8Array(this.wasmBinary) : undefined,
        locateFile: (path) => {
          if (path.endsWith(".wasm")) {
            return this.wasmUrl;
          }
          return path;
        }
      });

      this.engine = this.mod._h32_create(this.sampleRateOverride | 0, this.cpuHz | 0);
      if (!this.engine) {
        throw new Error("h32_create failed");
      }

      if (!this.loadRomBytes(this.romBuffer)) {
        throw new Error("ROM load failed");
      }

      this.outPtr = this.mod._malloc(this.outFrames * 8);
      this.stemPtr = this.mod._malloc(this.outFrames * 12);
      this.statusPtr = this.mod._malloc(this.statusStride);
      this.applyControls(false);
      this.ready = true;
      this.port.postMessage({ type: "ready", bankCount: this.mod._h32_get_bank_count(this.engine) >>> 0 });
    } catch (err) {
      this.failed = true;
      this.port.postMessage({ type: "error", message: String(err) });
    }
  }

  loadRomBytes(romBuffer) {
    if (!this.mod || !this.engine || !romBuffer) {
      return false;
    }
    const romBytes = new Uint8Array(romBuffer);
    const ptr = this.mod._malloc(romBytes.length);
    this.mod.HEAPU8.set(romBytes, ptr);
    const rc = this.mod._h32_load_rom(this.engine, ptr, romBytes.length);
    this.mod._free(ptr);
    return rc === 0;
  }

  reloadRom() {
    if (!this.ready || !this.mod) {
      return;
    }
    const ok = this.loadRomBytes(this.romBuffer);
    const bankCount = this.mod._h32_get_bank_count(this.engine) >>> 0;
    this.port.postMessage({ type: "rom-status", ok, bankCount });
  }

  applyControls(forceReset) {
    if (!this.mod || !this.engine) {
      return;
    }

    let song = 0;
    let bank = 0;
    let speed = 3;
    let drumsOn = 1;
    let running = 1;
    let chALevel = 100;
    let chBLevel = 100;
    let chCLevel = 100;
    let chAPan = 0;
    let chBPan = 0;
    let chCPan = 0;
    let mixMode = 1;

    if (this.ctrl) {
      song = Atomics.load(this.ctrl, 1);
      bank = Atomics.load(this.ctrl, 2);
      speed = Atomics.load(this.ctrl, 3);
      drumsOn = Atomics.load(this.ctrl, 4);
      running = Atomics.load(this.ctrl, 5);
      chALevel = Atomics.load(this.ctrl, 6);
      chBLevel = Atomics.load(this.ctrl, 7);
      chCLevel = Atomics.load(this.ctrl, 8);
      chAPan = Atomics.load(this.ctrl, 9);
      chBPan = Atomics.load(this.ctrl, 10);
      chCPan = Atomics.load(this.ctrl, 11);
      mixMode = Atomics.load(this.ctrl, 15);
      if (!forceReset) {
        Atomics.store(this.ctrl, 0, 0);
      }
    }

    if (mixMode !== 0 && mixMode !== 1) {
      mixMode = 1;
    }

    if (forceReset) {
      this.mod._h32_reset_full(this.engine, song, bank >>> 0);
    }
    this.mod._h32_set_controls(this.engine, song, bank >>> 0, speed, drumsOn, running);
    this.mod._h32_set_mix_mode(this.engine, mixMode);

    /* Preserve legacy renderer behavior when mixMode=0. */
    this.mod._h32_set_channel_mix(this.engine, 0, Math.max(0, Math.min(100, chALevel)), chAPan);
    this.mod._h32_set_channel_mix(this.engine, 1, Math.max(0, Math.min(100, chBLevel)), chBPan);
    this.mod._h32_set_channel_mix(this.engine, 2, Math.max(0, Math.min(100, chCLevel)), chCPan);
  }

  saturate(channel, x, drivePct) {
    if (drivePct <= 0) {
      return x;
    }

    /* Analog-ish waveshaper:
     * - drive into asymmetrical tanh stages
     * - gentle even-harmonic bias
     * - one-pole DC blocker after shaping
     */
    const drive = Math.max(0.0, Math.min(1.0, drivePct / 100.0));
    const inGain = 1.0 + 7.0 * drive;
    const asym = 0.06 + 0.22 * drive;
    const pos = Math.tanh((x + asym) * inGain);
    const neg = Math.tanh((x - asym) * (inGain * 0.92));
    let y = (pos + neg) * 0.5;

    /* Blend shaped and clean to keep low-drive detail. */
    const wet = 0.35 + 0.65 * drive;
    y = x * (1.0 - wet) + y * wet;

    /* Soft output trim with slight high-drive compression feel. */
    y *= 1.0 / (1.0 + drive * 0.8);

    /* DC blocker: y[n] = x[n] - x[n-1] + r*y[n-1] */
    {
      const r = 0.995;
      const out = y - this.satLastIn[channel] + r * this.satLastOut[channel];
      this.satLastIn[channel] = y;
      this.satLastOut[channel] = out;
      y = out;
    }

    return y;
  }

  emitStatus() {
    if (!this.mod || !this.engine || !this.statusPtr) {
      return;
    }

    this.mod._h32_get_status(this.engine, this.statusPtr);
    const h = this.mod.HEAPU8;
    const u32 = this.mod.HEAPU32;
    const baseU8 = this.statusPtr;
    const baseU32 = this.statusPtr >>> 2;

    const status = {
      songEnded: h[baseU8],
      initialized: h[baseU8 + 1],
      running: h[baseU8 + 2],
      song: h[baseU8 + 4],
      speed: h[baseU8 + 5],
      drumsOn: h[baseU8 + 6],
      mixMode: h[baseU8 + 7],
      bank: u32[baseU32 + 2],
      bankCount: u32[baseU32 + 3],
      sampleRate: u32[baseU32 + 4],
      cpuHz: u32[baseU32 + 5],
      steps: u32[baseU32 + 6]
    };

    this.port.postMessage({ type: "status", status });
  }

  process(_inputs, outputs) {
    const output = outputs[0];
    if (!output || output.length === 0) {
      return true;
    }

    const leftOut = output[0];
    const frames = leftOut.length;

    if (!this.ready || this.failed) {
      for (let c = 0; c < output.length; c++) {
        output[c].fill(0);
      }
      return true;
    }

    if (this.ctrl && Atomics.load(this.ctrl, 0) !== 0) {
      this.applyControls(false);
    }

    if (frames > this.outFrames) {
      if (this.outPtr) {
        this.mod._free(this.outPtr);
      }
      if (this.stemPtr) {
        this.mod._free(this.stemPtr);
      }
      this.outFrames = frames;
      this.outPtr = this.mod._malloc(this.outFrames * 8);
      this.stemPtr = this.mod._malloc(this.outFrames * 12);
    }

    leftOut.fill(0);
    if (output.length > 1) {
      output[1].fill(0);
    }

    const mixMode = this.ctrl ? Atomics.load(this.ctrl, 15) : 1;
    if (mixMode === 0) {
      this.mod._h32_render(this.engine, this.outPtr, frames);
      const rendered = this.mod.HEAPF32.subarray(this.outPtr >>> 2, (this.outPtr >>> 2) + (frames * 2));
      for (let i = 0, j = 0; i < frames; i++, j += 2) {
        leftOut[i] = rendered[j];
        if (output.length > 1) {
          output[1][i] = rendered[j + 1];
        }
      }
    } else {
      const aLevel = this.ctrl ? Atomics.load(this.ctrl, 6) : 100;
      const bLevel = this.ctrl ? Atomics.load(this.ctrl, 7) : 100;
      const cLevel = this.ctrl ? Atomics.load(this.ctrl, 8) : 100;
      const aPan = this.ctrl ? Atomics.load(this.ctrl, 9) : 0;
      const bPan = this.ctrl ? Atomics.load(this.ctrl, 10) : 0;
      const cPan = this.ctrl ? Atomics.load(this.ctrl, 11) : 0;
      const aDrive = this.ctrl ? Atomics.load(this.ctrl, 12) : 0;
      const bDrive = this.ctrl ? Atomics.load(this.ctrl, 13) : 0;
      const cDrive = this.ctrl ? Atomics.load(this.ctrl, 14) : 0;

      const ga = Math.max(0, aLevel) / 100.0;
      const gb = Math.max(0, bLevel) / 100.0;
      const gc = Math.max(0, cLevel) / 100.0;
      const pa = Math.max(-100, Math.min(100, aPan)) / 100.0;
      const pb = Math.max(-100, Math.min(100, bPan)) / 100.0;
      const pc = Math.max(-100, Math.min(100, cPan)) / 100.0;
      const dla = (1.0 - pa) * 0.5;
      const dra = (1.0 + pa) * 0.5;
      const dlb = (1.0 - pb) * 0.5;
      const drb = (1.0 + pb) * 0.5;
      const dlc = (1.0 - pc) * 0.5;
      const drc = (1.0 + pc) * 0.5;

      this.mod._h32_render_stems(this.engine, this.stemPtr, frames);
      const stems = this.mod.HEAPF32.subarray(this.stemPtr >>> 2, (this.stemPtr >>> 2) + (frames * 3));

      for (let i = 0, j = 0; i < frames; i++, j += 3) {
        const sa = this.saturate(0, stems[j] * ga, aDrive);
        const sb = this.saturate(1, stems[j + 1] * gb, bDrive);
        const sc = this.saturate(2, stems[j + 2] * gc, cDrive);
        let l = sa * dla + sb * dlb + sc * dlc;
        let r = sa * dra + sb * drb + sc * drc;

        l = l / (1.0 + 0.65 * Math.abs(l));
        r = r / (1.0 + 0.65 * Math.abs(r));

        leftOut[i] = l;
        if (output.length > 1) {
          output[1][i] = r;
        }
      }
    }

    this.statusTicker++;
    if ((this.statusTicker & 15) === 0) {
      this.emitStatus();
    }

    return true;
  }
}

registerProcessor("harmony32-processor", Harmony32Processor);
