#!/usr/bin/env bash
# EVO Linux Control Suite — Script de instalación para usuario
# Instala el binario, el comando `evo`, el servicio de systemd y el acceso directo de escritorio.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"

BIN_DIR="${HOME}/.local/bin"
SYSTEMD_USER_DIR="${HOME}/.config/systemd/user"
APPLICATIONS_DIR="${HOME}/.local/share/applications"
ICONS_DIR="${HOME}/.local/share/icons/hicolor"

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

echo -e "${BOLD}=== Instalador de EVO Linux Control Suite ===${RESET}"
echo

# 0. Asegurar que ~/.cargo/bin esté en el PATH si Rust está instalado
if [[ -f "${HOME}/.cargo/env" ]]; then
  # shellcheck source=/dev/null
  source "${HOME}/.cargo/env"
elif [[ -d "${HOME}/.cargo/bin" && ":$PATH:" != *":${HOME}/.cargo/bin:"* ]]; then
  export PATH="${HOME}/.cargo/bin:${PATH}"
fi

# Detectar ubicación de recursos de empaquetado (servicios, desktop, iconos)
PACKAGING_DIR=""
if [[ -d "${REPO_DIR}/crates/evo-daemon/packaging" ]]; then
  PACKAGING_DIR="${REPO_DIR}/crates/evo-daemon/packaging"
elif [[ -d "${SCRIPT_DIR}/packaging" ]]; then
  PACKAGING_DIR="${SCRIPT_DIR}/packaging"
elif [[ -d "${PWD}/packaging" ]]; then
  PACKAGING_DIR="${PWD}/packaging"
fi

# Detectar script CLI `evo`
EVO_CLI_SRC=""
if [[ -f "${REPO_DIR}/scripts/evo" ]]; then
  EVO_CLI_SRC="${REPO_DIR}/scripts/evo"
elif [[ -f "${SCRIPT_DIR}/evo" ]]; then
  EVO_CLI_SRC="${SCRIPT_DIR}/evo"
elif [[ -f "${PWD}/evo" ]]; then
  EVO_CLI_SRC="${PWD}/evo"
fi

# Función para resolver binario de evo-daemon existente
find_local_daemon_bin() {
  if [[ "${1:-}" != "--rebuild" ]]; then
    if [[ -x "${SCRIPT_DIR}/evo-daemon" ]]; then
      echo "${SCRIPT_DIR}/evo-daemon"
      return 0
    elif [[ -x "${REPO_DIR}/target/release/evo-daemon" ]]; then
      echo "${REPO_DIR}/target/release/evo-daemon"
      return 0
    elif [[ -x "${PWD}/evo-daemon" ]]; then
      echo "${PWD}/evo-daemon"
      return 0
    fi
  fi
  return 1
}

# Función para resolver binario de evo-desktop existente
find_local_desktop_bin() {
  if [[ "${1:-}" != "--rebuild" ]]; then
    if [[ -x "${SCRIPT_DIR}/evo-desktop" ]]; then
      echo "${SCRIPT_DIR}/evo-desktop"
      return 0
    elif [[ -x "${REPO_DIR}/target/release/evo-desktop" ]]; then
      echo "${REPO_DIR}/target/release/evo-desktop"
      return 0
    elif [[ -x "${PWD}/evo-desktop" ]]; then
      echo "${PWD}/evo-desktop"
      return 0
    fi
  fi
  return 1
}

# 1. Resolver o compilar binario de evo-daemon
DAEMON_SRC=""
if DAEMON_SRC="$(find_local_daemon_bin "${1:-}")"; then
  echo -e "${GREEN}✓${RESET} Binario daemon precompilado encontrado (${DAEMON_SRC})."
