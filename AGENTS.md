# Harmony32 Reverse-Engineering Notes (Current State)

## Scope
This file captures the current reverse-engineering and implementation status of the original `M27C256B@DIP28.BIN` target, the native YM2149 playback paths, and the in-browser realtime player path. The ROM dump itself is not tracked in this repository.

## Current Files
- `disassembly_0000_01DF.asm`: labeled disassembly of the music engine block.
- `z80_mini.c/.h`: instruction-decoding Z80 runtime used by current playback paths.
- `harmony32_board.c/.h`: virtual hardware board model (ROM/RAM decode, bank pin, CPU + I/O wiring).
- `z80_music_engine_equiv.c/.h`: legacy C translation of engine logic (kept for reference/comparison).
- `ym2149_core_standalone.c/.h`: standalone YM2149 core adapted from MAME AY/YM behavior.
- `ym2149_basic_wav.c`: WAV renderer using `harmony32_board` + `z80_mini` + YM2149 core.
- `harmony32_web_api.c/.h`: browser-facing C API for realtime rendering/control (`h32_*` exports).
- `web/`: static web app (`AudioWorklet` runtime, UI, build/deploy scripts, Cloudflare headers).

## ROM Layout (Per 16 KiB Bank)
Each bank uses this Z80-visible address map template (`0x0000..0x3FFF`):
- `0x0000..0x01DF`: engine code.
- `0x01E0..0x01FF`: 16-song pointer table (16 x little-endian 16-bit pointers).
- `0x0200..0x02FF`: note-period lookup table (128 x little-endian 16-bit entries, includes zero gaps).
- `0x0300..`: linear event streams.

Both banks currently use identical pointer values:
`0x0300, 0x06D0, 0x0AA0, 0x0E70, 0x1240, 0x1610, 0x19E0, 0x1DB0, 0x2180, 0x2550, 0x2920, 0x2CF0, 0x30C0, 0x3490, 0x3860, 0x3C30`.

## Device Map (Inferred)

### Program/ROM
- Z80 executes from `0x0000` through board memory callbacks.
- ROM decode for `0x0000..0x3FFF` uses virtual bank pin:
  - physical ROM address = `(bank_pin << 14) | addr`.
- Bank switching is hardware-style pin toggling (no runtime ROM memcpy into CPU context).
- Bank count is dynamic: `ceil(rom_len / 0x4000)`; partial final bank is valid and unread bytes return `0xFF`.

### RAM
- Explicit engine state: `0x4000..0x400C`.
- Stack expected around `SP=0x4800`.

### RAM Variables (`0x4000..0x400B`)
- `0x4000..0x4002`: channel duration counters A/B/C.
- `0x4003..0x4005`: channel amplitude-decay working values A/B/C.
- `0x4006`: current event duration (`ctl & 0x7F`).
- `0x4007`: current event flag (derived from `ctl & 0x80`).
- `0x4008..0x4009`: note period low/high bytes.
- `0x400A`: mixer control written to PSG reg7 (`0x08` or `0x38`).
- `0x400B..0x400C`: current stream pointer.

## I/O Ports
- `IN  (0x01)`: controls.
- `OUT (0x03)`: PSG register address/select.
- `OUT (0x02)`: PSG register data.

## Input Bit Mapping (`port 0x01`)
- Bits `0..3`: song index `0..15`.
- Bits `4..5`: speed selector `0..3`.
- Bit `6`: drum/noise mode (`reg7 = 0x08` when clear, `0x38` when set).
- Bit `7`: unused by this routine.

## Song Data Format
- Record format: 2 bytes (`idx`, `ctl`) from stream.
- `idx == 0xFF` terminates song.
- `ctl & 0x7F` is duration base.
- `ctl & 0x80` sets event behavior flag.
- No in-stream jump opcodes observed in this routine.

## Note Table Format (`0x0200..0x02FF`)
- 128 x little-endian 16-bit values.
- Stream `idx` is used as byte offset into table base `0x0200`.
- Practical stream indices are even-aligned for 16-bit periods.

## Timing Model
- Playback delay is software-loop based (`DEC C` + `DJNZ`) and CPU-clock dependent.
- Tempo depends on speed bits and assumed Z80 clock.

## YM2149 Implementation Status
- The old simplified square-wave YM model has been removed from runtime path.
- Active native path uses `ym2149_core_standalone` in `ym2149_basic_wav.c`.
- Active browser path uses `ym2149_core_standalone` through `harmony32_web_api.c` inside a WASM `AudioWorklet`.
- Core includes tone/noise/envelope stepping and MAME-derived YM2149 volume tables.
- Current renderer defaults:
  - `cpu_hz=2000000`
  - `tail_ms=0`
- Volume registers `8..10` are clamped to 4-bit fixed volume for this board path (prevents unintended envelope-enable artifacts in current content).

## Web App Status
- Web build target: Emscripten (`emcc`) output to `web/dist` via `web/build.sh`.
- Deployment helper: `web/deploy.sh` (build + `wrangler pages deploy`).
- Hosting model: fully static; Cloudflare Pages headers in `web/_headers` set `COOP/COEP` for `SharedArrayBuffer`.
- Audio model: realtime synthesis in `AudioWorklet`; UI control state passed via shared memory.
- Playback behavior in realtime mode:
  - song loops indefinitely (stream end auto-reinitializes current song/bank),
  - song/bank/speed/drums updates apply immediately (next audio quantum),
  - bank-only change toggles hardware bank pin live without full engine reset,
  - UI uses song `1..16`, bank `1..N` with prev/next wrap, speed `1..4` where `4` maps to fastest engine speed.
  - UI exposes `CPU Reset` (CPU-only) and `Full Reset` (board/song reinit) recovery controls.

## Z80 Runtime Status
- Active runtime path uses `z80_mini` for CPU execution and `harmony32_board` for memory/I/O hardware mapping.
- Implemented opcode surface currently covers the ROM engine block at `0x0000..0x01DF` (including `CB`/`ED` ops used by this firmware path).
- Song init/step flow is driven by real PC/SP/stack execution; board stepping returns T-state totals per music tick.
- Legacy `z80_music_engine_equiv` remains in-tree for reference and A/B analysis, but is no longer on active web/native runtime paths.

## Known Remaining Accuracy Work
- Per-write sub-tick PSG scheduling is not yet implemented.
- Timing is step-accurate but not yet write-timestamp accurate within each engine step.
- `z80_mini` is currently firmware-opcode-surface complete for this ROM path, not a full general-purpose Z80 implementation.
