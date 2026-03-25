const els = {
  playPauseBtn: document.getElementById("playPauseBtn"),
  cpuResetBtn: document.getElementById("cpuResetBtn"),
  fullResetBtn: document.getElementById("fullResetBtn"),
  songPrevBtn: document.getElementById("songPrevBtn"),
  songNextBtn: document.getElementById("songNextBtn"),
  songValue: document.getElementById("songValue"),
  bankPrevBtn: document.getElementById("bankPrevBtn"),
  bankNextBtn: document.getElementById("bankNextBtn"),
  bankValue: document.getElementById("bankValue"),
  speed1Btn: document.getElementById("speed1Btn"),
  speed2Btn: document.getElementById("speed2Btn"),
  speed3Btn: document.getElementById("speed3Btn"),
  speed4Btn: document.getElementById("speed4Btn"),
  drumsSwitch: document.getElementById("drumsSwitch"),
  cpuFreqInput: document.getElementById("cpuFreqInput"),
  ymFreqInput: document.getElementById("ymFreqInput"),
  mixLegacyBtn: document.getElementById("mixLegacyBtn"),
  mixStemBtn: document.getElementById("mixStemBtn"),
  ymResampledBtn: document.getElementById("ymResampledBtn"),
  ymDirectBtn: document.getElementById("ymDirectBtn"),

  chALevel: document.getElementById("chALevel"),
  chBLevel: document.getElementById("chBLevel"),
  chCLevel: document.getElementById("chCLevel"),
  chAPan: document.getElementById("chAPan"),
  chBPan: document.getElementById("chBPan"),
  chCPan: document.getElementById("chCPan"),
  chADrive: document.getElementById("chADrive"),
  chBDrive: document.getElementById("chBDrive"),
  chCDrive: document.getElementById("chCDrive"),
  chALevelValue: document.getElementById("chALevelValue"),
  chBLevelValue: document.getElementById("chBLevelValue"),
  chCLevelValue: document.getElementById("chCLevelValue"),
  chAPanValue: document.getElementById("chAPanValue"),
  chBPanValue: document.getElementById("chBPanValue"),
  chCPanValue: document.getElementById("chCPanValue"),
  chADriveValue: document.getElementById("chADriveValue"),
  chBDriveValue: document.getElementById("chBDriveValue"),
  chCDriveValue: document.getElementById("chCDriveValue"),

  irEnable: document.getElementById("irEnable"),
  irMix: document.getElementById("irMix"),
  irMixValue: document.getElementById("irMixValue"),
  irSelect: document.getElementById("irSelect"),
  irUpload: document.getElementById("irUpload"),
  compAmount: document.getElementById("compAmount"),
  compAmountValue: document.getElementById("compAmountValue"),

  romSelect: document.getElementById("romSelect"),
  romUpload: document.getElementById("romUpload"),
  romInfo: document.getElementById("romInfo"),
  audioState: document.getElementById("audioState")
};

const CONTROL_IDX = {
  dirty: 0,
  song: 1,
  bank: 2,
  speed: 3,
  drumsOn: 4,
  running: 5,
  chALevel: 6,
  chBLevel: 7,
  chCLevel: 8,
  chAPan: 9,
  chBPan: 10,
  chCPan: 11,
  chADrive: 12,
  chBDrive: 13,
  chCDrive: 14,
  mixMode: 15,
  cpuHz: 16,
  ymHz: 17,
  ymRenderMode: 18
};

const DEFAULT_CPU_HZ = 2000000;
const DEFAULT_CPU_MHZ = DEFAULT_CPU_HZ / 1000000;
const DEFAULT_YM_HZ = 2000000;
const DEFAULT_YM_MHZ = DEFAULT_YM_HZ / 1000000;
const MAX_CPU_HZ = 0xffffffff;

let ctx = null;
let node = null;
let dryGainNode = null;
let wetGainNode = null;
let convolverNode = null;
let compressorNode = null;
let controlSab = null;
let control = null;
let controlU32 = null;
let romBuffer = null;
let wasmBinaryBuffer = null;
let bundledRoms = [];
let currentBundledRomIndex = 0;
let bundledIrs = [];
let currentBundledIrIndex = 0;
let currentSongUi = 1;
let currentBankUi = 1;
let bankCountUi = 1;
let currentSpeedUi = 2;
let currentMixMode = 1;
let currentCpuHzUi = DEFAULT_CPU_HZ;
let currentYmHzUi = DEFAULT_YM_HZ;
let currentYmRenderMode = 1;

