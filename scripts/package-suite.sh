#!/usr/bin/env bash
# EVO Linux Control Suite — Empaquetado del release autónomo
#
# Genera dist/evo-suite-linux-x86_64.tar.gz con:
#   evo-daemon, evo-desktop, evo (CLI), packaging/ (udev, systemd, iconos) e install.sh
# y dist/install.sh (bootstrap que descarga el tarball y lo instala).
#
# No requiere git: solo los binarios ya compilados.
#
# Uso:
#   scripts/package-suite.sh [directorio-de-salida]
#
# Variables:
#   EVO_DAEMON_BIN   Ruta al binario evo-daemon (por defecto target/release/)
#   EVO_DESKTOP_BIN  Ruta al binario evo-desktop (por defecto target/release/)
#   EVO_VERSION      Versión a mostrar (por defecto la de Cargo.toml)

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT_DIR="${1:-${ROOT_DIR}/dist}"
DAEMON_BIN="${EVO_DAEMON_BIN:-${ROOT_DIR}/target/release/evo-daemon}"
DESKTOP_BIN="${EVO_DESKTOP_BIN:-${ROOT_DIR}/target/release/evo-desktop}"
PACKAGING_SRC="${ROOT_DIR}/crates/evo-daemon/packaging"
VERSION="${EVO_VERSION:-$(grep -m1 '^version' "${ROOT_DIR}/Cargo.toml" | cut -d'"' -f2)}"

STAGE="${OUT_DIR}/stage"
SUITE_NAME="evo-suite-linux-x86_64"
SUITE_DIR="${STAGE}/${SUITE_NAME}"

die() {
  echo "Error: $*" >&2
  exit 1
}

[[ -x "${DAEMON_BIN}" ]] || die "No existe el binario del daemon: ${DAEMON_BIN}"
[[ -d "${PACKAGING_SRC}" ]] || die "No existe el directorio de empaquetado: ${PACKAGING_SRC}"

echo "Empaquetando EVO Linux Control Suite ${VERSION}..."
rm -rf "${STAGE}"
mkdir -p "${SUITE_DIR}/packaging" "${OUT_DIR}"

install -m 755 "${DAEMON_BIN}" "${SUITE_DIR}/evo-daemon"
install -m 755 "${ROOT_DIR}/scripts/evo" "${SUITE_DIR}/evo"
install -m 755 "${ROOT_DIR}/scripts/install.sh" "${SUITE_DIR}/install.sh"
cp -f "${PACKAGING_SRC}"/* "${SUITE_DIR}/packaging/"
chmod 644 "${SUITE_DIR}/packaging/"*
printf '%s\n' "${VERSION}" >"${SUITE_DIR}/VERSION"

if [[ -x "${DESKTOP_BIN}" ]]; then
  install -m 755 "${DESKTOP_BIN}" "${SUITE_DIR}/evo-desktop"
  echo "  ✓ evo-desktop incluido"
else
  echo "  ! evo-desktop no encontrado; el paquete solo incluirá el daemon y el control remoto"
fi

tar -czf "${OUT_DIR}/${SUITE_NAME}.tar.gz" -C "${STAGE}" "${SUITE_NAME}"
install -m 755 "${ROOT_DIR}/scripts/install-bootstrap.sh" "${OUT_DIR}/install.sh"

(cd "${OUT_DIR}" && sha256sum "${SUITE_NAME}.tar.gz" install.sh >SHA256SUMS)

echo
echo "Listo:"
ls -lh "${OUT_DIR}/${SUITE_NAME}.tar.gz" "${OUT_DIR}/install.sh" "${OUT_DIR}/SHA256SUMS"
