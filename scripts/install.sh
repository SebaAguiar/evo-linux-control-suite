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
  BOLD="\033[1m"
  RESET="\033[0m"
else
  GREEN=""
  CYAN=""
  YELLOW=""
  BOLD=""
  RESET=""
fi

echo -e "${BOLD}=== Instalador de EVO Linux Control Suite ===${RESET}"
echo

# 1. Compilación del binario release (si no existe o se pide)
cd "${REPO_DIR}"
if [[ ! -f "${REPO_DIR}/target/release/evo-daemon" || "${1:-}" == "--rebuild" ]]; then
  echo -e "${CYAN}==>${RESET} Compilando evo-daemon en modo release..."
  cargo build --release -p evo-daemon
else
  echo -e "${GREEN}✓${RESET} Binario release ya compilado (${REPO_DIR}/target/release/evo-daemon)."
fi

# 2. Instalar binario en ~/.local/bin/evo-daemon
echo -e "${CYAN}==>${RESET} Instalando ejecutables en ${BIN_DIR}..."
mkdir -p "${BIN_DIR}"
if systemctl --user is-active --quiet evo-daemon.service 2>/dev/null; then
  echo -e "  Deteniendo servicio previo para reemplazar el binario..."
  systemctl --user stop evo-daemon.service 2>/dev/null || true
fi
install -m 755 "${REPO_DIR}/target/release/evo-daemon" "${BIN_DIR}/evo-daemon"
echo -e "  ${GREEN}✓${RESET} ${BIN_DIR}/evo-daemon"

# 3. Instalar comando CLI `evo` en ~/.local/bin/evo
cp "${REPO_DIR}/scripts/evo" "${BIN_DIR}/evo"
chmod +x "${BIN_DIR}/evo"
mkdir -p "${HOME}/.config/evo-suite"
echo "${REPO_DIR}" > "${HOME}/.config/evo-suite/repo_path"
echo -e "  ${GREEN}✓${RESET} ${BIN_DIR}/evo"

# 4. Instalar servicio systemd de usuario
echo -e "${CYAN}==>${RESET} Configurando servicio systemd de usuario..."
mkdir -p "${SYSTEMD_USER_DIR}"
cp "${REPO_DIR}/crates/evo-daemon/packaging/evo-daemon.service" "${SYSTEMD_USER_DIR}/evo-daemon.service"
systemctl --user daemon-reload
echo -e "  ${GREEN}✓${RESET} ${SYSTEMD_USER_DIR}/evo-daemon.service"

# 5. Instalar iconos
echo -e "${CYAN}==>${RESET} Instalando iconos..."
mkdir -p "${ICONS_DIR}/scalable/apps" "${ICONS_DIR}/512x512/apps"
cp "${REPO_DIR}/crates/evo-daemon/packaging/evo-control.svg" "${ICONS_DIR}/scalable/apps/evo-control.svg"
cp "${REPO_DIR}/crates/evo-daemon/packaging/evo-control.png" "${ICONS_DIR}/512x512/apps/evo-control.png"
echo -e "  ${GREEN}✓${RESET} Iconos instalados en ${ICONS_DIR}"

# 6. Instalar acceso directo .desktop
echo -e "${CYAN}==>${RESET} Instalando lanzador de aplicaciones..."
mkdir -p "${APPLICATIONS_DIR}"
cp "${REPO_DIR}/crates/evo-daemon/packaging/evo-control.desktop" "${APPLICATIONS_DIR}/evo-control.desktop"
if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database "${APPLICATIONS_DIR}" >/dev/null 2>&1 || true
fi
echo -e "  ${GREEN}✓${RESET} ${APPLICATIONS_DIR}/evo-control.desktop"

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