function setPlayEnabled(enabled) {
  if (els.playPauseBtn) {
    els.playPauseBtn.disabled = !enabled;
  }
}

function setPlayPauseVisualState(isRunning) {
  if (!els.playPauseBtn) {
    return;
  }
  if (isRunning) {
    els.playPauseBtn.textContent = "Pause";
    els.playPauseBtn.setAttribute("aria-label", "Pause");
    return;
  }
  els.playPauseBtn.textContent = "Play";
  els.playPauseBtn.setAttribute("aria-label", "Play");
}

function setAudioStateMessage(message) {
  if (els.audioState) {
    els.audioState.textContent = message;
  }
}

function clampInt(value, min, max, fallback) {
  const parsed = Number.parseInt(String(value), 10);
  if (Number.isNaN(parsed)) {
    return fallback;
  }
  return Math.max(min, Math.min(max, parsed));
}

function parseCpuMHz(value) {
  const parsed = Number.parseFloat(String(value));
  if (!Number.isFinite(parsed) || parsed <= 0) {
    return null;
  }
  return parsed;
}

function formatCpuMHz(value) {
  const normalized = Number.isFinite(value) && value > 0 ? value : DEFAULT_CPU_MHZ;
  return normalized.toFixed(6).replace(/\.?0+$/, "");
}

function cpuMHzToHz(mhz) {
  const hz = Math.round(mhz * 1000000);
  if (!Number.isFinite(hz) || hz <= 0) {
    return DEFAULT_CPU_HZ;
  }
  return Math.max(1, Math.min(MAX_CPU_HZ, hz));
}

function readCpuHzFromUi() {
  const parsedMHz = parseCpuMHz(els.cpuFreqInput && els.cpuFreqInput.value);
  return parsedMHz === null ? currentCpuHzUi : cpuMHzToHz(parsedMHz);
}

function syncCpuInputFromHz(cpuHz) {
  if (!els.cpuFreqInput) {
    return;
  }
  const normalizedHz = Number.isFinite(cpuHz) && cpuHz > 0 ? Math.min(MAX_CPU_HZ, Math.round(cpuHz)) : DEFAULT_CPU_HZ;
  currentCpuHzUi = normalizedHz;
  els.cpuFreqInput.value = formatCpuMHz(normalizedHz / 1000000);
}

function ymMHzToHz(mhz) {
  const hz = Math.round(mhz * 1000000);
  if (!Number.isFinite(hz) || hz <= 0) {
    return DEFAULT_YM_HZ;
  }
  return Math.max(1, Math.min(MAX_CPU_HZ, hz));
}

function readYmHzFromUi() {
  const parsedMHz = parseCpuMHz(els.ymFreqInput && els.ymFreqInput.value);
  return parsedMHz === null ? currentYmHzUi : ymMHzToHz(parsedMHz);
}

function syncYmInputFromHz(ymHz) {
  if (!els.ymFreqInput) {
    return;
  }
  const normalizedHz = Number.isFinite(ymHz) && ymHz > 0 ? Math.min(MAX_CPU_HZ, Math.round(ymHz)) : DEFAULT_YM_HZ;
  currentYmHzUi = normalizedHz;
  els.ymFreqInput.value = formatCpuMHz(normalizedHz / 1000000);
}

function setBankCount(nextCount) {
  bankCountUi = clampInt(nextCount, 1, 0x7fffffff, 1);
  if (currentBankUi < 1 || currentBankUi > bankCountUi) {
    currentBankUi = 1;
  }
}

function inferRunning() {
  if (control) {
    return Atomics.load(control, CONTROL_IDX.running) ? 1 : 0;
  }
  if (ctx && ctx.state === "suspended") {
    return 0;
  }
  return 1;
}

