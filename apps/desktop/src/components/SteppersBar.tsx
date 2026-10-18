import React from "react";
import type { ActiveMode, ChannelState, MonitorMixState } from "../types";
import { MIN_DB, MAX_DB } from "../hooks/useEvoSocket";

interface SteppersBarProps {
  activeMode: ActiveMode;
  channels: ChannelState[];
  monitorMix: MonitorMixState;
  onGainChange: (chIdx: number, gainDb: number) => void;
  onMonitorMixChange: (levelDb: number, pan: number) => void;
}

export const SteppersBar: React.FC<SteppersBarProps> = ({
  activeMode,
  channels,
  monitorMix,
  onGainChange,
  onMonitorMixChange,
}) => {
  const isMix = activeMode === "mix";

  const buttons = isMix
    ? [
        { label: "-10%", step: -0.2 },
        { label: "-5%", step: -0.1 },
        { label: "+5%", step: 0.1 },
        { label: "+10%", step: 0.2 },
      ]
    : [
        { label: "-2 dB", step: -2.0 },
        { label: "-0.5", step: -0.5 },
        { label: "+0.5", step: 0.5 },
        { label: "+2 dB", step: 2.0 },
      ];

  const handleStep = (step: number) => {
    if (activeMode === "ch0" || activeMode === "ch1") {
      const chIdx = activeMode === "ch0" ? 0 : 1;
      let newGain = Math.round((channels[chIdx].gain_db + step) * 2) / 2;
      newGain = Math.max(MIN_DB, Math.min(MAX_DB, newGain));
      onGainChange(chIdx, newGain);
    } else if (activeMode === "mix") {
      let newPan = Math.round((monitorMix.pan + step) * 50) / 50;
      newPan = Math.max(-1.0, Math.min(1.0, newPan));
      onMonitorMixChange(monitorMix.level_db, newPan);
    } else if (activeMode === "out") {
      let newVol = Math.round((monitorMix.level_db + step) * 2) / 2;
      newVol = Math.max(-127.0, Math.min(0.0, newVol));
      onMonitorMixChange(newVol, monitorMix.pan);
    }
  };

  return (
    <div className="steppers-bar">
      {buttons.map((b, idx) => (
        <button
          key={idx}
          className="step-btn"
          onClick={() => handleStep(b.step)}
        >
          {b.label}
        </button>
      ))}
    </div>
  );
};
