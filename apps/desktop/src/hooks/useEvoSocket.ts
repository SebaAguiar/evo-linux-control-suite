import { useState, useEffect, useRef, useCallback } from "react";
import type { ChannelState, MonitorMixState, ConnectionStatus, ClientCommand } from "../types";

export const MIN_DB = 0.0;
export const MAX_DB = 56.0;
export const MIN_OUT_DB = -60.0;
export const MAX_OUT_DB = 0.0;

export interface ToastInfo {
  text: string;
  isError: boolean;
}

export function useEvoSocket() {
  const [channels, setChannels] = useState<ChannelState[]>([
    { gain_db: 32.5, muted: false, phantom: false },
    { gain_db: 0.0, muted: false, phantom: false },
  ]);
  const [monitorMix, setMonitorMix] = useState<MonitorMixState>({
    level_db: 0.0,
    pan: 0.0,
  });
  const [smartgainState, setSmartgainState] = useState<string>("idle");
  const [connStatus, setConnStatus] = useState<ConnectionStatus>("disconnected");
  const [toast, setToast] = useState<ToastInfo | null>(null);
  const [meters, setMeters] = useState<number[]>([-128, -128]);

  const wsRef = useRef<WebSocket | null>(null);
  const nextIdRef = useRef<number>(1);
  const currentRevRef = useRef<number>(0);
  const toastTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const showToast = useCallback((text: string, isError = false) => {
    if (toastTimerRef.current) clearTimeout(toastTimerRef.current);
    setToast({ text, isError });
    toastTimerRef.current = setTimeout(() => {
      setToast(null);
    }, 3200);
  }, []);

  const sendCmd = useCallback((cmd: ClientCommand) => {
    const ws = wsRef.current;
    if (!ws || ws.readyState !== WebSocket.OPEN) return;
    const msg = { t: "cmd", id: nextIdRef.current++, ...cmd };
    ws.send(JSON.stringify(msg));
  }, []);

  useEffect(() => {
    let active = true;
    let reconnectTimeout: ReturnType<typeof setTimeout>;

    function connect() {
      if (!active) return;
      setConnStatus("connecting");

      const wsUrl = "ws://127.0.0.1:8080/ws";
      const ws = new WebSocket(wsUrl);
      ws.binaryType = "arraybuffer";
      wsRef.current = ws;

      ws.addEventListener("open", () => {
        if (!active) return;
        setConnStatus("connected");
      });

      ws.addEventListener("message", (event) => {
        if (!active) return;

        // Fotogramas binarios de medidores de alta frecuencia (SAD §4.6)
        if (event.data instanceof ArrayBuffer) {
          const bytes = new Uint8Array(event.data);
          if (bytes[0] === 0x01 && bytes.length >= 4) {
            const len = bytes[3];
            const peaks = new Int8Array(event.data, 4, Math.min(len, bytes.length - 4));
            setMeters(Array.from(peaks));
          }
          return;
        }

        if (typeof event.data !== "string") return;
        try {
          const msg = JSON.parse(event.data);
          if (typeof msg.rev === "number") {
            if (msg.rev < currentRevRef.current) return;
            currentRevRef.current = msg.rev;
          }

          if (msg.t === "hello" || msg.t === "snapshot") {
            if (msg.state) {
              if (Array.isArray(msg.state.inputs)) {
                setChannels(msg.state.inputs.map((ch: any) => ({
                  gain_db: typeof ch.gain_db === "number" ? ch.gain_db : 0.0,
                  muted: !!ch.muted,
                  phantom: !!ch.phantom,
                })));
              }
              if (Array.isArray(msg.state.outputs) && msg.state.outputs[0]?.mix) {
                const m = msg.state.outputs[0].mix;
                setMonitorMix({
                  level_db: typeof m.level_db === "number" ? m.level_db : 0.0,
                  pan: typeof m.pan === "number" ? m.pan : 0.0,
                });
              }
              if (msg.state.smartgain) {
                setSmartgainState(msg.state.smartgain.state || "idle");
              }
            }
          } else if (msg.t === "patch" && Array.isArray(msg.ops)) {
            setChannels((prev) => {
              const next = prev.map((ch) => ({ ...ch }));
              for (const op of msg.ops) {
                if (typeof op.path === "string" && op.path.startsWith("inputs/")) {
                  const parts = op.path.split("/");
                  const chIdx = parseInt(parts[1], 10);
                  const field = parts[2];
                  if (!isNaN(chIdx) && next[chIdx]) {
                    if (field === "gain_db" && typeof op.v === "number") next[chIdx].gain_db = op.v;
                    else if (field === "muted" && typeof op.v === "boolean") next[chIdx].muted = op.v;
                    else if (field === "phantom" && typeof op.v === "boolean") next[chIdx].phantom = op.v;
                  }
                }
              }
              return next;
            });

            for (const op of msg.ops) {
              if (typeof op.path === "string" && op.path.startsWith("outputs/0/mix")) {
                if (op.v && typeof op.v === "object") {
                  setMonitorMix((prev) => ({
                    level_db: typeof op.v.level_db === "number" ? op.v.level_db : prev.level_db,
                    pan: typeof op.v.pan === "number" ? op.v.pan : prev.pan,
                  }));
                }
              } else if (op.path === "smartgain") {
                setSmartgainState(op.v || "idle");
              }
            }
          } else if (msg.t === "err") {
            showToast(msg.msg || "Comando rechazado o en espera de enfriamiento", true);
            if (ws.readyState === WebSocket.OPEN) {
              ws.send(JSON.stringify({ t: "resync" }));
            }
          }
        } catch {
          // Ignorar frames no-JSON o de medidores binarios
        }
      });

      ws.addEventListener("close", () => {
        if (!active) return;
        setConnStatus("disconnected");
        reconnectTimeout = setTimeout(connect, 2000);
      });

      ws.addEventListener("error", () => {
        if (!active) return;
        setConnStatus("disconnected");
      });
    }

    connect();

    return () => {
      active = false;
      clearTimeout(reconnectTimeout);
      if (wsRef.current) wsRef.current.close();
    };
  }, [showToast]);

  return {
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
  };
}
