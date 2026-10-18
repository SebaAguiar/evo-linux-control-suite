import React from "react";
import type { ConnectionStatus } from "../types";

interface TitleBarProps {
  connStatus: ConnectionStatus;
  onOpenQr: () => void;
}

export const TitleBar: React.FC<TitleBarProps> = ({ connStatus, onOpenQr }) => {
  const connClass =
    connStatus === "connected"
      ? "connected"
      : connStatus === "connecting"
      ? "connecting"
      : "";

  const connTitle =
    connStatus === "connected"
      ? "Conectado al Daemon de EVO"
      : connStatus === "connecting"
      ? "Conectando al Daemon..."
      : "Desconectado (iniciando reconexión...)";

  return (
    <header className="evo-titlebar">
      <div className="titlebar-brand">
        <span className="brand-badge">EVO 4</span>
        <span className="brand-subtitle">Control</span>
      </div>
      <div className="titlebar-right">
        <button
          className="qr-btn"
          onClick={onOpenQr}
          title="Escanear QR para abrir en el celular"
        >
          <svg viewBox="0 0 24 24">
            <path d="M3 3h8v8H3V3zm2 2v4h4V5H5zm8-2h8v8h-8V3zm2 2v4h4V5h-4zM3 13h8v8H3v-8zm2 2v4h4v-4H5zm13-2h3v2h-3v-2zm-5 2h3v2h-3v-2zm2 2h3v2h-3v-2zm3 2h3v2h-3v-2zm-5 0h2v2h-2v-2zm2-4h2v2h-2v-2z" />
          </svg>
          <span>Móvil QR</span>
        </button>
        <div className={`conn-status-dot ${connClass}`} title={connTitle} />
      </div>
    </header>
  );
};
