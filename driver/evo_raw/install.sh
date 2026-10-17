#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
MODULE_NAME="evo_raw"

if [[ $EUID -ne 0 ]]; then
    echo "Este script debe ejecutarse como root (use: sudo ./install.sh o sudo bash driver/evo_raw/install.sh)"
    exit 1
fi

echo "==> Compilando módulo de kernel ${MODULE_NAME}..."
make -C "${SCRIPT_DIR}" all

echo "==> Instalando módulo en el sistema (permanente)..."
make -C "${SCRIPT_DIR}" install

echo "==> Configurando carga automática en el arranque..."
mkdir -p /etc/modules-load.d
echo "${MODULE_NAME}" > "/etc/modules-load.d/${MODULE_NAME}.conf"

echo "==> Instalando reglas udev..."
cp "${SCRIPT_DIR}/99-evo4.rules" /etc/udev/rules.d/
udevadm control --reload-rules
udevadm trigger

echo "==> Cargando módulo ${MODULE_NAME}..."
if lsmod | grep -q "^${MODULE_NAME}"; then
    rmmod "${MODULE_NAME}" || true
fi

modprobe "${MODULE_NAME}"

echo "==> Módulo instalado y cargado permanentemente."
if [[ -e /dev/evo4 ]]; then
    chmod 666 /dev/evo4 2>/dev/null || true
    echo "==> ¡Dispositivo /dev/evo4 creado y listo!"
    ls -l /dev/evo4
fi
