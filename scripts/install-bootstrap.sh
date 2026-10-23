#!/usr/bin/env bash
# EVO Linux Control Suite — Bootstrap del instalador
#
# Descarga el paquete precompilado del último release y ejecuta el instalador.
# No requiere git ni Rust: solo curl (o wget) y tar.
#
# Uso:
#   curl -fsSL https://github.com/SebaAguiar/evo-linux-control-suite/releases/latest/download/install.sh | bash
#   curl -fsSL .../install.sh | bash -s -- --no-udev
#
# Variables:
#   EVO_VERSION=v1.0.0   Fijar una versión en vez de "latest"
#   EVO_REPO=usuario/repo  Usar un fork

set -euo pipefail

REPO_SLUG="${EVO_REPO:-SebaAguiar/evo-linux-control-suite}"
CHANNEL="${EVO_VERSION:-latest}"
BASE_URL="https://github.com/${REPO_SLUG}/releases/${CHANNEL}/download"
ASSET="evo-suite-linux-x86_64.tar.gz"

if [[ -t 1 ]]; then
  BOLD="\033[1m"
  CYAN="\033[1;36m"
  RED="\033[1;31m"
  GREEN="\033[1;32m"
  RESET="\033[0m"
else
  BOLD=""
  CYAN=""
  RED=""
  GREEN=""
  RESET=""
fi

die() {
  echo -e "${RED}Error:${RESET} $*" >&2
  exit 1
}

if command -v curl >/dev/null 2>&1; then
  DOWNLOADER="curl"
elif command -v wget >/dev/null 2>&1; then
  DOWNLOADER="wget"
else
  die "Necesitás curl o wget instalados."
fi

download() {
  local url="$1" dest="$2"
  if [[ "${DOWNLOADER}" == "curl" ]]; then
    curl -fsSL --retry 3 --connect-timeout 15 -o "${dest}" "${url}"
  else
    wget -q --tries=3 --timeout=20 -O "${dest}" "${url}"
  fi
}

TMP_DIR="$(mktemp -d -t evo-bootstrap-XXXXXX)"
cleanup() { rm -rf "${TMP_DIR}"; }
trap cleanup EXIT

echo -e "${BOLD}=== EVO Linux Control Suite ===${RESET}"
echo "Descargando el paquete precompilado (${CHANNEL})..."
echo "  ${BASE_URL}/${ASSET}"

download "${BASE_URL}/${ASSET}" "${TMP_DIR}/${ASSET}" ||
  die "No se pudo descargar el paquete. Verificá tu conexión o que el release exista."

if download "${BASE_URL}/SHA256SUMS" "${TMP_DIR}/SHA256SUMS" 2>/dev/null; then
  if grep -q " ${ASSET}\$" "${TMP_DIR}/SHA256SUMS"; then
    echo "Verificando checksum..."
    (cd "${TMP_DIR}" && grep " ${ASSET}\$" SHA256SUMS | sha256sum -c - >/dev/null) ||
      die "El checksum no coincide. La descarga se corrompió; volvé a intentar."
  fi
fi

tar -xzf "${TMP_DIR}/${ASSET}" -C "${TMP_DIR}" ||
  die "No se pudo extraer el paquete."

SUITE_DIR="${TMP_DIR}/evo-suite-linux-x86_64"
[[ -x "${SUITE_DIR}/install.sh" ]] || die "El paquete no contiene el instalador."

echo -e "${GREEN}Paquete listo.${RESET} Ejecutando el instalador..."
echo

"${SUITE_DIR}/install.sh" "$@"