else
  # Intentar compilar si cargo está disponible y estamos en un repo de Rust
  if command -v cargo >/dev/null 2>&1 && [[ -f "${REPO_DIR}/Cargo.toml" ]]; then
    echo -e "${CYAN}==>${RESET} Compilando evo-daemon en modo release con Cargo..."
    (cd "${REPO_DIR}" && cargo build --release -p evo-daemon)
    DAEMON_SRC="${REPO_DIR}/target/release/evo-daemon"
  else
    # No hay cargo o no hay fuentes de Rust. Intentar descargar release oficial de GitHub
    echo -e "${YELLOW}==>${RESET} No se encontró 'cargo' ni un binario local precompilado."
    echo -e "${CYAN}==>${RESET} Intentando descargar versión precompilada desde GitHub Releases..."

    TMP_DIR="$(mktemp -d -t evo-install-XXXXXX)"
    CLEANUP() { rm -rf "${TMP_DIR}"; }
    trap CLEANUP EXIT

    TARBALL_URL="https://github.com/SebaAguiar/evo-linux-control-suite/releases/latest/download/evo-daemon-linux-x86_64.tar.gz"
    DOWNLOADED=false

    if command -v curl >/dev/null 2>&1; then
      if curl -fsSL -o "${TMP_DIR}/evo-daemon.tar.gz" "${TARBALL_URL}"; then
        DOWNLOADED=true
      fi
    elif command -v wget >/dev/null 2>&1; then
      if wget -q -O "${TMP_DIR}/evo-daemon.tar.gz" "${TARBALL_URL}"; then
        DOWNLOADED=true
      fi
    fi

    if [[ "${DOWNLOADED}" == "true" ]]; then
      tar -xzf "${TMP_DIR}/evo-daemon.tar.gz" -C "${TMP_DIR}"
      if [[ -x "${TMP_DIR}/evo-daemon" ]]; then
        DAEMON_SRC="${TMP_DIR}/evo-daemon"
        if [[ -z "${PACKAGING_DIR}" && -d "${TMP_DIR}/packaging" ]]; then
          PACKAGING_DIR="${TMP_DIR}/packaging"
        fi
        if [[ -z "${EVO_CLI_SRC}" && -f "${TMP_DIR}/evo" ]]; then
          EVO_CLI_SRC="${TMP_DIR}/evo"
        fi
        echo -e "${GREEN}✓${RESET} Binario precompilado descargado exitosamente."
      fi
    fi

    if [[ -z "${DAEMON_SRC}" || ! -x "${DAEMON_SRC}" ]]; then
      echo -e "${RED}Error:${RESET} No se encontró el compilador 'cargo' de Rust ni se pudo descargar el binario precompilado."
      echo
      echo -e "Opciones para continuar en esta máquina:"
      echo -e "  1. Si deseas compilar desde el código fuente, instala Rust:"
      echo -e "       ${CYAN}curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh${RESET}"
      echo -e "       ${CYAN}source \"\$HOME/.cargo/env\"${RESET}"
      echo -e "     Y luego vuelve a ejecutar este instalador."
      echo
      echo -e "  2. O descarga directamente el paquete precompilado de GitHub Releases:"
      echo -e "       ${CYAN}https://github.com/SebaAguiar/evo-linux-control-suite/releases/latest${RESET}"
      exit 1
    fi
  fi
fi

# Resolver o compilar binario de evo-desktop (opcional)
DESKTOP_SRC=""
if DESKTOP_SRC="$(find_local_desktop_bin "${1:-}")"; then
  echo -e "${GREEN}✓${RESET} Binario desktop encontrado (${DESKTOP_SRC})."
else
  if command -v cargo >/dev/null 2>&1 && [[ -d "${REPO_DIR}/apps/desktop" ]]; then
    if command -v bun >/dev/null 2>&1; then
      echo -e "${CYAN}==>${RESET} Compilando evo-desktop en modo release con Tauri..."
      (cd "${REPO_DIR}/apps/desktop" && bun run tauri build --no-bundle --ignore-version-mismatches)
      DESKTOP_SRC="${REPO_DIR}/target/release/evo-desktop"
    elif command -v npm >/dev/null 2>&1; then
      echo -e "${CYAN}==>${RESET} Compilando evo-desktop en modo release con Tauri (npm)..."
      (cd "${REPO_DIR}/apps/desktop" && npm run tauri build -- --no-bundle --ignore-version-mismatches)
      DESKTOP_SRC="${REPO_DIR}/target/release/evo-desktop"
    fi
  fi
fi

# 2. Instalar binario en ~/.local/bin/evo-daemon
echo -e "${CYAN}==>${RESET} Instalando ejecutables en ${BIN_DIR}..."
mkdir -p "${BIN_DIR}"
if systemctl --user is-active --quiet evo-daemon.service 2>/dev/null; then
  echo -e "  Deteniendo servicio previo para reemplazar el binario..."
  systemctl --user stop evo-daemon.service 2>/dev/null || true
fi
install -m 755 "${DAEMON_SRC}" "${BIN_DIR}/evo-daemon"
echo -e "  ${GREEN}✓${RESET} ${BIN_DIR}/evo-daemon"