function readControls(runningOverride) {
  const chALevel = clampInt(els.chALevel && els.chALevel.value, 0, 400, 100);
  const chBLevel = clampInt(els.chBLevel && els.chBLevel.value, 0, 400, 100);
  const chCLevel = clampInt(els.chCLevel && els.chCLevel.value, 0, 400, 100);
  const chAPan = clampInt(els.chAPan && els.chAPan.value, -100, 100, 0);
  const chBPan = clampInt(els.chBPan && els.chBPan.value, -100, 100, 0);
  const chCPan = clampInt(els.chCPan && els.chCPan.value, -100, 100, 0);
  const chADrive = clampInt(els.chADrive && els.chADrive.value, 0, 100, 0);
  const chBDrive = clampInt(els.chBDrive && els.chBDrive.value, 0, 100, 0);
  const chCDrive = clampInt(els.chCDrive && els.chCDrive.value, 0, 100, 0);

  return {
    song: currentSongUi - 1,
    bank: currentBankUi - 1,
    speed: 4 - currentSpeedUi,
    drumsOn: els.drumsSwitch && els.drumsSwitch.checked ? 1 : 0,
    running: typeof runningOverride === "number" ? runningOverride : inferRunning(),
    chALevel,
    chBLevel,
    chCLevel,
    chAPan,
    chBPan,
    chCPan,
    chADrive,
    chBDrive,
    chCDrive,
    mixMode: currentMixMode,
    cpuHz: readCpuHzFromUi(),
    ymHz: readYmHzFromUi(),
    ymRenderMode: currentYmRenderMode
  };
}

function updateFxGraphParameters() {
  if (!ctx || !dryGainNode || !wetGainNode || !compressorNode) {
    return;
  }

  const irOn = !!(els.irEnable && els.irEnable.checked && convolverNode && convolverNode.buffer);
  const irMixPct = clampInt(els.irMix && els.irMix.value, 0, 100, 0);
  const wet = irOn ? irMixPct / 100.0 : 0.0;
  const dry = 1.0 - wet;
  dryGainNode.gain.setValueAtTime(dry, ctx.currentTime);
  wetGainNode.gain.setValueAtTime(wet, ctx.currentTime);

  const compPct = clampInt(els.compAmount && els.compAmount.value, 0, 100, 0) / 100.0;
  const threshold = -6.0 - (34.0 * compPct);
  const ratio = 1.0 + (19.0 * compPct);
  compressorNode.threshold.setValueAtTime(threshold, ctx.currentTime);
  compressorNode.ratio.setValueAtTime(ratio, ctx.currentTime);
  compressorNode.knee.setValueAtTime(20.0 + 20.0 * compPct, ctx.currentTime);
  compressorNode.attack.setValueAtTime(0.003 + 0.02 * (1.0 - compPct), ctx.currentTime);
  compressorNode.release.setValueAtTime(0.08 + 0.25 * compPct, ctx.currentTime);
}

