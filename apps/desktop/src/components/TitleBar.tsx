import React from "react";
import type { ConnectionStatus } from "../types";
import type { UpdateInfo } from "../hooks/useUpdater";
import type { Theme } from "../hooks/useTheme";

interface TitleBarProps {
  connStatus: ConnectionStatus;
  onOpenQr: () => void;
  onOpenSettings?: () => void;
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
  onOpenSettings,
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

        {onOpenSettings && (
          <button
            className="qr-btn"
            onClick={onOpenSettings}
            title="Configuración (Red / Control Móvil / Acerca de)"
          >
            <svg viewBox="0 0 24 24">
              <path d="M19.14,12.94c0.04-0.3,0.06-0.61,0.06-0.94c0-0.32-0.02-0.64-0.07-0.94l2.03-1.58c0.18-0.14,0.23-0.41,0.12-0.61 l-1.92-3.32c-0.12-0.22-0.37-0.29-0.59-0.22l-2.39,0.96c-0.5-0.38-1.03-0.7-1.62-0.94L14.4,2.81c-0.04-0.24-0.24-0.41-0.48-0.41 h-3.84c-0.24,0-0.43,0.17-0.47,0.41L9.25,5.35C8.66,5.59,8.12,5.92,7.63,6.29L5.24,5.33c-0.22-0.08-0.47,0-0.59,0.22L2.74,8.87 C2.62,9.08,2.66,9.34,2.86,9.48l2.03,1.58C4.84,11.36,4.8,11.69,4.8,12s0.02,0.64,0.07,0.94l-2.03,1.58 c-0.18,0.14-0.23,0.41-0.12,0.61l1.92,3.32c0.12,0.22,0.37,0.29,0.59,0.22l2.39-0.96c0.5,0.38,1.03,0.7,1.62,0.94l0.36,2.54 c0.05,0.24,0.24,0.41,0.48,0.41h3.84c0.24,0,0.44-0.17,0.47-0.41l0.36-2.54c0.59-0.24,1.13-0.56,1.62-0.94l2.39,0.96 c0.22,0.08,0.47,0,0.59-0.22l1.92-3.32c0.12-0.22,0.07-0.47-0.12-0.61L19.14,12.94z M12,15.6c-1.98,0-3.6-1.62-3.6-3.6 s1.62-3.6,3.6-3.6s3.6,1.62,3.6,3.6S13.98,15.6,12,15.6z" />
            </svg>
            <span>Ajustes</span>
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
