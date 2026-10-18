import React, { useState } from "react";
import type { ChannelState } from "../types";
import { MAX_DB } from "../hooks/useEvoSocket";

interface ChannelStripProps {
  channelIdx: 0 | 1;
  channel: ChannelState;
  isActive: boolean;
  onSelect: () => void;
  onToggleMute: (chIdx: number) => void;
  onTogglePhantom: (chIdx: number) => void;
}

export const ChannelStrip: React.FC<ChannelStripProps> = ({
  channelIdx,
  channel,
  isActive,
  onSelect,
  onToggleMute,
  onTogglePhantom,
}) => {
  const [inCooldown, setInCooldown] = useState(false);

  const circ = 125.6;
  const ratio = Math.max(0, Math.min(1, channel.gain_db / MAX_DB));
  const strokeOffset = circ * (1 - ratio);
  const strokeColor = channel.muted ? "var(--red-mute)" : "var(--green-evo)";

  const handlePhantomClick = (e: React.MouseEvent) => {
    e.stopPropagation();
    if (inCooldown) return;
    setInCooldown(true);
    setTimeout(() => {
      setInCooldown(false);
    }, 2050);
    onTogglePhantom(channelIdx);
  };

  const handleMuteClick = (e: React.MouseEvent) => {
    e.stopPropagation();
    onToggleMute(channelIdx);
  };

  return (
    <div
      className={`channel-unit ${isActive ? "active" : ""}`}
      id={`ch-unit-${channelIdx}`}
    >
      <div
        className="channel-badge"
        id={`ch-select-${channelIdx}`}
        onClick={onSelect}
        title={`Seleccionar Canal ${channelIdx + 1}`}
      >
        <div className="mini-dial-wrapper">
          <svg className="mini-dial-svg" viewBox="0 0 48 48">
            <circle className="mini-dial-track" cx="24" cy="24" r="20" />
            <circle
              className="mini-dial-bar"
              id={`mini-bar-${channelIdx}`}
              cx="24"
              cy="24"
              r="20"
              strokeDasharray="125.6"
              style={{
                strokeDashoffset: strokeOffset,
                stroke: strokeColor,
              }}
            />
          </svg>
          <div className="mini-knob" />
        </div>
        <div className="channel-num-label">{channelIdx + 1}</div>
      </div>

      <div className="channel-buttons">
        <div
          className={`round-btn btn-48v ${channel.phantom ? "active" : ""} ${
            inCooldown ? "in-cooldown" : ""
          }`}
          id={`btn-48v-${channelIdx}`}
          onClick={handlePhantomClick}
          title={`Alimentación Phantom 48V Canal ${channelIdx + 1}`}
        >
          48V
        </div>
        <div
          className={`round-btn btn-mute ${channel.muted ? "active" : ""}`}
          id={`btn-mute-${channelIdx}`}
          onClick={handleMuteClick}
          title={`Silenciar Canal ${channelIdx + 1}`}
        >
          M
        </div>
      </div>
    </div>
  );
};