function renderControlState() {
  if (els.songValue) {
    els.songValue.textContent = String(currentSongUi);
  }
  if (els.bankValue) {
    els.bankValue.textContent = `${currentBankUi}/${bankCountUi}`;
  }
  if (els.bankPrevBtn) {
    els.bankPrevBtn.disabled = bankCountUi <= 1;
  }
  if (els.bankNextBtn) {
    els.bankNextBtn.disabled = bankCountUi <= 1;
  }

  els.speed1Btn.classList.toggle("active", currentSpeedUi === 1);
  els.speed2Btn.classList.toggle("active", currentSpeedUi === 2);
  els.speed3Btn.classList.toggle("active", currentSpeedUi === 3);
  els.speed4Btn.classList.toggle("active", currentSpeedUi === 4);

  if (els.mixLegacyBtn) {
    els.mixLegacyBtn.classList.toggle("active", currentMixMode === 0);
  }
  if (els.mixStemBtn) {
    els.mixStemBtn.classList.toggle("active", currentMixMode === 1);
  }
  if (els.ymResampledBtn) {
    els.ymResampledBtn.classList.toggle("active", currentYmRenderMode === 1);
  }
  if (els.ymDirectBtn) {
    els.ymDirectBtn.classList.toggle("active", currentYmRenderMode === 0);
  }

  if (els.chALevelValue && els.chALevel) {
    els.chALevelValue.textContent = `${clampInt(els.chALevel.value, 0, 400, 100)}%`;
  }
  if (els.chBLevelValue && els.chBLevel) {
    els.chBLevelValue.textContent = `${clampInt(els.chBLevel.value, 0, 400, 100)}%`;
  }
  if (els.chCLevelValue && els.chCLevel) {
    els.chCLevelValue.textContent = `${clampInt(els.chCLevel.value, 0, 400, 100)}%`;
  }
  if (els.chAPanValue && els.chAPan) {
    els.chAPanValue.textContent = String(clampInt(els.chAPan.value, -100, 100, 0));
  }
  if (els.chBPanValue && els.chBPan) {
    els.chBPanValue.textContent = String(clampInt(els.chBPan.value, -100, 100, 0));
  }
  if (els.chCPanValue && els.chCPan) {
    els.chCPanValue.textContent = String(clampInt(els.chCPan.value, -100, 100, 0));
  }
  if (els.chADriveValue && els.chADrive) {
    els.chADriveValue.textContent = `${clampInt(els.chADrive.value, 0, 100, 0)}%`;
  }
  if (els.chBDriveValue && els.chBDrive) {
    els.chBDriveValue.textContent = `${clampInt(els.chBDrive.value, 0, 100, 0)}%`;
  }
  if (els.chCDriveValue && els.chCDrive) {
    els.chCDriveValue.textContent = `${clampInt(els.chCDrive.value, 0, 100, 0)}%`;
  }

  if (els.irMixValue && els.irMix) {
    els.irMixValue.textContent = `${clampInt(els.irMix.value, 0, 100, 0)}%`;
  }
  if (els.compAmountValue && els.compAmount) {
    els.compAmountValue.textContent = `${clampInt(els.compAmount.value, 0, 100, 0)}%`;
  }

  updateFxGraphParameters();
}

function writeControlBlock(values, markDirty = true) {
  if (!control) {
    return;
  }
  Atomics.store(control, CONTROL_IDX.song, values.song);
  Atomics.store(control, CONTROL_IDX.bank, values.bank);
  Atomics.store(control, CONTROL_IDX.speed, values.speed);
  Atomics.store(control, CONTROL_IDX.drumsOn, values.drumsOn);
  Atomics.store(control, CONTROL_IDX.running, values.running);
  Atomics.store(control, CONTROL_IDX.chALevel, values.chALevel);
  Atomics.store(control, CONTROL_IDX.chBLevel, values.chBLevel);
  Atomics.store(control, CONTROL_IDX.chCLevel, values.chCLevel);
  Atomics.store(control, CONTROL_IDX.chAPan, values.chAPan);
  Atomics.store(control, CONTROL_IDX.chBPan, values.chBPan);
  Atomics.store(control, CONTROL_IDX.chCPan, values.chCPan);
  Atomics.store(control, CONTROL_IDX.chADrive, values.chADrive);
  Atomics.store(control, CONTROL_IDX.chBDrive, values.chBDrive);
  Atomics.store(control, CONTROL_IDX.chCDrive, values.chCDrive);
  Atomics.store(control, CONTROL_IDX.mixMode, values.mixMode);
  Atomics.store(controlU32, CONTROL_IDX.cpuHz, values.cpuHz >>> 0);
  Atomics.store(controlU32, CONTROL_IDX.ymHz, values.ymHz >>> 0);
  Atomics.store(control, CONTROL_IDX.ymRenderMode, values.ymRenderMode);
  if (markDirty) {
    Atomics.store(control, CONTROL_IDX.dirty, 1);
  }
}

function formatBytes(bytes) {
  return `${bytes} bytes`;
}

async function fetchRomManifest() {
  const res = await fetch("./roms/manifest.json");
  if (!res.ok) {
    throw new Error(`Failed to fetch ROM manifest: ${res.status}`);
  }
  const data = await res.json();
  const roms = data && Array.isArray(data.roms) ? data.roms : [];
  return roms.filter((rom) =>
    rom &&
    typeof rom.name === "string" &&
    typeof rom.path === "string" &&
    Number.isFinite(Number(rom.size))
  );
}

