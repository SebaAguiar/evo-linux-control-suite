import React, { useState, useEffect } from "react";
import type { ServerInfo } from "../types";

interface QrModalProps {
  isOpen: boolean;
  onClose: () => void;
}

export const QrModal: React.FC<QrModalProps> = ({ isOpen, onClose }) => {
  const [targetUrl, setTargetUrl] = useState("http://localhost:8080");
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    if (!isOpen) return;

    let active = true;
    async function fetchInfo() {
      try {
        const res = await fetch("http://127.0.0.1:8080/api/info");
        if (res.ok) {
          const data: ServerInfo = await res.json();
          if (active && data.lan_url) {
            setTargetUrl(data.lan_url);
          }
        }
      } catch {
        // Fallback local
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

  const isLocalhost = targetUrl.includes("localhost") || targetUrl.includes("127.0.0.1");

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

        <p className="qr-instructions" style={{ color: isLocalhost ? "#ff9f0a" : "#aaa" }}>
          {isLocalhost
            ? "Aviso: No se detectó una IP de red Wi-Fi/LAN activa. Conéctate a una red para permitir acceso desde el celular."
            : "Apunta con la cámara de tu celular para abrir EVO Control en la misma red Wi-Fi sin escribir la IP."}
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
