import React, { useCallback, useEffect, useState } from "react";
import type { UpdateInfo } from "../hooks/useUpdater";

interface SettingsModalProps {
  isOpen: boolean;
  onClose: () => void;
  updateInfo?: UpdateInfo | null;
  downloading?: boolean;
  progress?: number;
  onDownloadUpdate?: () => void;
}

interface ServerConfig {
  enabled: boolean;
  interface?: string | null;
  port: number;
}

interface ConfigInfo {
  server: ServerConfig;
  listening_on?: string | null;
  bind?: string;
  lan_addr?: string | null;
}

interface VersionInfo {
  version: string;
  name: string;
}

export const SettingsModal: React.FC<SettingsModalProps> = ({
  isOpen,
  onClose,
  updateInfo,
  downloading = false,
  progress = 0,
  onDownloadUpdate,
}) => {
  const [tab, setTab] = useState<"network" | "about">("network");
  const [config, setConfig] = useState<ConfigInfo | null>(null);
  const [version, setVersion] = useState<VersionInfo | null>(null);
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const [enabled, setEnabled] = useState(false);
  const [port, setPort] = useState(8080);

  const fetchConfig = useCallback(async () => {
    setLoading(true);
    try {
      const res = await fetch("http://127.0.0.1:8080/api/config");
      if (res.ok) {
        const data = (await res.json()) as ConfigInfo;
        setConfig(data);
        setEnabled(data.server.enabled);
        setPort(data.server.port || 8080);
      }
    } catch (err) {
      console.error(err);
    } finally {
      setLoading(false);
    }
  }, []);

  const fetchVersion = useCallback(async () => {
    try {
      const res = await fetch("http://127.0.0.1:8080/api/version");
      if (res.ok) {
        const data = (await res.json()) as VersionInfo;
        setVersion(data);
      }
    } catch (err) {
      console.error(err);
    }
  }, []);

  useEffect(() => {
    if (!isOpen) return;
    fetchConfig();
    fetchVersion();
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [isOpen, onClose, fetchConfig, fetchVersion]);

  const handleSave = useCallback(async () => {
    setSaving(true);
    setMessage(null);
    try {
      const res = await fetch("http://127.0.0.1:8080/api/config/server", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ enabled, port }),
      });
      if (res.ok) {
        setMessage(
          "Guardado. Reinicia evo-daemon para aplicar los cambios en el servidor remoto."
        );
        fetchConfig();
      } else {
        setMessage("Error al guardar la configuración");
      }
    } catch (err) {
      console.error(err);
      setMessage("Error al guardar la configuración");
    } finally {
      setSaving(false);
    }
  }, [enabled, port, fetchConfig]);

  if (!isOpen) return null;

  const listeningOnlyLoopback = config?.listening_on?.startsWith("127.0.0.1") || config?.lan_addr == null;

  return (
    <div className="qr-modal-overlay" onClick={onClose}>
      <div className="qr-modal-card" style={{ width: 380 }} onClick={(e) => e.stopPropagation()}>
        <div className="qr-modal-header">
          <div className="qr-modal-title">Configuración</div>
          <button className="qr-modal-close" onClick={onClose}>
            &times;
          </button>
        </div>

        <div style={{ display: "flex", gap: 8, marginBottom: 14, width: "100%" }}>
          <button
            className="qr-copy-btn"
            style={{
              flex: 1,
              padding: "6px 10px",
              background: tab === "network" ? "var(--color-accent)" : "var(--step-btn-bg)",
              color: tab === "network" ? "#111827" : "var(--step-btn-color)",
            }}
            onClick={() => setTab("network")}
          >
            Red / Control Móvil
          </button>
          <button
            className="qr-copy-btn"
            style={{
              flex: 1,
              padding: "6px 10px",
              background: tab === "about" ? "var(--color-accent)" : "var(--step-btn-bg)",
              color: tab === "about" ? "#111827" : "var(--step-btn-color)",
            }}
            onClick={() => setTab("about")}
          >
            Acerca de
          </button>
        </div>

        {tab === "network" && (
          <div style={{ width: "100%", textAlign: "left", display: "flex", flexDirection: "column", gap: 12 }}>
            <div style={{ fontSize: 11, color: "var(--color-muted)", lineHeight: 1.4 }}>
              Estado actual:{" "}
              {listeningOnlyLoopback ? (
                <span style={{ color: "#ff9f0a" }}>Solo escucha en loopback (127.0.0.1)</span>
              ) : (
                <span style={{ color: "var(--color-accent)" }}>
                  Escuchando en red local{config?.lan_addr ? ` (${config.lan_addr})` : ""}
                </span>
              )}
            </div>

            <label style={{ display: "flex", alignItems: "center", gap: 10, fontSize: 12 }}>
              <input
                type="checkbox"
                checked={enabled}
                onChange={(e) => setEnabled(e.target.checked)}
              />
              Permitir acceso desde otros dispositivos en esta red Wi-Fi/LAN
            </label>

            <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
              <span style={{ fontSize: 12, minWidth: 60 }}>Puerto</span>
              <input
                type="number"
                value={port}
                onChange={(e) => setPort(Number(e.target.value) || 8080)}
                className="qr-url-input"
                style={{ width: 100, textAlign: "left" }}
              />
            </div>

            <button
              className="qr-copy-btn"
              onClick={handleSave}
              disabled={saving || loading}
              style={{ alignSelf: "flex-start", padding: "8px 14px" }}
            >
              {saving ? "Guardando..." : "Guardar y aplicar"}
            </button>

            {message && (
              <div style={{ fontSize: 11, color: "var(--color-muted)", lineHeight: 1.4 }}>
                {message}
              </div>
            )}
          </div>
        )}

        {tab === "about" && (
          <div style={{ width: "100%", textAlign: "left", display: "flex", flexDirection: "column", gap: 12 }}>
            <div style={{ fontSize: 12 }}>
              <div style={{ fontWeight: 600 }}>EVO Linux Control Suite</div>
              <div style={{ color: "var(--color-muted)", marginTop: 4 }}>
                Versión actual: {version?.version || "Desconocida"}
              </div>
            </div>

            {updateInfo ? (
              <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
                <div style={{ fontSize: 11, color: "var(--color-accent)" }}>
                  Nueva versión disponible: v{updateInfo.version}
                </div>
                <button
                  className="qr-copy-btn"
                  onClick={onDownloadUpdate}
                  disabled={downloading}
                  style={{ alignSelf: "flex-start", padding: "8px 14px" }}
                >
                  {downloading ? `Actualizando ${progress}%` : "Actualizar"}
                </button>
              </div>
            ) : (
              <div style={{ fontSize: 11, color: "var(--color-muted)" }}>
                Estás usando la última versión
              </div>
            )}
          </div>
        )}
      </div>
    </div>
  );
};
