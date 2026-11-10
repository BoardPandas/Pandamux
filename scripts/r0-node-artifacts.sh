#!/usr/bin/env bash
set -euo pipefail

PANDAMUX_R0_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PANDAMUX_R0_TARGET="x86_64-unknown-linux-musl"
PANDAMUX_R0_TARGET_DIR="${CARGO_TARGET_DIR:-${PANDAMUX_R0_ROOT}/target}"
PANDAMUX_R0_DEFAULT_OUTPUT="${PANDAMUX_R0_TARGET_DIR}/r0-node-artifacts/${PANDAMUX_R0_TARGET}"
PANDAMUX_R0_MANIFEST="r0-node-manifest.txt"

manifest_value() {
  local manifest_path="$1"
  local key="$2"
  awk -F= -v wanted="${key}" '$1 == wanted { print substr($0, index($0, "=") + 1); found = 1; exit } END { if (!found) exit 1 }' "${manifest_path}"
}

file_size() {
  stat -c '%s' "$1"
}

verify_directory() {
  local artifact_dir="$1"
  local manifest_path="${artifact_dir}/${PANDAMUX_R0_MANIFEST}"
  test -f "${manifest_path}" || {
    echo "Missing R0 node manifest: ${manifest_path}" >&2
    return 1
  }

  test "$(manifest_value "${manifest_path}" format)" = "pandamux-r0-node-artifact-v1"
  test "$(manifest_value "${manifest_path}" publication)" = "disabled"
  test "$(manifest_value "${manifest_path}" target)" = "${PANDAMUX_R0_TARGET}"

  local recorded_commit
  local recorded_version
  recorded_commit="$(manifest_value "${manifest_path}" commit)"
  recorded_version="$(manifest_value "${manifest_path}" version)"
  [[ "${recorded_commit}" =~ ^[0-9a-f]{40}$ ]]
  [[ "${recorded_version}" =~ ^[0-9]+\.[0-9]+\.[0-9]+([.-][0-9A-Za-z.-]+)?$ ]]

  local kind
  for kind in server cli; do
    local filename
    local expected_size
    local expected_sha
    local artifact_path
    filename="$(manifest_value "${manifest_path}" "${kind}.filename")"
    expected_size="$(manifest_value "${manifest_path}" "${kind}.size")"
    expected_sha="$(manifest_value "${manifest_path}" "${kind}.sha256")"
    artifact_path="${artifact_dir}/${filename}"

    test -f "${artifact_path}" || {
      echo "Missing ${kind} binary: ${artifact_path}" >&2
      return 1
    }
    test -x "${artifact_path}" || {
      echo "Binary is not executable: ${artifact_path}" >&2
      return 1
    }
    [[ "${expected_size}" =~ ^[1-9][0-9]*$ ]]
    [[ "${expected_sha}" =~ ^[0-9a-f]{64}$ ]]
    test "$(file_size "${artifact_path}")" = "${expected_size}"
    test "$(sha256sum "${artifact_path}" | awk '{print $1}')" = "${expected_sha}"
  done

  echo "Verified R0 node artifacts in ${artifact_dir}"
}

build_artifacts() {
  local artifact_dir="$1"
  if test -n "$(git -C "${PANDAMUX_R0_ROOT}" status --porcelain)"; then
    echo "R0 node artifacts require a clean source tree so the recorded commit is exact." >&2
    return 1
  fi

  local builder
  if command -v cross >/dev/null 2>&1; then
    builder="cross"
  else
    builder="cargo"
  fi

  CARGO_TARGET_DIR="${PANDAMUX_R0_TARGET_DIR}" "${builder}" build \
    --release \
    --target "${PANDAMUX_R0_TARGET}" \
    -p pandamux-server \
    -p pandamux-cli

  local built_dir="${PANDAMUX_R0_TARGET_DIR}/${PANDAMUX_R0_TARGET}/release"
  local built_server="${built_dir}/pandamux-server"
  local built_cli="${built_dir}/pandamux-cli"
  test -f "${built_server}" || {
    echo "Build did not produce ${built_server}" >&2
    return 1
  }
  test -f "${built_cli}" || {
    echo "Build did not produce ${built_cli}" >&2
    return 1
  }

  mkdir -p "${artifact_dir}"
  install -m 0755 "${built_server}" "${artifact_dir}/pandamux-server"
  install -m 0755 "${built_cli}" "${artifact_dir}/pandamux"

  local commit
  local version
  local generated_at
  local server_size
  local server_sha
  local cli_size
  local cli_sha
  commit="$(git -C "${PANDAMUX_R0_ROOT}" rev-parse HEAD)"
  version="$(awk '
    /^\[workspace.package\]$/ { in_package = 1; next }
    /^\[/ { in_package = 0 }
    in_package && /^version = / { gsub(/[" ]/, "", $3); print $3; exit }
  ' "${PANDAMUX_R0_ROOT}/Cargo.toml")"
  generated_at="$(date -u '+%Y-%m-%dT%H:%M:%SZ')"
  server_size="$(file_size "${artifact_dir}/pandamux-server")"
  server_sha="$(sha256sum "${artifact_dir}/pandamux-server" | awk '{print $1}')"
  cli_size="$(file_size "${artifact_dir}/pandamux")"
  cli_sha="$(sha256sum "${artifact_dir}/pandamux" | awk '{print $1}')"

  local manifest_temp="${artifact_dir}/${PANDAMUX_R0_MANIFEST}.tmp"
  {
    echo "format=pandamux-r0-node-artifact-v1"
    echo "publication=disabled"
    echo "commit=${commit}"
    echo "version=${version}"
    echo "target=${PANDAMUX_R0_TARGET}"
    echo "generatedAt=${generated_at}"
    echo "server.filename=pandamux-server"
    echo "server.size=${server_size}"
    echo "server.sha256=${server_sha}"
    echo "cli.filename=pandamux"
    echo "cli.size=${cli_size}"
    echo "cli.sha256=${cli_sha}"
  } > "${manifest_temp}"
  mv "${manifest_temp}" "${artifact_dir}/${PANDAMUX_R0_MANIFEST}"

  verify_directory "${artifact_dir}"
  echo "Publication remains disabled. Transfer this directory, then run the verify command against the destination."
}

case "${1:-}" in
  build)
    build_artifacts "${2:-${PANDAMUX_R0_DEFAULT_OUTPUT}}"
    ;;
  verify)
    test -n "${2:-}" || {
      echo "Usage: $0 verify <artifact-directory>" >&2
      exit 2
    }
    verify_directory "$2"
    ;;
  *)
    echo "Usage: $0 build [output-directory] | verify <artifact-directory>" >&2
    exit 2
    ;;
esac