function renderRomOptions() {
  if (!els.romSelect) {
    return;
  }
  els.romSelect.innerHTML = "";
  bundledRoms.forEach((rom, index) => {
    const opt = document.createElement("option");
    opt.value = String(index);
    opt.textContent = `${rom.name} (${formatBytes(Number(rom.size))})`;
    els.romSelect.appendChild(opt);
  });
  els.romSelect.disabled = bundledRoms.length === 0;
}

async function loadBundledRomByIndex(index) {
  if (!Number.isInteger(index) || index < 0 || index >= bundledRoms.length) {
    throw new Error("Bundled ROM index out of range.");
  }
  const rom = bundledRoms[index];
  const res = await fetch(rom.path);
  if (!res.ok) {
    throw new Error(`Failed to fetch bundled ROM "${rom.name}": ${res.status}`);
  }
  romBuffer = await res.arrayBuffer();
  currentBundledRomIndex = index;
  if (els.romSelect) {
    els.romSelect.value = String(index);
  }
  if (els.romInfo) {
    els.romInfo.textContent = `Using bundled ROM (${rom.name}, ${formatBytes(romBuffer.byteLength)}).`;
  }
}

async function initBundledRomSelection() {
  bundledRoms = await fetchRomManifest();
  renderRomOptions();
  if (bundledRoms.length === 0) {
    if (els.romInfo) {
      els.romInfo.textContent = "No bundled ROMs found. Upload your own ROM to start playback.";
    }
    return;
  }
  await loadBundledRomByIndex(0);
}

async function fetchIrManifest() {
  const res = await fetch("./irs/manifest.json");
  if (!res.ok) {
    throw new Error(`Failed to fetch IR manifest: ${res.status}`);
  }
  const data = await res.json();
  const irs = data && Array.isArray(data.irs) ? data.irs : [];
  return irs.filter((ir) =>
    ir &&
    typeof ir.name === "string" &&
    typeof ir.path === "string" &&
    Number.isFinite(Number(ir.size))
  );
}

function renderIrOptions() {
  if (!els.irSelect) {
    return;
  }
  els.irSelect.innerHTML = "";
  bundledIrs.forEach((ir, index) => {
    const opt = document.createElement("option");
    opt.value = String(index);
    opt.textContent = `${ir.name} (${formatBytes(Number(ir.size))})`;
    els.irSelect.appendChild(opt);
  });
  els.irSelect.disabled = bundledIrs.length === 0;
}

async function applyBundledIrByIndex(index) {
  if (!ctx || !convolverNode) {
    return;
  }
  if (!Number.isInteger(index) || index < 0 || index >= bundledIrs.length) {
    throw new Error("Bundled IR index out of range.");
  }
  const ir = bundledIrs[index];
  const res = await fetch(ir.path);
  if (!res.ok) {
    throw new Error(`Failed to fetch bundled IR "${ir.name}": ${res.status}`);
  }
  const buf = await res.arrayBuffer();
  const audio = await ctx.decodeAudioData(buf.slice(0));
  convolverNode.buffer = audio;
  updateFxGraphParameters();
  setAudioStateMessage(`IR loaded: ${ir.name}`);
}

async function selectBundledIrByIndex(index) {
  if (!Number.isInteger(index) || index < 0 || index >= bundledIrs.length) {
    throw new Error("Bundled IR index out of range.");
  }
  currentBundledIrIndex = index;
  if (els.irSelect) {
    els.irSelect.value = String(index);
  }
  await applyBundledIrByIndex(index);
}

async function initBundledIrSelection() {
  bundledIrs = await fetchIrManifest();
  renderIrOptions();
  if (bundledIrs.length === 0) {
    throw new Error("No IRs found in manifest.");
  }
  currentBundledIrIndex = 0;
  if (els.irSelect) {
    els.irSelect.value = "0";
  }
}

async function fetchWasmBinary() {
  const res = await fetch("./harmony32_wasm.wasm");
  if (!res.ok) {
    throw new Error(`Failed to fetch wasm binary: ${res.status}`);
  }
  wasmBinaryBuffer = await res.arrayBuffer();
}