if [[ -n "${DESKTOP_SRC}" && -f "${DESKTOP_SRC}" ]]; then
  install -m 755 "${DESKTOP_SRC}" "${BIN_DIR}/evo-desktop"
  echo -e "  ${GREEN}✓${RESET} ${BIN_DIR}/evo-desktop"
fi

# 3. Instalar comando CLI `evo` en ~/.local/bin/evo
if [[ -n "${EVO_CLI_SRC}" && -f "${EVO_CLI_SRC}" ]]; then
  cp "${EVO_CLI_SRC}" "${BIN_DIR}/evo"
  chmod +x "${BIN_DIR}/evo"
  mkdir -p "${HOME}/.config/evo-suite"
  if [[ -f "${REPO_DIR}/Cargo.toml" ]]; then
    echo "${REPO_DIR}" > "${HOME}/.config/evo-suite/repo_path"
  fi
  echo -e "  ${GREEN}✓${RESET} ${BIN_DIR}/evo"
fi

# 4. Instalar servicio systemd de usuario
if [[ -n "${PACKAGING_DIR}" && -f "${PACKAGING_DIR}/evo-daemon.service" ]]; then
  echo -e "${CYAN}==>${RESET} Configurando servicio systemd de usuario..."
  mkdir -p "${SYSTEMD_USER_DIR}"
  cp "${PACKAGING_DIR}/evo-daemon.service" "${SYSTEMD_USER_DIR}/evo-daemon.service"
  systemctl --user daemon-reload
  echo -e "  ${GREEN}✓${RESET} ${SYSTEMD_USER_DIR}/evo-daemon.service"
fi

# 5. Instalar iconos
if [[ -n "${PACKAGING_DIR}" && -f "${PACKAGING_DIR}/evo-control.svg" ]]; then
  echo -e "${CYAN}==>${RESET} Instalando iconos..."
  mkdir -p "${ICONS_DIR}/scalable/apps" "${ICONS_DIR}/512x512/apps"
  cp "${PACKAGING_DIR}/evo-control.svg" "${ICONS_DIR}/scalable/apps/evo-control.svg"
  if [[ -f "${PACKAGING_DIR}/evo-control.png" ]]; then
    cp "${PACKAGING_DIR}/evo-control.png" "${ICONS_DIR}/512x512/apps/evo-control.png"
  fi
  echo -e "  ${GREEN}✓${RESET} Iconos instalados en ${ICONS_DIR}"
fi

# 6. Instalar acceso directo .desktop
if [[ -n "${PACKAGING_DIR}" && -f "${PACKAGING_DIR}/evo-control.desktop" ]]; then
  echo -e "${CYAN}==>${RESET} Instalando lanzador de aplicaciones..."
  mkdir -p "${APPLICATIONS_DIR}"
  cp "${PACKAGING_DIR}/evo-control.desktop" "${APPLICATIONS_DIR}/evo-control.desktop"
  if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "${APPLICATIONS_DIR}" >/dev/null 2>&1 || true
  fi
  echo -e "  ${GREEN}✓${RESET} ${APPLICATIONS_DIR}/evo-control.desktop"
fi

# 7. Habilitar servicio systemd
echo
echo -e "${CYAN}==>${RESET} Activando servicio evo-daemon..."
systemctl --user enable --now evo-daemon.service
echo -e "  ${GREEN}✓${RESET} Servicio iniciado y configurado para arranque automático al iniciar sesión."

echo
echo -e "${BOLD}${GREEN}¡Instalación completada con éxito!${RESET}"
echo
echo -e "${BOLD}Formas de usar EVO Control a partir de ahora:${RESET}"
echo -e "  1. ${BOLD}Automático al encender la PC:${RESET} El servicio ya corre en segundo plano."
echo -e "  2. ${BOLD}Desde cualquier terminal:${RESET}     Escribe simplemente ${CYAN}evo${RESET}"
echo -e "  3. ${BOLD}Desde el menú / Spotlight:${RESET}   Busca ${CYAN}EVO Control${RESET} (Mod+Space / Mod+D)"
echo -e "  4. ${BOLD}Desde tu navegador:${RESET}          Abre ${CYAN}http://localhost:8080${RESET}"
echo -e "  5. ${BOLD}Control del servicio:${RESET}        ${CYAN}evo status${RESET} / ${CYAN}evo stop${RESET} / ${CYAN}evo logs${RESET}"
