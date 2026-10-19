import React from "react";
import type { ConnectionStatus } from "../types";
import type { UpdateInfo } from "../hooks/useUpdater";
import type { Theme } from "../hooks/useTheme";

interface TitleBarProps {
  connStatus: ConnectionStatus;
  onOpenQr: () => void;
  updateInfo?: UpdateInfo | null;
  downloading?: boolean;
  progress?: number;
  onDownloadUpdate?: () => void;
  theme?: Theme;
  onToggleTheme?: () => void;
}

export const TitleBar: React.FC<TitleBarProps> = ({
  connStatus,
  onOpenQr,
  updateInfo,
  downloading = false,
  progress = 0,
  onDownloadUpdate,
  theme = "brand",
  onToggleTheme,
}) => {
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
        {updateInfo && (
          <button
            className={`update-btn ${downloading ? "downloading" : ""}`}
            onClick={onDownloadUpdate}
            disabled={downloading}
            title={
              downloading
                ? `Descargando actualización... ${progress}%`
                : `Nueva versión ${updateInfo.version} disponible. Haz clic para actualizar.`
            }
          >
            {downloading ? (
              <svg viewBox="0 0 24 24">
                <path d="M12 4V1L8 5l4 4V6c3.31 0 6 2.69 6 6 0 1.01-.25 1.97-.7 2.8l1.46 1.46C19.54 15.03 20 13.57 20 12c0-4.42-3.58-8-8-8zm0 14c-3.31 0-6-2.69-6-6 0-1.01.25-1.97.7-2.8L5.24 7.74C4.46 8.97 4 10.43 4 12c0 4.42 3.58 8 8 8v3l4-4-4-4v3z" />
              </svg>
            ) : (
              <svg viewBox="0 0 24 24">
                <path d="M19.35 10.04C18.67 6.59 15.64 4 12 4 9.11 4 6.6 5.64 5.35 8.04 2.34 8.36 0 10.91 0 14c0 3.31 2.69 6 6 6h13c2.76 0 5-2.24 5-5 0-2.64-2.05-4.78-4.65-4.96zM17 13l-5 5-5-5h3V9h4v4h3z" />
              </svg>
            )}
            <span>
              {downloading
                ? `Actualizando ${progress}%`
                : `Actualizar (v${updateInfo.version})`}
            </span>
          </button>
        )}

        {onToggleTheme && (
          <button
            className={`theme-toggle-btn ${theme === "brand" ? "theme-brand" : "theme-evo"}`}
            onClick={onToggleTheme}
            title={
              theme === "brand"
                ? "Paleta actual: Marca Arsian (Haz clic para cambiar a Oficial EVO)"
                : "Paleta actual: Oficial EVO (Haz clic para cambiar a Marca Arsian)"
            }
          >
            <span className="theme-toggle-dot" />
            <span className="theme-toggle-label">{theme === "brand" ? "Arsian" : "EVO"}</span>
          </button>
        )}

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
