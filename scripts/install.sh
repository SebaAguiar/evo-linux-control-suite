#!/usr/bin/env bash
# EVO Linux Control Suite — Instalador autónomo
#
# Instala EVO Linux Control Suite sin necesidad de git:
#   - Si se ejecuta desde el tarball de release, usa los binarios precompilados
#     que están junto a este script (daemon + app de escritorio + CLI).
#   - Si se ejecuta desde el repositorio, usa los binarios ya compilados o los
#     compila; si no hay toolchain, descarga el paquete precompilado del release.
#
# Uso:
#   ./install.sh                 Instalación normal
#   ./install.sh --no-udev       No tocar /etc/udev (sin sudo)
#   ./install.sh --no-service    No habilitar el servicio systemd de usuario
#   ./install.sh --rebuild       Forzar recompilación desde el código (repo)
#
# Variables de entorno equivalentes: EVO_SKIP_UDEV=1, EVO_SKIP_SERVICE=1

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"

BIN_DIR="${HOME}/.local/bin"
SYSTEMD_USER_DIR="${HOME}/.config/systemd/user"
APPLICATIONS_DIR="${HOME}/.local/share/applications"
ICONS_DIR="${HOME}/.local/share/icons/hicolor"
DATA_DIR="${HOME}/.local/share/evo-suite"

SKIP_UDEV="${EVO_SKIP_UDEV:-0}"
SKIP_SERVICE="${EVO_SKIP_SERVICE:-0}"
REBUILD=0

REPO_SLUG="SebaAguiar/evo-linux-control-suite"
RELEASE_BASE="https://github.com/${REPO_SLUG}/releases/latest/download"
SUITE_TARBALL="evo-suite-linux-x86_64.tar.gz"
DAEMON_TARBALL="evo-daemon-linux-x86_64.tar.gz"

if [[ -t 1 ]]; then
  GREEN="\033[1;32m"
  CYAN="\033[1;36m"
  YELLOW="\033[1;33m"
  RED="\033[1;31m"
  BOLD="\033[1m"
  RESET="\033[0m"
else
  GREEN=""
  CYAN=""
  YELLOW=""
  RED=""
  BOLD=""
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
Instalador de EVO Linux Control Suite

  ./install.sh                 Instala usando binarios precompilados o compila
  ./install.sh --no-udev       No instala la regla udev (no pide sudo)
  ./install.sh --no-service    No habilita el servicio systemd de usuario
  ./install.sh --rebuild       Fuerza recompilación desde el código fuente
  ./install.sh --help          Muestra esta ayuda

Variables: EVO_SKIP_UDEV=1, EVO_SKIP_SERVICE=1
EOF
}

for arg in "$@"; do
  case "${arg}" in
    --rebuild) REBUILD=1 ;;
    --no-udev) SKIP_UDEV=1 ;;
    --no-service) SKIP_SERVICE=1 ;;
    -h | --help)
      usage
      exit 0
      ;;
    *)
      warn "Argumento desconocido: ${arg}"
      usage
      exit 1
      ;;
  esac
done

echo -e "${BOLD}=== Instalador de EVO Linux Control Suite ===${RESET}"
if [[ -f "${SCRIPT_DIR}/VERSION" ]]; then
  echo -e "Versión: ${BOLD}$(cat "${SCRIPT_DIR}/VERSION")${RESET}"
fi
echo

case "$(uname -m)" in
  x86_64 | amd64) ;;
  *) die "Por ahora solo hay binarios para x86_64 (detectado: $(uname -m))." ;;
esac

DISTRO_ID=""
if [[ -r /etc/os-release ]]; then
  # shellcheck disable=SC1091
  . /etc/os-release
  DISTRO_ID="${ID:-} ${ID_LIKE:-}"
fi

# Rust/Cargo en PATH si existe (solo hace falta al compilar desde el código)
if [[ -f "${HOME}/.cargo/env" ]]; then
  # shellcheck source=/dev/null
  source "${HOME}/.cargo/env"
elif [[ -d "${HOME}/.cargo/bin" && ":${PATH}:" != *":${HOME}/.cargo/bin:"* ]]; then
  export PATH="${HOME}/.cargo/bin:${PATH}"
fi

