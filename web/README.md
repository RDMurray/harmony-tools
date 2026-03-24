# Harmony32 Web App

This is a fully static in-browser player using:
- C core (`harmony32_board` + `z80_mini` + `ym2149_core_standalone`) compiled to WebAssembly with Emscripten
- `AudioWorklet` realtime rendering
- `SharedArrayBuffer` control channel for low-latency control updates

## Build

```bash
cd web
./build.sh
```

Output is in `web/dist`.

## Serve Locally

Use any static server rooted at `web/dist`.

Example:
```bash
cd web/dist
python3 -m http.server 8080
```

## Cloudflare Pages headers

`dist/_headers` includes:
- `Cross-Origin-Opener-Policy: same-origin`
- `Cross-Origin-Embedder-Policy: require-corp`

These are required for `SharedArrayBuffer` in current browsers including recent iOS Safari.

## Controls

- Song: `0..15`
- Bank: `0..N-1` where `N = ceil(rom_size / 16384)` (partial final bank allowed)
- Speed: `0..3`
- Drums: `on/off`
- Render mode: `Legacy C Mix` or `Stem FX Mix`
- Channel level (A/B/C): `0..400%` (for over-unity drive in stem mode)
- Channel drive (A/B/C): `0..100%` (waveshaper saturation in stem mode)
- Channel pan (A/B/C): `-100..100` (`-100` full left, `100` full right)
- Master IR: bundled picker or upload override + wet/dry control (`ConvolverNode`)
- Master compressor: `0..100%`
- `CPU Reset`: reset only Z80 state (RAM/YM preserved; useful for glitch experiments)
- `Full Reset`: reinitialize current song/bank with board reset (recovery path)

Song/bank/speed/drums/mix/channel settings apply immediately (next audio quantum).

## ROM source

- This repository ships with no ROMs.
- Put your own ROM files in `web/roms/` (`*.bin`, top-level only) if you want local bundled-ROM builds.
- `./build.sh` copies them to `web/dist/roms/` and generates `web/dist/roms/manifest.json`.
- If no local ROMs are present, the build still succeeds and the app starts in upload-only mode.
- If local ROMs are present, the app defaults to the first ROM in sorted filename order.
- Upload override is always available in the UI.

## IR source

- Put bundled impulse responses in `web/irs/` (`*.wav`, top-level only).
- `./build.sh` copies them to `web/dist/irs/` and generates `web/dist/irs/manifest.json`.
- The app defaults to the first IR in sorted filename order.
- Optional upload override is still available in the UI.
