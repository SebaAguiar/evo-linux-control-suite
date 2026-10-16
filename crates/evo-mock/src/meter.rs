//! Fuente de medidores sintética y determinista.

use evo_domain::{DeviceProfile, MeterError, MeterFrame};
use evo_ports::MeterSource;

/// Medidores simulados que generan una señal predecible.
///
/// A diferencia de un valor constante, la señal **varía con el tiempo** (un seno
/// suave en dBFS), para que la UI y el pipeline de medidores se puedan desarrollar
/// y probar de verdad: un medidor que nunca se mueve no valida el renderizado ni
/// el *throttling* por cliente.
///
/// La fase es interna y avanza un paso por llamada a [`MeterSource::latest`], de
/// modo que el resultado es determinista y no depende de un reloj real.
pub struct MockMeterSource {
    started: bool,
    channels: u8,
    step: u32,
    /// Periodo del seno en pasos.
    period: u32,
    floor_db: f32,
    ceiling_db: f32,
}

impl Default for MockMeterSource {
    fn default() -> Self {
        Self::new()
    }
}

impl MockMeterSource {
    pub fn new() -> Self {
        Self {
            started: false,
            channels: 0,
            step: 0,
            period: 64,
            floor_db: -60.0,
            ceiling_db: -6.0,
        }
    }

    /// Ajusta la amplitud de la señal simulada, en dBFS.
    pub fn with_levels(mut self, floor_db: f32, ceiling_db: f32) -> Self {
        self.floor_db = floor_db;
        self.ceiling_db = ceiling_db;
        self
    }

    /// Cuántos pasos ha avanzado la señal.
    pub fn step(&self) -> u32 {
        self.step
    }

    fn peaks_at(&self, step: u32) -> Vec<f32> {
        (0..self.channels)
            .map(|ch| {
                // Desfase por canal para que cada tira tenga su propia lectura.
                let phase =
                    (step as f32 + ch as f32 * 8.0) / self.period as f32 * std::f32::consts::TAU;
                let norm = (phase.sin() + 1.0) / 2.0; // 0..1
                self.floor_db + norm * (self.ceiling_db - self.floor_db)
            })
            .collect()
    }
}

impl MeterSource for MockMeterSource {
    fn start(&mut self, profile: &DeviceProfile) -> Result<(), MeterError> {
        self.channels = profile.inputs;
        self.started = true;
        Ok(())
    }

    fn latest(&self) -> MeterFrame {
        // `latest` es `&self` (nunca bloquea, SAD §8.2), así que no puede avanzar
        // la fase. Se deriva un `seq` del reloj que sí controla el test.
        MeterFrame::from_db_fs(self.step as u16, &self.peaks_at(self.step))
    }

    fn stop(&mut self) -> Result<(), MeterError> {
        self.started = false;
        Ok(())
    }
}

impl MockMeterSource {
    /// Avanza la señal un paso. Pensado para tests y para el bucle del simulador.
    pub fn advance(&mut self) {
        if self.started {
            self.step = self.step.wrapping_add(1);
        }
    }

    /// ¿Está capturando?
    pub fn is_started(&self) -> bool {
        self.started
    }
}