# Recursos de empaquetado (servicio, regla udev, iconos, lanzador)
PACKAGING_DIR=""
if [[ -d "${REPO_DIR}/crates/evo-daemon/packaging" ]]; then
  PACKAGING_DIR="${REPO_DIR}/crates/evo-daemon/packaging"
elif [[ -d "${SCRIPT_DIR}/packaging" ]]; then
  PACKAGING_DIR="${SCRIPT_DIR}/packaging"
fi

EVO_CLI_SRC=""
if [[ -f "${REPO_DIR}/scripts/evo" ]]; then
  EVO_CLI_SRC="${REPO_DIR}/scripts/evo"
elif [[ -f "${SCRIPT_DIR}/evo" ]]; then
  EVO_CLI_SRC="${SCRIPT_DIR}/evo"
fi

TMP_DIR=""
cleanup() {
  if [[ -n "${TMP_DIR}" && -d "${TMP_DIR}" ]]; then
    rm -rf "${TMP_DIR}"
  fi
}
trap cleanup EXIT

find_local_daemon_bin() {
  [[ "${REBUILD}" == "1" ]] && return 1
  if [[ -x "${SCRIPT_DIR}/evo-daemon" ]]; then
    echo "${SCRIPT_DIR}/evo-daemon"
  elif [[ -x "${REPO_DIR}/target/release/evo-daemon" ]]; then
    echo "${REPO_DIR}/target/release/evo-daemon"
  else
    return 1
  fi
}

find_local_desktop_bin() {
  [[ "${REBUILD}" == "1" ]] && return 1
  if [[ -x "${SCRIPT_DIR}/evo-desktop" ]]; then
    echo "${SCRIPT_DIR}/evo-desktop"
  elif [[ -x "${REPO_DIR}/target/release/evo-desktop" ]]; then
    echo "${REPO_DIR}/target/release/evo-desktop"
  else
    return 1
  fi
}

download_file() {
  local url="$1" dest="$2"
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL --retry 3 -o "${dest}" "${url}"
  elif command -v wget >/dev/null 2>&1; then
    wget -q -O "${dest}" "${url}"
  else
    return 1
  fi
}

# Descarga el paquete completo (daemon + desktop + CLI) del último release.
# No requiere git ni Rust.
fetch_prebuilt_suite() {
  TMP_DIR="$(mktemp -d -t evo-suite-XXXXXX)"
  local tarball="${TMP_DIR}/${SUITE_TARBALL}"

  info "Descargando binarios precompilados desde GitHub Releases..."
  if download_file "${RELEASE_BASE}/${SUITE_TARBALL}" "${tarball}"; then
    if tar -xzf "${tarball}" -C "${TMP_DIR}" 2>/dev/null; then
      local suite="${TMP_DIR}/evo-suite-linux-x86_64"
      if [[ -x "${suite}/evo-daemon" ]]; then
        DAEMON_SRC="${suite}/evo-daemon"
        if [[ -x "${suite}/evo-desktop" ]]; then
          DESKTOP_SRC="${suite}/evo-desktop"
        fi
        if [[ -f "${suite}/evo" ]]; then
          EVO_CLI_SRC="${suite}/evo"
        fi
        if [[ -d "${suite}/packaging" ]]; then
          PACKAGING_DIR="${suite}/packaging"
        fi
        ok "Paquete completo descargado."
        return 0
      fi
    fi
  fi

  # Compatibilidad con releases viejos: solo daemon + CLI + packaging.
  warn "No se encontró el paquete completo; intento con el paquete del daemon..."
  local daemon_tar="${TMP_DIR}/${DAEMON_TARBALL}"
  if download_file "${RELEASE_BASE}/${DAEMON_TARBALL}" "${daemon_tar}" &&
    tar -xzf "${daemon_tar}" -C "${TMP_DIR}" 2>/dev/null &&
    [[ -x "${TMP_DIR}/evo-daemon" ]]; then
    DAEMON_SRC="${TMP_DIR}/evo-daemon"
    if [[ -f "${TMP_DIR}/evo" ]]; then
      EVO_CLI_SRC="${TMP_DIR}/evo"
    fi
    if [[ -d "${TMP_DIR}/packaging" ]]; then
      PACKAGING_DIR="${TMP_DIR}/packaging"
    fi
    ok "Paquete del daemon descargado (la app de escritorio no está incluida)."
    return 0
  fi

  return 1
}

