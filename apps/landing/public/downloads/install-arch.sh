#!/usr/bin/env bash
# EVO Linux Control Suite — Instalador para Arch Linux / CachyOS
#
# Descarga el paquete nativo precompilado, instala las dependencias de runtime
# que falten y lo instala con pacman.
#
# Uso:
#   bash install-arch.sh                    # descarga e instala el release actual
#   bash install-arch.sh ./paquete.zst      # instala un paquete local
#   bash install-arch.sh --no-deps          # no instalar dependencias
#
# Variables:
#   EVO_DOWNLOAD_BASE  Base de descarga (por defecto la landing del proyecto)
#   EVO_VERSION        Fijar una versión del paquete (por defecto 1.0.0-1)

set -euo pipefail

PKG_NAME="evo-linux-control-suite-bin"
PKG_VERSION="${EVO_VERSION:-1.0.0-1}"
PKG_FILE="${PKG_NAME}-${PKG_VERSION}-x86_64.pkg.tar.zst"
DOWNLOAD_BASE="${EVO_DOWNLOAD_BASE:-https://sebaaguiar.github.io/evo-linux-control-suite/downloads}"
ARCH_DEPS=(webkit2gtk-4.1 gtk3 gtk-layer-shell)

if [[ -t 1 ]]; then
  BOLD="\033[1m"
  CYAN="\033[1;36m"
  YELLOW="\033[1;33m"
  RED="\033[1;31m"
  GREEN="\033[1;32m"
  RESET="\033[0m"
else
  BOLD=""
  CYAN=""
  YELLOW=""
  RED=""
  GREEN=""
  RESET=""
fi

info() { echo -e "${CYAN}==>${RESET} $*"; }
ok() { echo -e "  ${GREEN}✓${RESET} $*"; }
warn() { echo -e "${YELLOW}Aviso:${RESET} $*"; }
die() {
  echo -e "${RED}Error:${RESET} $*" >&2
  exit 1
}

usage() {
  cat <<'EOF'
Instalador de EVO Linux Control Suite para Arch Linux / CachyOS

  bash install-arch.sh                 Descarga e instala el paquete del release
  bash install-arch.sh ./paquete.zst   Instala un paquete local
  bash install-arch.sh --no-deps       No instalar dependencias de runtime
  bash install-arch.sh --help          Muestra esta ayuda

Variables: EVO_DOWNLOAD_BASE, EVO_VERSION
EOF
}

INSTALL_DEPS=1
PKG_PATH=""

for arg in "$@"; do
  case "${arg}" in
    --no-deps) INSTALL_DEPS=0 ;;
    -h | --help)
      usage
      exit 0
      ;;
    -*)
      warn "Argumento desconocido: ${arg}"
      usage
      exit 1
      ;;
    *) PKG_PATH="${arg}" ;;
  esac
done

command -v pacman >/dev/null 2>&1 ||
  die "Este instalador es para Arch Linux / CachyOS (no se encontró pacman)."

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

TMP_DIR=""
cleanup() { [[ -n "${TMP_DIR}" ]] && rm -rf "${TMP_DIR}"; }
trap cleanup EXIT

arch_deps_satisfied() {
  pacman -T "${ARCH_DEPS[@]}" >/dev/null 2>&1
}

arch_purge_broken_cache() {
  local dir="/var/cache/pacman/pkg" file
  local broken=()
  [[ -d "${dir}" ]] || return 0

  for file in "${dir}"/*.pkg.tar.*; do
    [[ -e "${file}" ]] || continue
    [[ "${file}" == *.sig ]] && continue
    if head -c 512 "${file}" 2>/dev/null | grep -qiE '<!doctype html|<html'; then
      broken+=("${file}")
    fi
  done

  [[ ${#broken[@]} -gt 0 ]] || return 0

  warn "Cache de pacman corrupto (descargas fallidas guardadas como HTML):"
  printf '     %s\n' "${broken[@]##*/}"
  if sudo rm -f "${broken[@]}" "${broken[@]/%/.sig}" 2>/dev/null; then
    ok "Cache corrupto eliminado."
  else
    warn "No se pudo limpiar el cache corrupto; seguimos igual."
  fi
  return 0
}

arch_confirm_full_upgrade() {
  local answer=""
  echo
  warn "La base de datos de paquetes parece atrasada: los mirrors ya no tienen las versiones que pide pacman."
  warn "Para instalarlas sin dejar el sistema en partial upgrade hay que actualizar todo (pacman -Syu)."
  ( : </dev/tty ) 2>/dev/null || return 1
  read -r -p "     ¿Actualizar el sistema ahora? [s/N] " answer < /dev/tty || answer=""
  case "${answer}" in
    s | S | si | sí | y | Y | yes) return 0 ;;
    *) return 1 ;;
  esac
}

arch_install_deps() {
  if arch_deps_satisfied; then
    ok "Dependencias presentes (${ARCH_DEPS[*]})."
    return 0
  fi

  arch_purge_broken_cache

  if sudo pacman -S --needed --noconfirm "${ARCH_DEPS[@]}"; then
    return 0
  fi

  warn "pacman no pudo instalar las dependencias."
  if ! arch_confirm_full_upgrade; then
    echo "     Actualizá el sistema y reintentá:"
    echo "       sudo pacman -Syu && bash install-arch.sh"
    echo "     Si el problema persiste, refrescá el mirrorlist (p. ej. sudo cachyos-rate-mirrors)."
    return 1
  fi

  if ! sudo pacman -Syu --needed --noconfirm; then
    warn "pacman -Syu falló. Revisá /etc/pacman.d/mirrorlist y reintentá."
    return 1
  fi

  if sudo pacman -S --needed --noconfirm "${ARCH_DEPS[@]}"; then
    return 0
  fi

  warn "Las dependencias siguen sin poder instalarse. Revisá /etc/pacman.d/mirrorlist."
  return 1
}

if [[ -z "${PKG_PATH}" ]]; then
  TMP_DIR="$(mktemp -d -t evo-arch-XXXXXX)"

  info "Descargando ${PKG_FILE}"
  download "${DOWNLOAD_BASE}/${PKG_FILE}" "${TMP_DIR}/${PKG_FILE}" ||
    die "No se pudo descargar ${DOWNLOAD_BASE}/${PKG_FILE}"

  if download "${DOWNLOAD_BASE}/SHA256SUMS" "${TMP_DIR}/SHA256SUMS" 2>/dev/null &&
    grep -q " ${PKG_FILE}\$" "${TMP_DIR}/SHA256SUMS"; then
    info "Verificando checksum"
    (cd "${TMP_DIR}" && grep " ${PKG_FILE}\$" SHA256SUMS | sha256sum -c - >/dev/null) ||
      die "El checksum no coincide. Volvé a intentar."
    ok "Checksum correcto."
  fi

  PKG_PATH="${TMP_DIR}/${PKG_FILE}"
elif [[ ! -f "${PKG_PATH}" ]]; then
  die "No existe el archivo: ${PKG_PATH}"
fi

if [[ "${INSTALL_DEPS}" == "1" ]]; then
  arch_install_deps || die "No se pudieron instalar las dependencias de runtime."
fi

info "Instalando ${PKG_PATH##*/}"
sudo pacman -U --noconfirm "${PKG_PATH}" ||
  die "pacman no pudo instalar el paquete."

ok "EVO Linux Control Suite instalado."
echo "     Para el daemon: systemctl --user enable --now evo-daemon"
echo "     Para la app:    evo-desktop"
