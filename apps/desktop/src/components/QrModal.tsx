import React, { useState, useEffect } from "react";
import type { ServerInfo } from "../types";

interface QrModalProps {
  isOpen: boolean;
  onClose: () => void;
}

type QrStatus = "unknown" | "ok" | "no-lan" | "server-off" | "unreachable";

const STATUS_TEXT: Record<QrStatus, string> = {
  unknown:
    "Apunta con la cámara de tu celular para abrir EVO Control en la misma red Wi-Fi sin escribir la IP.",
  ok: "Apunta con la cámara de tu celular para abrir EVO Control en la misma red Wi-Fi sin escribir la IP.",
  "no-lan":
    "Aviso: No se detectó una IP de red Wi-Fi/LAN activa. Conéctate a una red para permitir acceso desde el celular.",
  "server-off":
    "Aviso: El servidor remoto está desactivado. Poné [server] enabled = true en ~/.config/evo-suite/config.toml y reiniciá el daemon.",
  unreachable:
    "Aviso: No se pudo consultar el daemon en 127.0.0.1:8080. Verificá que evo-daemon esté corriendo.",
};

export const QrModal: React.FC<QrModalProps> = ({ isOpen, onClose }) => {
  const [targetUrl, setTargetUrl] = useState("http://localhost:8080");
  const [status, setStatus] = useState<QrStatus>("unknown");
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    if (!isOpen) return;

    let active = true;
    setStatus("unknown");
    async function fetchInfo() {
      try {
        const res = await fetch("http://127.0.0.1:8080/api/info");
        if (!res.ok) {
          if (active) setStatus("unreachable");
          return;
        }
        const data: ServerInfo = await res.json();
        if (!active) return;
        if (data.lan_url) setTargetUrl(data.lan_url);
        if (!data.lan_url) setStatus("no-lan");
        else if (data.server_enabled === false) setStatus("server-off");
        else setStatus("ok");
      } catch {
        if (active) setStatus("unreachable");
      }
    }
    fetchInfo();

    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => {
      active = false;
      window.removeEventListener("keydown", handleKeyDown);
    };
  }, [isOpen, onClose]);

  if (!isOpen) return null;

  const handleCopy = () => {
    if (navigator.clipboard && navigator.clipboard.writeText) {
      navigator.clipboard.writeText(targetUrl).then(() => {
        setCopied(true);
        setTimeout(() => setCopied(false), 1800);
      });
    }
  };

  const isWarning = status === "no-lan" || status === "server-off" || status === "unreachable";

  return (
    <div className="qr-modal-overlay" onClick={onClose}>
      <div className="qr-modal-card" onClick={(e) => e.stopPropagation()}>
        <div className="qr-modal-header">
          <div className="qr-modal-title">Control Móvil</div>
          <button className="qr-modal-close" onClick={onClose}>
            &times;
          </button>
        </div>

        <div className="qr-code-wrapper">
          <img
            src={`http://127.0.0.1:8080/api/qr.svg?url=${encodeURIComponent(targetUrl)}&t=${Date.now()}`}
            alt="Código QR de control remoto"
          />
        </div>

        <p className="qr-instructions" style={{ color: isWarning ? "#ff9f0a" : "#aaa" }}>
          {STATUS_TEXT[status]}
        </p>

        <div className="qr-url-box">
          <input type="text" className="qr-url-input" readOnly value={targetUrl} />
          <button className="qr-copy-btn" onClick={handleCopy}>
            {copied ? "¡Copiado!" : "Copiar"}
          </button>
        </div>
      </div>
    </div>
  );
};