try_build_daemon() {
  command -v cargo >/dev/null 2>&1 || return 1
  [[ -f "${REPO_DIR}/Cargo.toml" ]] || return 1
  info "Compilando evo-daemon (release)..."
  (cd "${REPO_DIR}" && cargo build --release -p evo-daemon)
  DAEMON_SRC="${REPO_DIR}/target/release/evo-daemon"
}

try_build_desktop() {
  command -v cargo >/dev/null 2>&1 || return 1
  [[ -d "${REPO_DIR}/apps/desktop" ]] || return 1
  info "Compilando la app de escritorio (Tauri, release)..."
  if command -v bun >/dev/null 2>&1; then
    (cd "${REPO_DIR}/apps/desktop" && bun run tauri build --no-bundle --ignore-version-mismatches)
  elif command -v npm >/dev/null 2>&1; then
    (cd "${REPO_DIR}/apps/desktop" && npm run tauri build -- --no-bundle --ignore-version-mismatches)
  else
    return 1
  fi
  DESKTOP_SRC="${REPO_DIR}/target/release/evo-desktop"
}

# ---------------------------------------------------------------------------
# 1. Resolver binarios
# ---------------------------------------------------------------------------
DAEMON_SRC=""
DESKTOP_SRC=""

if DAEMON_SRC="$(find_local_daemon_bin)"; then
  ok "Daemon precompilado encontrado (${DAEMON_SRC})."
fi

if [[ -z "${DAEMON_SRC}" ]]; then
  if [[ "${REBUILD}" != "1" ]] && fetch_prebuilt_suite; then
    :
  elif try_build_daemon; then
    ok "Daemon compilado."
  elif fetch_prebuilt_suite; then
    :
  else
    die "No se encontraron binarios y no se pudieron descargar. Revisá tu conexión o compilá desde el código con Rust instalado."
  fi
fi

[[ -x "${DAEMON_SRC}" ]] || die "No se pudo resolver el binario evo-daemon."

if DESKTOP_SRC="$(find_local_desktop_bin)"; then
  ok "App de escritorio precompilada encontrada."
elif [[ "${REBUILD}" != "1" && -n "${TMP_DIR:-}" && -x "${TMP_DIR}/evo-suite-linux-x86_64/evo-desktop" ]]; then
  DESKTOP_SRC="${TMP_DIR}/evo-suite-linux-x86_64/evo-desktop"
elif try_build_desktop && [[ -x "${DESKTOP_SRC:-}" ]]; then
  ok "App de escritorio compilada."
else
  if [[ -z "${DESKTOP_SRC}" ]]; then
    warn "La app de escritorio no está disponible en esta máquina."
    echo "     El daemon, el comando 'evo' y el control remoto web sí se van a instalar."
    echo "     Para compilarla: instalá las dependencias de WebKitGTK/GTK3 y volvé a ejecutar."
  fi
fi

# ---------------------------------------------------------------------------
# 2. Instalar ejecutables
# ---------------------------------------------------------------------------
info "Instalando ejecutables en ${BIN_DIR}..."
mkdir -p "${BIN_DIR}"

if command -v systemctl >/dev/null 2>&1 &&
  systemctl --user is-active --quiet evo-daemon.service 2>/dev/null; then
  echo "  Deteniendo el servicio previo para reemplazar el binario..."
  systemctl --user stop evo-daemon.service 2>/dev/null || true
fi

install -m 755 "${DAEMON_SRC}" "${BIN_DIR}/evo-daemon"
ok "${BIN_DIR}/evo-daemon"

if [[ -n "${DESKTOP_SRC:-}" && -f "${DESKTOP_SRC}" ]]; then
  install -m 755 "${DESKTOP_SRC}" "${BIN_DIR}/evo-desktop"
  ok "${BIN_DIR}/evo-desktop"
fi

if [[ -n "${EVO_CLI_SRC}" && -f "${EVO_CLI_SRC}" ]]; then
  install -m 755 "${EVO_CLI_SRC}" "${BIN_DIR}/evo"
  ok "${BIN_DIR}/evo"
  mkdir -p "${HOME}/.config/evo-suite"
  if [[ -f "${REPO_DIR}/Cargo.toml" ]]; then
    echo "${REPO_DIR}" >"${HOME}/.config/evo-suite/repo_path"
  fi
