#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
PROJECT_NAME="${1:-harmony32}"

"${ROOT_DIR}/web/build.sh"

cd "${ROOT_DIR}/web"
npx wrangler pages deploy dist --project-name "${PROJECT_NAME}" --commit-dirty=true
