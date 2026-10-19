import React from "react";
import type { ActiveMode, ChannelState } from "../types";

interface CenterClusterProps {
  activeMode: ActiveMode;
  channels: ChannelState[];
  outputMeterDb?: number;
  onSelectMode: (mode: ActiveMode) => void;
  onToggleMasterMute: () => void;
}

export const CenterCluster: React.FC<CenterClusterProps> = ({
  activeMode,
  channels,
  outputMeterDb,
  onSelectMode,
  onToggleMasterMute,
}) => {
  const allMuted = channels.length > 0 && channels.every((c) => !!c.muted);
  const anyMuted = channels.some((c) => !!c.muted);

  const meterVal = outputMeterDb ?? -128;
  const meterRatio = Math.max(0, Math.min(1, (meterVal + 60) / 60));
  const meterColor = meterVal > -3 ? "#ff3b30" : meterVal > -18 ? "#ffcc00" : "var(--color-accent)";

  return (
    <div className="center-cluster">
      {/* Monitor Mix / Inputs */}
      <button
        className={`center-btn ${activeMode === "mix" ? "active" : ""}`}
        id="btn-mode-mix"
        title="Mezcla Monitor (Balance Placa / PC)"
        onClick={() => onSelectMode("mix")}
      >
        <svg viewBox="0 0 24 24">
          <path d="M4 18h2v-6H4v6zm6 0h2v-12h-2v12zm6 0h2v-9h-2v9z" />
        </svg>
      </button>

      {/* Master Mute */}
      <button
        className={`center-btn ${allMuted ? "active" : ""} ${
          anyMuted && !allMuted ? "partial" : ""
        }`}
        id="btn-master-mute"
        title={allMuted ? "Activar audio en todos los canales" : "Mute General (ambos canales)"}
        onClick={onToggleMasterMute}
      >
        <span style={{ fontWeight: 700, fontSize: "13px" }}>M</span>
      </button>

      {/* Output / Speaker */}
      <button
        className={`center-btn ${activeMode === "out" ? "active" : ""}`}
        id="btn-mode-out"
        title="Volumen General de Salida"
        style={
          meterRatio > 0.04
            ? {
                boxShadow: `0 0 ${Math.round(meterRatio * 12) + 2}px ${meterColor}`,
                borderColor: meterColor,
                transition: "box-shadow 0.05s ease-out, border-color 0.05s ease-out",
              }
            : undefined
        }
        onClick={() => onSelectMode("out")}
      >
        <svg viewBox="0 0 24 24">
          <path d="M3 9v6h4l5 5V4L7 9H3zm13.5 3c0-1.77-1.02-3.29-2.5-4.03v8.05c1.48-.73 2.5-2.25 2.5-4.02zM14 3.23v2.06c2.89.86 5 3.54 5 6.71s-2.11 5.85-5 6.71v2.06c4.01-.91 7-4.49 7-8.77s-2.99-7.86-7-8.77z" />
        </svg>
      </button>
    </div>
  );
};