async function ensureAudio() {
  if (ctx && node) {
    return;
  }

  if (typeof AudioWorkletNode === "undefined") {
    throw new Error("AudioWorkletNode is not available in this browser.");
  }

  if (typeof SharedArrayBuffer === "undefined") {
    throw new Error("SharedArrayBuffer is unavailable. Set COOP/COEP headers on static hosting.");
  }

  if (!romBuffer) {
    await initBundledRomSelection();
  }
  if (!romBuffer) {
    throw new Error("No ROM loaded. Upload a ROM first or add local files under web/roms/.");
  }
  if (!wasmBinaryBuffer) {
    await fetchWasmBinary();
  }

  controlSab = new SharedArrayBuffer(Int32Array.BYTES_PER_ELEMENT * 32);
  control = new Int32Array(controlSab);
  controlU32 = new Uint32Array(controlSab);
  writeControlBlock(readControls(1), false);

  ctx = new AudioContext({ latencyHint: "interactive" });

  await ctx.audioWorklet.addModule("./harmony32-worklet.js");
  node = new AudioWorkletNode(ctx, "harmony32-processor", {
    numberOfOutputs: 1,
    outputChannelCount: [2],
    processorOptions: {
      wasmUrl: new URL("./harmony32_wasm.wasm", import.meta.url).href,
      wasmBinary: wasmBinaryBuffer,
      romBuffer,
      sampleRate: ctx.sampleRate,
      cpuHz: readCpuHzFromUi(),
      ymHz: readYmHzFromUi(),
      controlSab
    }
  });

  node.port.onmessage = (evt) => {
    const msg = evt.data;
    if (!msg) {
      return;
    }
    if (msg.type === "error") {
      setAudioStateMessage(`error: ${msg.message}`);
      console.error("Worklet error:", msg.message);
    } else if (msg.type === "ready") {
      if (typeof msg.bankCount === "number" && msg.bankCount > 0) {
        setBankCount(msg.bankCount);
        renderControlState();
        applyControls();
      }
      setAudioStateMessage("ready");
    } else if (msg.type === "rom-status") {
      if (!msg.ok) {
        setAudioStateMessage("error: ROM reload failed");
        console.error("ROM reload failed.");
      } else if (typeof msg.bankCount === "number" && msg.bankCount > 0) {
        setBankCount(msg.bankCount);
        renderControlState();
        applyControls();
      }
    } else if (msg.type === "status" && msg.status) {
      if (typeof msg.status.bankCount === "number" && msg.status.bankCount > 0 && msg.status.bankCount !== bankCountUi) {
        setBankCount(msg.status.bankCount);
        renderControlState();
      }
      if (typeof msg.status.bank === "number") {
        const bankUi = msg.status.bank + 1;
        if (bankUi >= 1 && bankUi <= bankCountUi && bankUi !== currentBankUi) {
          currentBankUi = bankUi;
          renderControlState();
        }
      }
      if (typeof msg.status.mixMode === "number" && msg.status.mixMode !== currentMixMode) {
        currentMixMode = msg.status.mixMode === 0 ? 0 : 1;
        renderControlState();
      }
      if (typeof msg.status.ymRenderMode === "number" && msg.status.ymRenderMode !== currentYmRenderMode) {
        currentYmRenderMode = msg.status.ymRenderMode === 0 ? 0 : 1;
        renderControlState();
      }
      if (typeof msg.status.cpuHz === "number" && msg.status.cpuHz > 0 && document.activeElement !== els.cpuFreqInput) {
        syncCpuInputFromHz(msg.status.cpuHz);
      }
      if (typeof msg.status.ymHz === "number" && msg.status.ymHz > 0 && document.activeElement !== els.ymFreqInput) {
        syncYmInputFromHz(msg.status.ymHz);
      }
    }
  };

  convolverNode = new ConvolverNode(ctx, { normalize: true });
  dryGainNode = new GainNode(ctx, { gain: 1.0 });
  wetGainNode = new GainNode(ctx, { gain: 0.0 });
  compressorNode = new DynamicsCompressorNode(ctx);

  node.connect(dryGainNode);
  node.connect(convolverNode);
  convolverNode.connect(wetGainNode);
  dryGainNode.connect(compressorNode);
  wetGainNode.connect(compressorNode);
  compressorNode.connect(ctx.destination);

  if (bundledIrs.length > 0) {
    try {
      await applyBundledIrByIndex(currentBundledIrIndex);
    } catch (err) {
      console.warn("Bundled IR preload failed:", err);
      setAudioStateMessage(`warning: ${String(err)}`);
    }
  }

  updateFxGraphParameters();
}

