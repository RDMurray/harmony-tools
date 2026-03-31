#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
WEB_DIR="${ROOT_DIR}/web"
DIST_DIR="${WEB_DIR}/dist"
ROMS_DIR="${WEB_DIR}/roms"
DIST_ROMS_DIR="${DIST_DIR}/roms"
IRS_DIR="${WEB_DIR}/irs"
DIST_IRS_DIR="${DIST_DIR}/irs"

mkdir -p "${DIST_DIR}"
mkdir -p "${DIST_ROMS_DIR}"
mkdir -p "${DIST_IRS_DIR}"

if ! command -v emcc >/dev/null 2>&1; then
  echo "error: emcc not found. Install Emscripten SDK first." >&2
  exit 1
fi

emcc \
  "${ROOT_DIR}/harmony32_web_api.c" \
  "${ROOT_DIR}/harmony32_board.c" \
  "${ROOT_DIR}/z80_mini.c" \
  "${ROOT_DIR}/ym2149_core_standalone.c" \
  -O3 \
  -s WASM=1 \
  -s MODULARIZE=1 \
  -s EXPORT_ES6=1 \
  -s ENVIRONMENT='web,worker' \
  -s ALLOW_MEMORY_GROWTH=1 \
  -s EXPORTED_FUNCTIONS='["_malloc","_free","_h32_create","_h32_destroy","_h32_load_rom","_h32_set_controls","_h32_set_cpu_hz","_h32_set_ym_hz","_h32_set_ym_backend","_h32_set_ym_render_mode","_h32_set_channel_mix","_h32_set_mix_mode","_h32_reset_song","_h32_reset_cpu","_h32_reset_full","_h32_get_bank_count","_h32_render","_h32_render_stems","_h32_get_status"]' \
  -s EXPORTED_RUNTIME_METHODS='["HEAPU8","HEAPU32","HEAPF32"]' \
  -o "${DIST_DIR}/harmony32_wasm.js"

cp "${WEB_DIR}/index.html" "${DIST_DIR}/index.html"
cp "${WEB_DIR}/style.css" "${DIST_DIR}/style.css"
cp "${WEB_DIR}/app.js" "${DIST_DIR}/app.js"
cp "${WEB_DIR}/harmony32-worklet.js" "${DIST_DIR}/harmony32-worklet.js"
cp "${WEB_DIR}/_headers" "${DIST_DIR}/_headers"

find "${DIST_ROMS_DIR}" -maxdepth 1 -type f \( -iname "*.bin" -o -name "manifest.json" \) -delete
find "${DIST_IRS_DIR}" -maxdepth 1 -type f \( -iname "*.wav" -o -name "manifest.json" \) -delete

mapfile -t ROM_NAMES < <(find "${ROMS_DIR}" -maxdepth 1 -type f -iname "*.bin" -printf "%f\n" | LC_ALL=C sort)

json_escape() {
  printf '%s' "$1" | sed -e 's/\\/\\\\/g' -e 's/"/\\"/g'
}

{
  echo '{'
  echo '  "roms": ['
  for i in "${!ROM_NAMES[@]}"; do
    name="${ROM_NAMES[$i]}"
    src="${ROMS_DIR}/${name}"
    dst="${DIST_ROMS_DIR}/${name}"
    cp "${src}" "${dst}"
    size="$(wc -c < "${src}" | tr -d '[:space:]')"
    esc_name="$(json_escape "${name}")"
    comma=","
    if [ "$i" -eq "$(( ${#ROM_NAMES[@]} - 1 ))" ]; then
      comma=""
    fi
    printf '    { "name": "%s", "path": "./roms/%s", "size": %s }%s\n' "${esc_name}" "${esc_name}" "${size}" "${comma}"
  done
  echo '  ]'
  echo '}'
} > "${DIST_ROMS_DIR}/manifest.json"

mapfile -t IR_NAMES < <(find "${IRS_DIR}" -maxdepth 1 -type f -iname "*.wav" -printf "%f\n" | LC_ALL=C sort)
if [ "${#IR_NAMES[@]}" -eq 0 ]; then
  echo "error: no IR files found in ${IRS_DIR} (*.wav)." >&2
  exit 1
fi

{
  echo '{'
  echo '  "irs": ['
  for i in "${!IR_NAMES[@]}"; do
    name="${IR_NAMES[$i]}"
    src="${IRS_DIR}/${name}"
    dst="${DIST_IRS_DIR}/${name}"
    cp "${src}" "${dst}"
    size="$(wc -c < "${src}" | tr -d '[:space:]')"
    esc_name="$(json_escape "${name}")"
    comma=","
    if [ "$i" -eq "$(( ${#IR_NAMES[@]} - 1 ))" ]; then
      comma=""
    fi
    printf '    { "name": "%s", "path": "./irs/%s", "size": %s }%s\n' "${esc_name}" "${esc_name}" "${size}" "${comma}"
  done
  echo '  ]'
  echo '}'
} > "${DIST_IRS_DIR}/manifest.json"

echo "Built web app into ${DIST_DIR}"
