import { useState, useCallback, useEffect } from "react";
import type { ActiveMode } from "./types";
import { useEvoSocket } from "./hooks/useEvoSocket";
import { useUpdater } from "./hooks/useUpdater";
import { useTheme } from "./hooks/useTheme";
import { TitleBar } from "./components/TitleBar";
import { MainDial } from "./components/MainDial";
import { SteppersBar } from "./components/SteppersBar";
import { ChannelStrip } from "./components/ChannelStrip";
import { CenterCluster } from "./components/CenterCluster";
import { QrModal } from "./components/QrModal";
import { ToastBanner } from "./components/ToastBanner";
import "./App.css";

export function App() {
  const [activeMode, setActiveMode] = useState<ActiveMode>("ch0");
  const [isQrOpen, setIsQrOpen] = useState(false);

  const { theme, toggleTheme } = useTheme();
  const updater = useUpdater();

  const {
    channels,
    setChannels,
    monitorMix,
    setMonitorMix,
    smartgainState,
    connStatus,
    sendCmd,
    toast,
    showToast,
    meters,
  } = useEvoSocket();

  useEffect(() => {
    if (updater.updateInfo) {
      showToast(
        `Nueva versión v${updater.updateInfo.version} disponible. Pulsa "Actualizar" para instalarla.`,
        false
      );
    }
  }, [updater.updateInfo, showToast]);

  const handleGainChange = useCallback(
    (chIdx: number, newGain: number) => {
      setChannels((prev) => {
        const next = [...prev];
        if (next[chIdx]) {
          next[chIdx] = { ...next[chIdx], gain_db: newGain };
        }
        return next;
      });
      sendCmd({ op: "set_gain", ch: chIdx, db: newGain });
    },
    [sendCmd, setChannels]
  );

  const handleToggleMute = useCallback(
    (chIdx: number) => {
      const current = channels[chIdx]?.muted;
      setChannels((prev) => {
        const next = [...prev];
        if (next[chIdx]) {
          next[chIdx] = { ...next[chIdx], muted: !current };
        }
        return next;
      });
      sendCmd({ op: "set_mute", ch: chIdx, on: !current });
    },
    [channels, sendCmd, setChannels]
  );

  const handleTogglePhantom = useCallback(
    (chIdx: number) => {
      const current = channels[chIdx]?.phantom;
      sendCmd({ op: "set_phantom", ch: chIdx, on: !current });
    },
    [channels, sendCmd]
  );

  const handleToggleMasterMute = useCallback(() => {
    const allMuted = channels.length > 0 && channels.every((c) => !!c.muted);
    const targetState = !allMuted;
    for (let i = 0; i < channels.length; i++) {
      if (channels[i].muted !== targetState) {
        sendCmd({ op: "set_mute", ch: i, on: targetState });
      }
    }
  }, [channels, sendCmd]);

  const handleMonitorMixChange = useCallback(
    (levelDb: number, pan: number) => {
      setMonitorMix({ level_db: levelDb, pan });
      sendCmd({ op: "set_monitor_mix", out: 0, level_db: levelDb, pan });
    },
    [sendCmd, setMonitorMix]
  );

  const handleSmartGainClick = useCallback(() => {
    showToast(
      "SmartGain: 1) Pulsa el botón verde en la placa. 2) Toca el canal (1 o 2) que titila. 3) Pulsa SmartGain de nuevo para calibrar.",
      false
    );
  }, [showToast]);

  return (
    <div className="evo-window" data-theme={theme}>
      <ToastBanner toast={toast} />
      <QrModal isOpen={isQrOpen} onClose={() => setIsQrOpen(false)} />

      <TitleBar
        connStatus={connStatus}
        onOpenQr={() => setIsQrOpen(true)}
        updateInfo={updater.updateInfo}
        downloading={updater.downloading}
        progress={updater.progress}
        onDownloadUpdate={updater.downloadAndInstall}
        theme={theme}
        onToggleTheme={toggleTheme}
      />

      <div className="evo-surface-viewport">
        <div className="evo-deck">
          {/* Left Wing: Inputs */}
          <section className="deck-col deck-inputs">
            <div className="col-header">INPUTS</div>
            <div className="inputs-strips">
              <ChannelStrip
                channelIdx={0}
                channel={channels[0]}
                isActive={activeMode === "ch0"}
                meterDb={meters[0]}
                onSelect={() => setActiveMode("ch0")}
                onToggleMute={handleToggleMute}
                onTogglePhantom={handleTogglePhantom}
              />
              <ChannelStrip
                channelIdx={1}
                channel={channels[1]}
                isActive={activeMode === "ch1"}
                meterDb={meters[1]}
                onSelect={() => setActiveMode("ch1")}
                onToggleMute={handleToggleMute}
                onTogglePhantom={handleTogglePhantom}
              />
            </div>
            <div
              className={`smartgain-pill ${smartgainState !== "idle" ? "busy" : ""}`}
              id="smartgain-btn"
              onClick={handleSmartGainClick}
              title="SmartGain: pulsa el botón verde en la placa, elige el canal y pulsa de nuevo para calibrar"
            >
              <svg viewBox="0 0 24 24">
                <path d="M12 14c1.66 0 3-1.34 3-3V5c0-1.66-1.34-3-3-3S9 3.34 9 5v6c0 1.66 1.34 3 3 3z" />
                <path d="M17 11c0 2.76-2.24 5-5 5s-5-2.24-5-5H5c0 3.53 2.61 6.43 6 6.92V21h2v-3.08c3.39-.49 6-3.39 6-6.92h-2z" />
              </svg>
              <span>SmartGain</span>
            </div>
          </section>

          {/* Center Column: Main Rotary Dial & Steppers */}
          <section className="deck-col deck-center">
            <MainDial
              activeMode={activeMode}
              channels={channels}
              monitorMix={monitorMix}
              meterDb={activeMode === "ch0" ? meters[0] : activeMode === "ch1" ? meters[1] : undefined}
              onGainChange={handleGainChange}
              onMonitorMixChange={handleMonitorMixChange}
              showToast={showToast}
            />

            <SteppersBar
              activeMode={activeMode}
              channels={channels}
              monitorMix={monitorMix}
              onGainChange={handleGainChange}
              onMonitorMixChange={handleMonitorMixChange}
            />
          </section>

          {/* Right Wing: Outputs & Monitoring */}
          <section className="deck-col deck-outputs">
            <div className="col-header">OUTPUTS</div>
            <CenterCluster
              activeMode={activeMode}
              channels={channels}
              onSelectMode={setActiveMode}
              onToggleMasterMute={handleToggleMasterMute}
            />
            <div className="evo-logo" title="Audient EVO">
              <svg viewBox="0 0 32 32">
                <path d="M22 8c-3.1 0-5.3 2.1-6 3.1-.7-1-2.9-3.1-6-3.1C5.6 8 2 11.6 2 16s3.6 8 8 8c3.1 0 5.3-2.1 6-3.1.7 1 2.9 3.1 6 3.1 4.4 0 8-3.6 8-8s-3.6-8-8-8zm-12 12c-2.2 0-4-1.8-4-4s1.8-4 4-4c2.4 0 4.1 2.5 5.2 4-1.1 1.5-2.8 4-5.2 4zm12 0c-2.4 0-4.1-2.5-5.2-4 1.1-1.5 2.8-4 5.2-4 2.2 0 4 1.8 4 4s-1.8 4-4 4z" />
              </svg>
            </div>
          </section>
        </div>
      </div>
    </div>
  );
}

export default App;