async function startAudio() {
  await ensureAudio();
  writeControlBlock(readControls(1));
  await ctx.resume();
  setAudioStateMessage(`running @ ${ctx.sampleRate}Hz / CPU ${formatCpuMHz(readCpuHzFromUi() / 1000000)}MHz / YM ${formatCpuMHz(readYmHzFromUi() / 1000000)}MHz`);
  setPlayPauseVisualState(true);
}

async function stopAudio() {
  if (!ctx || !node || !control) {
    return;
  }
  writeControlBlock(readControls(0));
  await ctx.suspend();
  setAudioStateMessage("stopped");
  setPlayPauseVisualState(false);
}

function applyControls() {
  writeControlBlock(readControls());
}

function fullReset() {
  applyControls();
  if (node) {
    node.port.postMessage({ type: "full-reset" });
  }
}

function cpuResetOnly() {
  if (node) {
    node.port.postMessage({ type: "cpu-reset" });
  }
}

async function onBundledRomSelect(ev) {
  const nextIndex = clampInt(ev.target && ev.target.value, 0, bundledRoms.length - 1, currentBundledRomIndex);
  await loadBundledRomByIndex(nextIndex);

  currentSongUi = 1;
  currentBankUi = 1;
  renderControlState();
  applyControls();

  if (node) {
    const transferBuffer = romBuffer;
    const retainedBuffer = romBuffer.slice(0);
    node.port.postMessage({ type: "set-rom", romBuffer: transferBuffer }, [transferBuffer]);
    romBuffer = retainedBuffer;
    node.port.postMessage({ type: "full-reset" });
  }
}

async function onBundledIrSelect(ev) {
  const nextIndex = clampInt(ev.target && ev.target.value, 0, bundledIrs.length - 1, currentBundledIrIndex);
  await selectBundledIrByIndex(nextIndex);
}

async function onRomUpload(ev) {
  const file = ev.target.files && ev.target.files[0];
  if (!file) {
    return;
  }

  romBuffer = await file.arrayBuffer();
  els.romInfo.textContent = `Using upload override (${file.name}, ${romBuffer.byteLength} bytes).`;
  setPlayEnabled(true);

  if (node) {
    node.port.postMessage({ type: "set-rom", romBuffer }, [romBuffer]);
    romBuffer = await file.arrayBuffer();
  }
}

async function onIrUpload(ev) {
  const file = ev.target.files && ev.target.files[0];
  if (!file || !ctx || !convolverNode) {
    return;
  }
  const buf = await file.arrayBuffer();
  const audio = await ctx.decodeAudioData(buf.slice(0));
  convolverNode.buffer = audio;
  setAudioStateMessage(`IR loaded: ${file.name}`);
  updateFxGraphParameters();
}