fi

# ---------------------------------------------------------------------------
# 3. Recursos de empaquetado (copia local para referencia/desinstalación)
# ---------------------------------------------------------------------------
mkdir -p "${DATA_DIR}"
if [[ -n "${PACKAGING_DIR}" && -d "${PACKAGING_DIR}" ]]; then
  cp -f "${PACKAGING_DIR}"/* "${DATA_DIR}/" 2>/dev/null || true
fi

# ---------------------------------------------------------------------------
# 4. Regla udev (acceso al dispositivo sin root)
# ---------------------------------------------------------------------------
RULE_SRC="${PACKAGING_DIR}/70-audient-evo.rules"
if [[ -f "${RULE_SRC}" ]]; then
  cp -f "${RULE_SRC}" "${DATA_DIR}/70-audient-evo.rules" 2>/dev/null || true

  if [[ "${SKIP_UDEV}" == "1" ]]; then
    echo "  (omitido) Regla udev — activala luego con:"
    echo "     sudo install -m644 '${DATA_DIR}/70-audient-evo.rules' /etc/udev/rules.d/"
    echo "     sudo udevadm control --reload-rules && sudo udevadm trigger"
  elif command -v sudo >/dev/null 2>&1 && sudo -v; then
    info "Instalando regla udev (acceso sin root al dispositivo)..."
    sudo install -m 644 "${RULE_SRC}" /etc/udev/rules.d/70-audient-evo.rules
    sudo udevadm control --reload-rules 2>/dev/null || true
    sudo udevadm trigger 2>/dev/null || true
    ok "/etc/udev/rules.d/70-audient-evo.rules"
  else
    warn "No se pudo instalar la regla udev (se necesita sudo)."
    echo "     Ejecutá manualmente:"
    echo "       sudo install -m644 '${DATA_DIR}/70-audient-evo.rules' /etc/udev/rules.d/"
    echo "       sudo udevadm control --reload-rules && sudo udevadm trigger"
  fi
fi

# ---------------------------------------------------------------------------
# 5. Servicio systemd de usuario
# ---------------------------------------------------------------------------
if [[ -n "${PACKAGING_DIR}" && -f "${PACKAGING_DIR}/evo-daemon.service" ]]; then
  info "Configurando servicio systemd de usuario..."
  mkdir -p "${SYSTEMD_USER_DIR}"
  cp -f "${PACKAGING_DIR}/evo-daemon.service" "${SYSTEMD_USER_DIR}/evo-daemon.service"
  ok "${SYSTEMD_USER_DIR}/evo-daemon.service"

  user_systemd_available() {
    command -v systemctl >/dev/null 2>&1 &&
      systemctl --user show-environment >/dev/null 2>&1
  }

  if [[ "${SKIP_SERVICE}" == "1" ]]; then
    echo "  (omitido) Servicio systemd — iniciá el daemon con: ${BIN_DIR}/evo-daemon"
  elif user_systemd_available; then
    systemctl --user daemon-reload 2>/dev/null || true
    if systemctl --user enable evo-daemon.service 2>/dev/null; then
      ok "Servicio habilitado al iniciar sesión."
    else
      warn "No se pudo habilitar el servicio. Probá: systemctl --user enable evo-daemon"
    fi

    if systemctl --user restart evo-daemon.service 2>/dev/null; then
      sleep 1
      if systemctl --user is-active --quiet evo-daemon.service; then
        ok "Daemon corriendo en segundo plano."
      else
        warn "El servicio no quedó activo. Mirá: journalctl --user -u evo-daemon -n 50"
      fi
    else
      warn "El daemon no pudo arrancar. Ejecutalo a mano para ver el error: ${BIN_DIR}/evo-daemon"
    fi
  else
    warn "systemd de usuario no disponible en esta sesión."
    echo "     Iniciá el daemon manualmente: ${BIN_DIR}/evo-daemon"
  fi
fi

# ---------------------------------------------------------------------------
# 6. Iconos y lanzador de escritorio
# ---------------------------------------------------------------------------
if [[ -n "${PACKAGING_DIR}" && -d "${PACKAGING_DIR}" ]]; then
  if [[ -f "${PACKAGING_DIR}/evo-control.svg" ]]; then
    mkdir -p "${ICONS_DIR}/scalable/apps" "${ICONS_DIR}/512x512/apps"
    cp -f "${PACKAGING_DIR}/evo-control.svg" "${ICONS_DIR}/scalable/apps/evo-control.svg"
    if [[ -f "${PACKAGING_DIR}/evo-control.png" ]]; then
      cp -f "${PACKAGING_DIR}/evo-control.png" "${ICONS_DIR}/512x512/apps/evo-control.png"
    fi
    ok "Iconos instalados."
  fi

  if [[ -f "${PACKAGING_DIR}/evo-control.desktop" ]]; then
    mkdir -p "${APPLICATIONS_DIR}"
    cp -f "${PACKAGING_DIR}/evo-control.desktop" "${APPLICATIONS_DIR}/evo-control.desktop"
    command -v update-desktop-database >/dev/null 2>&1 &&
      update-desktop-database "${APPLICATIONS_DIR}" >/dev/null 2>&1 || true
    ok "Lanzador 'EVO Control' instalado."
  fi
fi

# ---------------------------------------------------------------------------
# 7. Dependencias de runtime y PATH
# ---------------------------------------------------------------------------
missing_libs() {
  local bin="$1"
  command -v ldd >/dev/null 2>&1 || return 0
  ldd "${bin}" 2>/dev/null | awk '/not found/ {print $1}' | sort -u | tr '\n' ' '
}

report_missing_libs() {
  local bin="$1" label="$2" missing
  missing="$(missing_libs "${bin}")"
  if [[ -z "${missing}" ]]; then
    return 0
  fi

  warn "${label}: faltan librerías del sistema: ${missing}"
  case "${DISTRO_ID}" in
    *arch*)
      echo "     sudo pacman -S --needed webkit2gtk-4.1 gtk3 alsa-lib systemd-libs"
      ;;
    *debian* | *ubuntu*)
      echo "     sudo apt install libwebkit2gtk-4.1-0 libgtk-3-0 libasound2 libudev1"
      ;;
    *fedora* | *rhel*)
      echo "     sudo dnf install webkit2gtk4.1 gtk3 alsa-lib systemd-libs"
      ;;
    *)
      echo "     Instalá WebKitGTK 4.1, GTK3, ALSA y las librerías de udev."
      ;;
  esac
}

report_missing_libs "${BIN_DIR}/evo-daemon" "evo-daemon"
if [[ -x "${BIN_DIR}/evo-desktop" ]]; then
  report_missing_libs "${BIN_DIR}/evo-desktop" "evo-desktop"
fi

if [[ ":${PATH}:" != *":${BIN_DIR}:"* ]]; then
  warn "${BIN_DIR} no está en tu PATH."
  echo "     Agregalo a tu shell, por ejemplo:"
  echo "       echo 'export PATH=\"\$HOME/.local/bin:\$PATH\"' >> ~/.bashrc && source ~/.bashrc"
fi

# ---------------------------------------------------------------------------
# 8. Resumen
# ---------------------------------------------------------------------------
VERSION=""
if [[ -f "${DATA_DIR}/VERSION" ]]; then
  VERSION=" $(cat "${DATA_DIR}/VERSION")"
fi
if [[ -f "${SCRIPT_DIR}/VERSION" ]]; then
  VERSION=" $(cat "${SCRIPT_DIR}/VERSION")"
  cp -f "${SCRIPT_DIR}/VERSION" "${DATA_DIR}/VERSION" 2>/dev/null || true
fi

echo
echo -e "${BOLD}${GREEN}¡Instalación completada!${RESET}${VERSION}"
echo
echo -e "${BOLD}Cómo usar EVO Control:${RESET}"
echo -e "  1. Automático:   el servicio corre al iniciar sesión"
echo -e "  2. Terminal:     ${CYAN}evo${RESET}          (estado: ${CYAN}evo status${RESET})"
echo -e "  3. Menú:         buscá ${CYAN}EVO Control${RESET}"
echo -e "  4. Navegador:    ${CYAN}http://localhost:8080${RESET}"
echo
echo -e "Desinstalar: ${CYAN}systemctl --user disable --now evo-daemon${RESET} y borrá ${BIN_DIR}/evo*"
