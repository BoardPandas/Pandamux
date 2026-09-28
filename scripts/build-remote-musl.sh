#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET_DIR="${REPO_ROOT}/target"
OUTPUT_DIR="${REPO_ROOT}/resources/remote-binaries"

mkdir -p "${OUTPUT_DIR}"

TARGETS=("x86_64-unknown-linux-musl" "aarch64-unknown-linux-musl")

echo "Building Linux musl binaries for remote environments..."

for TARGET in "${TARGETS[@]}"; do
  echo "Target: ${TARGET}"
  if command -v cross &> /dev/null; then
    cross build --release --target "${TARGET}" -p pandamux-server -p pandamux-cli
  else
    cargo build --release --target "${TARGET}" -p pandamux-server -p pandamux-cli || {
      echo "Standard cargo build failed for ${TARGET}; consider installing musl-tools or cross."
    }
  fi

  ARCH_DIR="${OUTPUT_DIR}/${TARGET}"
  mkdir -p "${ARCH_DIR}"

  for BIN in "pandamux-server" "pandamux"; do
    SRC="${TARGET_DIR}/${TARGET}/release/${BIN}"
    if [ -f "${SRC}" ]; then
      cp -f "${SRC}" "${ARCH_DIR}/${BIN}"
      HASH=$(sha256sum "${SRC}" | awk '{print $1}')
      SIZE=$(stat -c%s "${SRC}" 2>/dev/null || stat -f%z "${SRC}" 2>/dev/null || wc -c < "${SRC}")
      echo "  ${BIN}: sha256=${HASH} size=${SIZE} bytes"
    fi
  done
done

echo "Remote musl build completed."