function wireEvents() {
  const instantApply = () => {
    renderControlState();
    applyControls();
  };

  const instantCpuApply = () => {
    if (parseCpuMHz(els.cpuFreqInput && els.cpuFreqInput.value) === null) {
      return;
    }
    currentCpuHzUi = readCpuHzFromUi();
    applyControls();
  };

  const commitCpuApply = () => {
    syncCpuInputFromHz(readCpuHzFromUi());
    applyControls();
  };

  const instantYmApply = () => {
    if (parseCpuMHz(els.ymFreqInput && els.ymFreqInput.value) === null) {
      return;
    }
    currentYmHzUi = readYmHzFromUi();
    applyControls();
  };

  const commitYmApply = () => {
    syncYmInputFromHz(readYmHzFromUi());
    applyControls();
  };

  const instantUiOnly = () => {
    renderControlState();
  };

  els.playPauseBtn.addEventListener("click", () => {
    const isRunning = ctx && ctx.state === "running";
    const op = isRunning ? stopAudio() : startAudio();
    op.catch((err) => {
      setAudioStateMessage(`error: ${String(err)}`);
      console.error(err);
    });
  });

  els.cpuResetBtn.addEventListener("click", cpuResetOnly);
  els.fullResetBtn.addEventListener("click", fullReset);

  els.songPrevBtn.addEventListener("click", () => {
    currentSongUi = currentSongUi <= 1 ? 16 : currentSongUi - 1;
    instantApply();
  });
  els.songNextBtn.addEventListener("click", () => {
    currentSongUi = currentSongUi >= 16 ? 1 : currentSongUi + 1;
    instantApply();
  });

  els.bankPrevBtn.addEventListener("click", () => {
    if (bankCountUi <= 1) {
      return;
    }
    currentBankUi = currentBankUi <= 1 ? bankCountUi : currentBankUi - 1;
    instantApply();
  });
  els.bankNextBtn.addEventListener("click", () => {
    if (bankCountUi <= 1) {
      return;
    }
    currentBankUi = currentBankUi >= bankCountUi ? 1 : currentBankUi + 1;
    instantApply();
  });

  els.speed1Btn.addEventListener("click", () => {
    currentSpeedUi = 1;
    instantApply();
  });
  els.speed2Btn.addEventListener("click", () => {
    currentSpeedUi = 2;
    instantApply();
  });
  els.speed3Btn.addEventListener("click", () => {
    currentSpeedUi = 3;
    instantApply();
  });
  els.speed4Btn.addEventListener("click", () => {
    currentSpeedUi = 4;
    instantApply();
  });

  els.mixLegacyBtn.addEventListener("click", () => {
    currentMixMode = 0;
    instantApply();
  });
  els.mixStemBtn.addEventListener("click", () => {
    currentMixMode = 1;
    instantApply();
  });
  els.ymResampledBtn.addEventListener("click", () => {
    currentYmRenderMode = 1;
    instantApply();
  });
  els.ymDirectBtn.addEventListener("click", () => {
    currentYmRenderMode = 0;
    instantApply();
  });

  els.drumsSwitch.addEventListener("change", instantApply);
  els.cpuFreqInput.addEventListener("input", instantCpuApply);
  els.cpuFreqInput.addEventListener("change", commitCpuApply);
  els.ymFreqInput.addEventListener("input", instantYmApply);
  els.ymFreqInput.addEventListener("change", commitYmApply);
  els.chALevel.addEventListener("input", instantApply);
  els.chBLevel.addEventListener("input", instantApply);
  els.chCLevel.addEventListener("input", instantApply);
  els.chAPan.addEventListener("input", instantApply);
  els.chBPan.addEventListener("input", instantApply);
  els.chCPan.addEventListener("input", instantApply);
  els.chADrive.addEventListener("input", instantApply);
  els.chBDrive.addEventListener("input", instantApply);
  els.chCDrive.addEventListener("input", instantApply);

  els.irEnable.addEventListener("change", instantUiOnly);
  els.irMix.addEventListener("input", instantUiOnly);
  els.compAmount.addEventListener("input", instantUiOnly);

  els.romUpload.addEventListener("change", (ev) => {
    onRomUpload(ev).catch((err) => {
      setAudioStateMessage(`error: ${String(err)}`);
      console.error(err);
    });
  });

  els.irUpload.addEventListener("change", (ev) => {
    onIrUpload(ev).catch((err) => {
      setAudioStateMessage(`error: ${String(err)}`);
      console.error(err);
    });
  });

  els.irSelect.addEventListener("change", (ev) => {
    onBundledIrSelect(ev).catch((err) => {
      setAudioStateMessage(`error: ${String(err)}`);
      console.error(err);
    });
  });

  els.romSelect.addEventListener("change", (ev) => {
    onBundledRomSelect(ev).catch((err) => {
      setAudioStateMessage(`error: ${String(err)}`);
      console.error(err);
    });
  });
}

setPlayPauseVisualState(false);
setPlayEnabled(false);
syncCpuInputFromHz(DEFAULT_CPU_HZ);
syncYmInputFromHz(DEFAULT_YM_HZ);
renderControlState();
wireEvents();
Promise.all([
  initBundledRomSelection(),
  initBundledIrSelection().catch((err) => {
    setAudioStateMessage(`error: ${String(err)}`);
    console.error(err);
  })
]).then(() => {
  setPlayEnabled(Boolean(romBuffer));
}).catch((err) => {
  setPlayEnabled(false);
  setAudioStateMessage(`error: ${String(err)}`);
  console.error(err);
});
