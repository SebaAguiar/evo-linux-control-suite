export type ActiveMode = "ch0" | "ch1" | "mix" | "out";

export interface ChannelState {
  gain_db: number;
  muted: boolean;
  phantom: boolean;
}

export interface MonitorMixState {
  level_db: number; // -127.0 dB to 0.0 dB
  pan: number;      // -1.0 (Placa/IN) to +1.0 (PC), 0.0 = 50/50 center
}

export type ConnectionStatus = "connected" | "connecting" | "disconnected";

export type ClientCommand =
  | { op: "set_gain"; ch: number; db: number }
  | { op: "set_mute"; ch: number; on: boolean }
  | { op: "set_phantom"; ch: number; on: boolean }
  | { op: "set_monitor_mix"; out: number; level_db: number; pan: number }
  | { op: "trigger_smartgain"; channels: number[] }
  | { op: "cancel_smartgain" };

export interface ServerInfo {
  loopback_url: string;
  lan_url?: string | null;
  port: number;
  server_enabled?: boolean;
}
