# Harmony32

Reverse-engineering, playback, and conversion tools for the Microminiatures Harmony 32 / Harmony 64 chime boxes and their YM2149-based music system.

## What Is Here

- Native/player core in C for Z80 + YM2149 playback and analysis
- Browser player in `web/`
- Rust `harmony-midi` tool for firmware extraction/build and Harmony<->MIDI conversion

## ROM Policy

This repository does not include any firmware ROM dumps.

- The tools and web app can work with user-supplied ROM files.
- For the web app, place your own ROM dumps in `web/roms/` locally before building, or use the upload control in the browser.
- For CLI workflows, pass your own firmware/code files to the Rust tool commands.

## Build

Rust tool:

```bash
cargo test
```

Web app:

```bash
cd web
./build.sh
```

See `web/README.md` for browser build and hosting details.

## License And Provenance

This repository is licensed under the BSD 3-Clause License. See `LICENSE`.

`ym2149_core_standalone.c/.h` are adapted from the MAME AY-3-8910 / YM2149 implementation lineage and retain BSD-3-Clause attribution. The original copied MAME reference files are not required for building this project and are not included in this public tree.
