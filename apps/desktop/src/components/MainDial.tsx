import React, { useState, useRef, useEffect, useCallback, useMemo } from "react";
import type { ActiveMode, ChannelState, MonitorMixState } from "../types";
import { MIN_DB, MAX_DB, MIN_OUT_DB, MAX_OUT_DB } from "../hooks/useEvoSocket";

const TOTAL_LEDS = 24;

interface MainDialProps {
  activeMode: ActiveMode;
  channels: ChannelState[];
  monitorMix: MonitorMixState;
  onGainChange: (chIdx: number, gainDb: number) => void;
  onMonitorMixChange: (levelDb: number, pan: number) => void;
  showToast: (text: string, isError?: boolean) => void;
}

export const MainDial: React.FC<MainDialProps> = ({
  activeMode,
  channels,
  monitorMix,
  onGainChange,
  onMonitorMixChange,
  showToast,
}) => {
  const [isEditing, setIsEditing] = useState(false);
  const [isDragging, setIsDragging] = useState(false);
  const [inputValue, setInputValue] = useState("");
  const inputRef = useRef<HTMLInputElement>(null);
  const dialRef = useRef<HTMLDivElement>(null);

  // Active value & labels
  const currentInfo = useMemo(() => {
    if (activeMode === "ch0" || activeMode === "ch1") {
      const chIdx = activeMode === "ch0" ? 0 : 1;
      const ch = channels[chIdx];
      if (!ch) return { text: "0.0", color: "var(--text-knob-num)", unit: "dB", desc: `CANAL ${chIdx + 1} • GAIN`, ratio: 0, isMuted: false };

      const ratio = Math.max(0, Math.min(1, (ch.gain_db - MIN_DB) / (MAX_DB - MIN_DB)));
      if (ch.muted) {
        return { text: "MUTE", color: "var(--red-mute)", unit: "", desc: `CANAL ${chIdx + 1} • MUTE`, ratio, isMuted: true };
      }
      return { text: ch.gain_db.toFixed(1), color: "var(--text-knob-num)", unit: "dB", desc: `CANAL ${chIdx + 1} • GAIN`, ratio, isMuted: false };
    }

    if (activeMode === "mix") {
      const pan = monitorMix.pan;
      const ratio = (pan + 1.0) / 2.0;
      if (Math.abs(pan) < 0.04) {
        return { text: "50/50", color: "var(--color-accent)", unit: "", desc: "PLACA ↔ PC", ratio, isMuted: false };
      }
      if (pan < -0.04) {
        const placaPct = Math.round(((-pan) * 0.5 + 0.5) * 100);
        const pcPct = 100 - placaPct;
        return { text: `${placaPct}%`, color: "var(--text-knob-num)", unit: "IN", desc: `PLACA ${placaPct}% • PC ${pcPct}%`, ratio, isMuted: false };
      }
      const pcPct = Math.round(((pan) * 0.5 + 0.5) * 100);
      const placaPct = 100 - pcPct;
      return { text: `${pcPct}%`, color: "var(--text-knob-num)", unit: "PC", desc: `PLACA ${placaPct}% • PC ${pcPct}%`, ratio, isMuted: false };
    }

    // activeMode === "out"
    const vol = monitorMix.level_db;
    if (vol <= -90.0) {
      return { text: "MUTE", color: "var(--red-mute)", unit: "", desc: "SALIDA SILENCIADA", ratio: 0, isMuted: true };
    }
    const ratio = Math.max(0, Math.min(1, (vol - MIN_OUT_DB) / (MAX_OUT_DB - MIN_OUT_DB)));
    return { text: vol.toFixed(1), color: "var(--text-knob-num)", unit: "dB", desc: "VOLUMEN SALIDA", ratio, isMuted: false };
  }, [activeMode, channels, monitorMix]);

  // Sync displayed value when not editing
  useEffect(() => {
    if (!isEditing) {
      setInputValue(currentInfo.text);
    }
  }, [currentInfo.text, isEditing]);

  // Pre-calculate 24 halo dots
  const haloDots = useMemo(() => {
    const cx = 125;
    const cy = 125;
    const radius = 100;
    const startAngle = 135 * (Math.PI / 180);
    const sweepAngle = 270 * (Math.PI / 180);
    const dots = [];

    for (let i = 0; i < TOTAL_LEDS; i++) {
      const angle = startAngle + (i / (TOTAL_LEDS - 1)) * sweepAngle;
      const x = cx + radius * Math.cos(angle);
      const y = cy + radius * Math.sin(angle);
      dots.push({ x, y });
    }
    return dots;
  }, []);

  const activeLedCount = Math.round(currentInfo.ratio * TOTAL_LEDS);
  const haloColor = currentInfo.isMuted
    ? "var(--red-mute)"
    : activeMode === "mix" || activeMode === "out"
    ? "var(--color-accent)"
    : "var(--led-active)";

  const haloGlow = currentInfo.isMuted
    ? "rgba(255, 59, 48, 0.5)"
    : activeMode === "mix" || activeMode === "out"
    ? "var(--color-accent-glow)"
    : "var(--led-glow)";

  // Drag interaction
  const isDraggingRef = useRef(false);
  const startYRef = useRef(0);
  const startValRef = useRef(0);

  const handlePointerDown = (e: React.PointerEvent) => {
    isDraggingRef.current = true;
    setIsDragging(true);
    startYRef.current = e.clientY;
    if (activeMode === "ch0") startValRef.current = channels[0].gain_db;
    else if (activeMode === "ch1") startValRef.current = channels[1].gain_db;
    else if (activeMode === "mix") startValRef.current = monitorMix.pan;
    else if (activeMode === "out") startValRef.current = monitorMix.level_db;

    if (dialRef.current) {
      dialRef.current.setPointerCapture(e.pointerId);
    }
  };

  const handlePointerMove = (e: React.PointerEvent) => {
    if (!isDraggingRef.current) return;
    const deltaY = startYRef.current - e.clientY;

    if (activeMode === "ch0" || activeMode === "ch1") {
      const chIdx = activeMode === "ch0" ? 0 : 1;
      const deltaDb = (deltaY / 140) * (MAX_DB - MIN_DB);
      let newGain = Math.round((startValRef.current + deltaDb) * 2) / 2;
      newGain = Math.max(MIN_DB, Math.min(MAX_DB, newGain));
      if (newGain !== channels[chIdx].gain_db) {
        onGainChange(chIdx, newGain);
      }
    } else if (activeMode === "mix") {
      const deltaPan = (deltaY / 160) * 2.0;
      let newPan = Math.round((startValRef.current + deltaPan) * 50) / 50;
      newPan = Math.max(-1.0, Math.min(1.0, newPan));
      if (newPan !== monitorMix.pan) {
        onMonitorMixChange(monitorMix.level_db, newPan);
      }
    } else if (activeMode === "out") {
      const deltaDb = (deltaY / 140) * (MAX_OUT_DB - MIN_OUT_DB);
      let newVol = Math.round((startValRef.current + deltaDb) * 2) / 2;
      newVol = Math.max(-127.0, Math.min(0.0, newVol));
      if (newVol !== monitorMix.level_db) {
        onMonitorMixChange(newVol, monitorMix.pan);
      }
    }
  };

  const handlePointerUp = (e: React.PointerEvent) => {
    isDraggingRef.current = false;
    setIsDragging(false);
    if (dialRef.current && dialRef.current.hasPointerCapture(e.pointerId)) {
      dialRef.current.releasePointerCapture(e.pointerId);
    }
  };

  const handleWheel = (e: React.WheelEvent) => {
    e.preventDefault();
    const dir = e.deltaY < 0 ? 1 : -1;

    if (activeMode === "ch0" || activeMode === "ch1") {
      const chIdx = activeMode === "ch0" ? 0 : 1;
      let newGain = Math.round((channels[chIdx].gain_db + dir * 0.5) * 2) / 2;
      newGain = Math.max(MIN_DB, Math.min(MAX_DB, newGain));
      if (newGain !== channels[chIdx].gain_db) {
        onGainChange(chIdx, newGain);
      }
    } else if (activeMode === "mix") {
      let newPan = Math.round((monitorMix.pan + dir * 0.04) * 50) / 50;
      newPan = Math.max(-1.0, Math.min(1.0, newPan));
      if (newPan !== monitorMix.pan) {
        onMonitorMixChange(monitorMix.level_db, newPan);
      }
    } else if (activeMode === "out") {
      let newVol = Math.round((monitorMix.level_db + dir * 0.5) * 2) / 2;
      newVol = Math.max(-127.0, Math.min(0.0, newVol));
      if (newVol !== monitorMix.level_db) {
        onMonitorMixChange(newVol, monitorMix.pan);
      }
    }
  };

  const handleDoubleClick = () => {
    if (activeMode === "mix") {
      onMonitorMixChange(monitorMix.level_db, 0.0);
      showToast("Mix centrado: 50% Placa / 50% PC");
      return;
    }

    // Doble clic para escribir valor en dB (canales o salida)
    setIsEditing(true);
    if (currentInfo.isMuted) {
      if (activeMode === "ch0") setInputValue(channels[0].gain_db.toFixed(1));
      else if (activeMode === "ch1") setInputValue(channels[1].gain_db.toFixed(1));
      else if (activeMode === "out") setInputValue(monitorMix.level_db.toFixed(1));
    } else {
      setInputValue(currentInfo.text);
    }
    requestAnimationFrame(() => {
      inputRef.current?.focus();
      inputRef.current?.select();
    });
  };

  // Direct numeric input commit
  const commitInput = useCallback(() => {
    setIsEditing(false);
    const raw = inputValue.trim().replace(/,/g, ".").replace(/db/gi, "").trim();

    if (activeMode === "ch0" || activeMode === "ch1") {
      const chIdx = activeMode === "ch0" ? 0 : 1;
      const num = parseFloat(raw);
      if (isNaN(num)) {
        showToast("Ingresa un número válido en dB (ej: 34 o 38.5)", true);
        setInputValue(channels[chIdx].gain_db.toFixed(1));
      } else {
        const clamped = Math.max(MIN_DB, Math.min(MAX_DB, num));
        const rounded = Math.round(clamped * 2) / 2;
        onGainChange(chIdx, rounded);
        setInputValue(rounded.toFixed(1));
        if (num > MAX_DB) {
          showToast(`Canal ${chIdx + 1}: Ganancia limitada a ${rounded.toFixed(1)} dB (máx. hardware)`);
        } else {
          showToast(`Canal ${chIdx + 1}: Ganancia fijada en ${rounded.toFixed(1)} dB`);
        }
      }
    } else if (activeMode === "out") {
      const num = parseFloat(raw);
      if (isNaN(num)) {
        showToast("Ingresa un número válido en dB (ej: -12 o 0)", true);
        setInputValue(monitorMix.level_db.toFixed(1));
      } else {
        const clamped = Math.max(-127.0, Math.min(0.0, num));
        const rounded = Math.round(clamped * 2) / 2;
        onMonitorMixChange(rounded, monitorMix.pan);
        setInputValue(rounded.toFixed(1));
        showToast(`Volumen fijado en ${rounded.toFixed(1)} dB`);
      }
    } else if (activeMode === "mix") {
      const num = parseFloat(raw);
      if (!isNaN(num)) {
        let pan = 0.0;
        if (raw.includes("%") || Math.abs(num) > 1.0) {
          if (raw.toLowerCase().includes("in") || raw.toLowerCase().includes("placa")) {
            pan = -Math.min(100, Math.max(0, num)) / 100;
          } else if (raw.toLowerCase().includes("pc")) {
            pan = Math.min(100, Math.max(0, num)) / 100;
          } else {
            pan = (Math.min(100, Math.max(0, num)) - 50) / 50;
          }
        } else {
          pan = Math.max(-1.0, Math.min(1.0, num));
        }
        pan = Math.round(pan * 50) / 50;
        onMonitorMixChange(monitorMix.level_db, pan);
      }
    }
  }, [inputValue, activeMode, channels, monitorMix, onGainChange, onMonitorMixChange, showToast]);

  const handleKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Enter") {
      e.preventDefault();
      commitInput();
      inputRef.current?.blur();
    } else if (e.key === "Escape") {
      e.preventDefault();
      setIsEditing(false);
      setInputValue(currentInfo.text);
      inputRef.current?.blur();
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      const step = e.shiftKey ? 2.0 : 0.5;
      if (activeMode === "ch0" || activeMode === "ch1") {
        const chIdx = activeMode === "ch0" ? 0 : 1;
        const newGain = Math.min(MAX_DB, Math.max(MIN_DB, Math.round((channels[chIdx].gain_db + step) * 2) / 2));
        onGainChange(chIdx, newGain);
        setInputValue(newGain.toFixed(1));
      } else if (activeMode === "out") {
        const newVol = Math.min(0.0, Math.max(-127.0, Math.round((monitorMix.level_db + step) * 2) / 2));
        onMonitorMixChange(newVol, monitorMix.pan);
        setInputValue(newVol.toFixed(1));
      }
      inputRef.current?.select();
    } else if (e.key === "ArrowDown") {
      e.preventDefault();
      const step = e.shiftKey ? -2.0 : -0.5;
      if (activeMode === "ch0" || activeMode === "ch1") {
        const chIdx = activeMode === "ch0" ? 0 : 1;
        const newGain = Math.min(MAX_DB, Math.max(MIN_DB, Math.round((channels[chIdx].gain_db + step) * 2) / 2));
        onGainChange(chIdx, newGain);
        setInputValue(newGain.toFixed(1));
      } else if (activeMode === "out") {
        const newVol = Math.min(0.0, Math.max(-127.0, Math.round((monitorMix.level_db + step) * 2) / 2));
        onMonitorMixChange(newVol, monitorMix.pan);
        setInputValue(newVol.toFixed(1));
      }
      inputRef.current?.select();
    }
  };

  return (
    <div
      className={`main-dial-container ${isDragging ? "dragging" : ""}`}
      id="dial-touch-zone"
      ref={dialRef}
      onPointerDown={handlePointerDown}
      onPointerMove={handlePointerMove}
      onPointerUp={handlePointerUp}
      onWheel={handleWheel}
      onDoubleClick={handleDoubleClick}
    >
      <svg className="halo-svg" viewBox="0 0 250 250">
        {haloDots.map((dot, idx) => {
          const isActive = idx < activeLedCount;
          return (
            <circle
              key={idx}
              cx={dot.x}
              cy={dot.y}
              r={4.2}
              className="halo-dot"
              fill={isActive ? haloColor : "var(--led-inactive)"}
              style={{
                filter: isActive ? `drop-shadow(0 0 4px ${haloGlow})` : "none",
              }}
            />
          );
        })}
      </svg>

      <div className="main-knob">
        <div className="knob-indent">
          {isEditing ? (
            <div className="knob-input-row editing">
              <input
                type="text"
                inputMode="decimal"
                className="knob-val-input"
                ref={inputRef}
                value={inputValue}
                autoFocus
                autoComplete="off"
                spellCheck={false}
                title="Escribe el valor en dB y pulsa Enter"
                aria-label="Valor de ganancia o volumen"
                style={{ color: currentInfo.color }}
                onPointerDown={(e) => e.stopPropagation()}
                onMouseDown={(e) => e.stopPropagation()}
                onTouchStart={(e) => e.stopPropagation()}
                onBlur={commitInput}
                onChange={(e) => setInputValue(e.target.value)}
                onKeyDown={handleKeyDown}
              />
              {currentInfo.unit && (
                <span className="knob-val-unit">{currentInfo.unit}</span>
              )}
            </div>
          ) : (
            <div
              className="knob-val-display"
              title="Arrastra para ajustar • Doble clic para escribir dB"
            >
              <span className="knob-val-num" style={{ color: currentInfo.color }}>
                {currentInfo.text}
              </span>
              {currentInfo.unit && (
                <span className="knob-val-unit">{currentInfo.unit}</span>
              )}
            </div>
          )}

          <div
            className="knob-unit-text"
            style={{ color: isEditing ? "var(--color-accent)" : "var(--color-muted)" }}
          >
            {isEditing ? "ENTER: FIJAR • ESC: SALIR" : currentInfo.desc}
          </div>
        </div>
      </div>
    </div>
  );
};
