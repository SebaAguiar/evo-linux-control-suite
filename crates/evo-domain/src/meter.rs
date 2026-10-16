//! Medidores: canal de alta frecuencia, independiente del flujo de comandos.
//!
//! El SAD §8.1 **retira** `poll_meters` del trait de dispositivo: los medidores
//! se derivan del flujo de audio vía PipeWire/ALSA mediante el puerto
//! `MeterSource`, y son un canal aparte que nunca pasa por el bus de comandos
//! (SAD §1.6).

use crate::MAX_CHANNELS;
use serde::{Deserialize, Serialize};

/// Fotograma de medidores: pico por canal en dBFS.
///
/// Se dimensiona con un array fijo ([`MAX_CHANNELS`]) para poder leerlo y
/// publicarlo sin asignaciones en el hilo de audio (SAD §8.2). `seq` es un
/// contador monótono que permite al cliente descartar fotogramas viejos.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MeterFrame {
    pub seq: u16,
    /// Picos en dBFS, en rango `[-128, 0]` (0 = fondo de escala).
    peaks_dbfs: [i8; MAX_CHANNELS],
    /// Número de canales válidos desde el inicio del array.
    len: u8,
}

impl MeterFrame {
    /// Fotograma silencioso para un número de canales.
    pub const fn silent(seq: u16, channels: u8) -> Self {
        Self {
            seq,
            peaks_dbfs: [SILENCE_DBFS; MAX_CHANNELS],
            len: channels,
        }
    }

    /// Construye un fotograma cuantizando dBFS a `i8`, con clamping a
    /// `[-128, 0]`. Valores no finitos (`NaN`, `±Inf`) se cuantifican como
    /// silencio (`-128`), que es la lectura segura.
    pub fn from_db_fs(seq: u16, peaks: &[f32]) -> Self {
        let mut frame = Self::silent(seq, peaks.len().min(MAX_CHANNELS) as u8);
        for (slot, db) in frame.peaks_dbfs.iter_mut().zip(peaks.iter()) {
            *slot = quantize_db_fs(*db);
        }
        frame
    }

    pub const fn channels(&self) -> u8 {
        self.len
    }

    /// Pico de un canal como `i8` dBFS, si el índice es válido.
    pub fn peak_raw(&self, ch: u8) -> Option<i8> {
        if ch < self.len {
            Some(self.peaks_dbfs[ch as usize])
        } else {
            None
        }
    }

    /// Pico de un canal en dBFS como `f32`.
    pub fn peak_db_fs(&self, ch: u8) -> Option<f32> {
        self.peak_raw(ch).map(|v| v as f32)
    }

    /// Vista de los canales válidos.
    pub fn peaks(&self) -> &[i8] {
        &self.peaks_dbfs[..self.len as usize]
    }

    /// Codifica el fotograma binario compacto del contrato IPC:
    /// `[0x01][seq:u16 LE][n:u8][peak_i8 × n]` (SAD §4.6).
    ///
    /// ~1 byte por canal en lugar de JSON: es lo que permite servir 60 Hz a
    /// varios clientes sin saturar (SAD §2.2).
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(4 + self.len as usize);
        out.push(METER_FRAME_TAG);
        out.extend_from_slice(&self.seq.to_le_bytes());
        out.push(self.len);
        // `i8` → `u8` conservando los bits: el contrato define el byte crudo.
        out.extend(self.peaks().iter().map(|p| *p as u8));
        out
    }
}

/// Etiqueta del primer byte de un fotograma de medidores binario.
pub const METER_FRAME_TAG: u8 = 0x01;

/// Nivel de silencio en dBFS cuantizado a `i8`.
pub const SILENCE_DBFS: i8 = i8::MIN;

/// Cuantiza dBFS a `i8` en `[-128, 0]`.
pub fn quantize_db_fs(db: f32) -> i8 {
    if !db.is_finite() {
        return SILENCE_DBFS;
    }
    db.clamp(SILENCE_DBFS as f32, 0.0).round() as i8
}
