//! Estado mutable que la [`SafetyPolicy`] necesita, mantenido por el núcleo.
//!
//! La política del dominio es **pura**: no recuerda nada. Pero el *cooldown* de
//! phantom y el "ya hay un SmartGain en curso" sí son estado. Ese estado lo
//! posee el actor, y vive aquí, en un tipo pequeño y testeable, en lugar de
//! repartido por el bucle del actor.

use evo_domain::{
    CommandRejection, DeviceCommand, DeviceProfile, DeviceState, MAX_CHANNELS, SafetyContext,
    SafetyPolicy,
};
use std::time::Duration;

/// Envoltorio con estado de la [`SafetyPolicy`].
#[derive(Debug)]
pub struct SafetyRuntime {
    policy: SafetyPolicy,
    phantom_locked: bool,
    last_phantom_change_at: [Option<Duration>; MAX_CHANNELS],
    smartgain_running: bool,
}

impl SafetyRuntime {
    pub fn new(policy: SafetyPolicy) -> Self {
        Self {
            policy,
            phantom_locked: false,
            last_phantom_change_at: [None; MAX_CHANNELS],
            smartgain_running: false,
        }
    }

    pub const fn policy(&self) -> &SafetyPolicy {
        &self.policy
    }

    /// Bloqueo global de phantom (SAD §4.3). Lo conmuta el usuario desde el
    /// escritorio y **nunca** lo levanta un cliente remoto por su cuenta.
    pub fn set_phantom_locked(&mut self, locked: bool) {
        self.phantom_locked = locked;
    }

    pub const fn phantom_locked(&self) -> bool {
        self.phantom_locked
    }

    pub fn set_smartgain_running(&mut self, running: bool) {
        self.smartgain_running = running;
    }

    pub const fn smartgain_running(&self) -> bool {
        self.smartgain_running
    }

    /// Contexto actual, para pasar a la política pura.
    pub fn context(&self) -> SafetyContext {
        SafetyContext {
            phantom_locked: self.phantom_locked,
            last_phantom_change_at: self.last_phantom_change_at,
            smartgain_running: self.smartgain_running,
        }
    }

    /// Valida y normaliza un comando con el contexto actual.
    pub fn validate(
        &self,
        cmd: DeviceCommand,
        profile: &DeviceProfile,
        state: &DeviceState,
        now: Duration,
    ) -> Result<DeviceCommand, CommandRejection> {
        self.policy
            .validate(cmd, profile, state, &self.context(), now)
    }

    /// Registra un comando que **ya llegó al hardware**, para que el próximo se
    /// valide contra la realidad y no contra una intención.
    ///
    /// Se llama con el resultado de `apply`, no con el comando enviado: si la
    /// transferencia falló, el estado del hardware no cambió y el cooldown no
    /// debe empezar a contar.
    pub fn note_applied(&mut self, cmd: &DeviceCommand, now: Duration) {
        match *cmd {
            DeviceCommand::SetPhantom { ch, .. } => {
                if let Some(slot) = self.last_phantom_change_at.get_mut(ch as usize) {
                    *slot = Some(now);
                }
            }
            DeviceCommand::TriggerSmartGain => self.smartgain_running = true,
            DeviceCommand::CancelSmartGain => self.smartgain_running = false,
            _ => {}
        }
    }

    /// Informa una fase de SmartGain observada, para no quedar "ocupado" para
    /// siempre si el ciclo termina sin un `Cancel` explícito (SAD §1.6).
    pub fn note_smartgain_phase(&mut self, phase: evo_domain::SmartGainPhase) {
        if phase.is_terminal() {
            self.smartgain_running = false;
        }
    }
}
