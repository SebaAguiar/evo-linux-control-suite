//! Agrupación de ráfagas de comandos (*coalescing*), SAD §2.2.
//!
//! Arrastrar el slider de ganancia en el móvil produce cientos de eventos por
//! segundo; cada control transfer cuesta ~1 ms de bus y el EVO no admite que se
//! le escriba más rápido de lo que responde. La solución del SAD es **agrupar por
//! parámetro y conservar solo el último valor** (*last-write-wins*) dentro de una
//! ventana de 8–16 ms.
//!
//! Dos decisiones que hacen esto testeable:
//!
//! - **El tiempo entra como parámetro** ([`Coalescer::push`] recibe `at`), igual
//!   que en la `SafetyPolicy`. Nada de `Instant::now()` aquí dentro, así que los
//!   tests no dependen de `sleep`s.
//! - **La agrupación es una función pura**: `push` + [`Coalescer::flush`]. El
//!   actor solo decide *cuándo* llamar a `flush`.
//!
//! Los comandos de **efecto de borde** (`TriggerSmartGain`, `SetPhantom`) no
//! tienen clave de coalescing: se aplican uno a uno, en orden, porque perder uno
//! cambiaría el resultado.

use evo_domain::{CoalesceKey, DeviceCommand};
use std::time::Duration;

/// Un comando en espera, con su clave y el instante en que llegó.
#[derive(Debug, Clone, Copy)]
struct Pending {
    cmd: DeviceCommand,
    key: Option<CoalesceKey>,
    at: Duration,
}

/// Ventana de coalescing por defecto.
///
/// 12 ms: dentro de la banda 8–16 ms del SAD §2.2, por encima del periodo de un
/// fotograma a 60 Hz (16.7 ms) no aportaría nada, y más corto empezaría a dejar
/// pasar ráfagas enteras.
pub const DEFAULT_COALESCE_WINDOW: Duration = Duration::from_millis(12);

/// Acumulador de comandos con descarte *last-write-wins*.
#[derive(Debug)]
pub struct Coalescer {
    window: Duration,
    pending: Vec<Pending>,
}

impl Coalescer {
    pub fn new(window: Duration) -> Self {
        Self {
            window,
            pending: Vec::new(),
        }
    }

    pub fn window(&self) -> Duration {
        self.window
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    pub fn len(&self) -> usize {
        self.pending.len()
    }

    /// Encola un comando. `at` es el instante de llegada en un reloj monótono.
    pub fn push(&mut self, cmd: DeviceCommand, at: Duration) {
        self.pending.push(Pending {
            cmd,
            key: cmd.coalesce_key(),
            at,
        });
    }

    /// ¿El comando más antiguo ya superó la ventana?
    ///
    /// El actor lo usa para decidir si debe vaciar aunque siga llegando trabajo.
    pub fn has_due(&self, now: Duration) -> bool {
        self.pending
            .iter()
            .any(|p| now.saturating_sub(p.at) >= self.window)
    }

    /// Cuánto falta para que venza el comando más antiguo.
    ///
    /// `Some(Duration::ZERO)` si ya venció; `None` si no hay nada pendiente.
    pub fn time_until_due(&self, now: Duration) -> Option<Duration> {
        let oldest = self.pending.iter().map(|p| p.at).min()?;
        let elapsed = now.saturating_sub(oldest);
        Some(self.window.saturating_sub(elapsed))
    }

    /// Devuelve los comandos a aplicar, en orden de llegada.
    ///
    /// De cada clave conserva **solo la última aparición**; los de efecto de borde
    /// se conservan todos. El orden relativo se mantiene: si llegan
    /// `gain=1, phantom=true, gain=2`, se aplica `phantom=true, gain=2`.
    pub fn flush(&mut self) -> Vec<DeviceCommand> {
        let pending = std::mem::take(&mut self.pending);

        // Recorriendo desde el final, la primera aparición de cada clave es el
        // último valor: todo lo anterior de esa clave está superado.
        let mut kept_keys: Vec<CoalesceKey> = Vec::with_capacity(pending.len());
        let mut survivor = vec![true; pending.len()];
        for (i, item) in pending.iter().enumerate().rev() {
            if let Some(key) = item.key {
                if kept_keys.contains(&key) {
                    survivor[i] = false;
                } else {
                    kept_keys.push(key);
                }
            }
        }

        pending
            .into_iter()
            .zip(survivor)
            .filter_map(|(item, keep)| keep.then_some(item.cmd))
            .collect()
    }
}
