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

## Usage

Download prebuilt `harmony-midi` binaries from the [latest release](https://github.com/RDMurray/harmony-tools/releases/latest).

If you are running from source, use `cargo run -- <command>` in place of `harmony-midi <command>`.

```bash
harmony-midi extract-firmware M27C256B.BIN extracted/
harmony-midi harmony-to-midi extracted/code.bin extracted/song01.bin song01.mid
harmony-midi midi-to-harmony extracted/code.bin song01.mid song01.bin
harmony-midi build-firmware extracted/ rebuilt.bin
```

For full command help:

```bash
harmony-midi --help
harmony-midi <command> --help
```

## Build And Development

Rust tool:

```bash
cargo test
```

Rust tool release:

```bash
git tag v0.1.0
git push origin v0.1.0
```

This triggers the GitHub Actions release workflow for `harmony-midi` only and uploads Linux, macOS, and Windows CLI archives to the GitHub Release.

Web app:

```bash
cd web
./build.sh
```

See `web/README.md` for browser build and hosting details.

## License And Provenance

This repository is licensed under the BSD 3-Clause License. See `LICENSE`.

`ym2149_core_standalone.c/.h` are adapted from the MAME AY-3-8910 / YM2149 implementation lineage and retain BSD-3-Clause attribution. The original copied MAME reference files are not required for building this project and are not included in this public tree.
