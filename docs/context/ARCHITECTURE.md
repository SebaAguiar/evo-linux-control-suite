# Documento de Arquitectura de Software (SAD): EVO Linux Control Suite

*Versión 1.0 · 15 de octubre de 2026 · Estado: Borrador para revisión técnica · Autor: Arquitectura / Principal Engineering*

## 0. Resumen Ejecutivo y Restricciones Rígidas

Este documento define la arquitectura de la **EVO Linux Control Suite**: un panel de control de escritorio (Tauri v2 + Rust) para la familia Audient EVO, con un servidor local embebido (HTTP + WebSocket) que sirve un control remoto móvil vía Wi-Fi.

**Restricciones asumidas como inamovibles:**

| Dimensión | Decisión |
| --- | --- |
| Shell de escritorio | Tauri v2 |
| Backend / Core | Rust (tokio, rusb, axum) |
| Persistencia | SQLite (presets, clientes, eventos) + JSON/TOML (configuración) |
| UI/UX | Inspirada en el driver oficial EVO 4: simple, de bajo ruido visual |
| Hardware | EVO 4 (Fase 1), EVO 8 y EVO 16 (Fase 3) |
| Plataforma | Linux (Ubuntu, Fedora, Arch); cliente remoto: navegador móvil |

**Atributos de calidad priorizados (en orden):** (1) Seguridad del hardware y del audio (nunca enviar comandos que dañen equipos conectados), (2) Latencia percibida < 50 ms de extremo a extremo, (3) Consistencia de estado entre hardware, escritorio y móvil, (4) Extensibilidad a nuevos modelos sin tocar el núcleo, (5) Huella mínima de CPU/RAM.

---

## 1. Arquitectura del Sistema y Patrones de Diseño

### 1.1 Patrón Arquitectónico General

Se adopta una **Arquitectura Hexagonal (Puertos y Adaptadores) con núcleo orientado a eventos (Event-Driven) y modelo de actores ligeros sobre tokio**.

Justificación:

- El dominio (estado de la interfaz de audio, reglas de seguridad, sincronización) debe ser independiente de USB, de Tauri y de la red. Esto permite probar el núcleo con un dispositivo simulado y soportar nuevos modelos EVO mediante adaptadores.
- Existen **tres fuentes concurrentes de cambios** (hardware físico, UI de escritorio, clientes remotos). Un bus de eventos con una única fuente de verdad evita condiciones de carrera y divergencia de estado.
- Un enfoque de microservicios no aplica: es una aplicación de escritorio monoproceso. Se obtiene modularidad mediante *crates* de un workspace Cargo, no mediante procesos separados.

### 1.2 Estructura de Workspace (Crates)

```text
evo-suite/
├── crates/
│   ├── evo-domain/        # Núcleo puro: entidades, comandos, eventos, reglas. Sin I/O.
│   ├── evo-ports/         # Traits (puertos): DeviceDriver, StateStore, EventSink
│   ├── evo-usb/           # Adaptador USB (rusb) + drivers por modelo (evo4, evo8, evo16)
│   ├── evo-mock/          # Dispositivo simulado para tests/CI/desarrollo sin hardware
│   ├── evo-core/          # Orquestador: actor de dispositivo, store de estado, bus de eventos
│   ├── evo-remote/        # Adaptador de red: axum (HTTP estático + WebSocket), auth, mDNS
│   ├── evo-persist/       # Adaptador SQLite (rusqlite/sqlx) + configuración TOML/JSON
│   └── evo-ipc/           # Contratos compartidos (serde) entre Tauri y WebSocket
├── apps/
│   ├── desktop/           # Tauri v2 (src-tauri) + frontend SPA
│   └── remote-web/        # Frontend móvil (bundle estático embebido en el binario)
└── udev/70-audient-evo.rules
```

**Regla de dependencias:** `evo-domain` no depende de nadie; `evo-ports` solo de `evo-domain`; los adaptadores dependen de los puertos; `evo-core` depende de puertos (no de adaptadores concretos); la composición se realiza únicamente en `apps/desktop` (composition root).

### 1.3 Vista Lógica por Capas

```text
┌─────────────────────────────────────────────────────────────────────┐
│ PRESENTACIÓN                                                        │
│  Desktop UI (Svelte/Solid en WebView)    Mobile Web UI (navegador)  │
└───────────────┬──────────────────────────────────────┬──────────────┘
                │ Tauri IPC (commands/events)          │ WebSocket (WSS opcional)
┌───────────────▼──────────────────────────────────────▼──────────────┐
│ ADAPTADORES DE ENTRADA (Driving)                                    │
│  tauri_bridge                         evo-remote (axum)             │
└───────────────┬──────────────────────────────────────┬──────────────┘
                │            Command (mpsc)            │
┌───────────────▼──────────────────────────────────────▼──────────────┐
│ NÚCLEO DE APLICACIÓN (evo-core)                                     │
│  CommandBus → Validator/SafetyPolicy → DeviceActor                  │
│  StateStore (single-writer, versionado) → EventBus (broadcast)      │
└───────────────┬──────────────────────────────────────┬──────────────┘
                │ trait EvoDevice                      │ trait StateRepo
┌───────────────▼──────────────┐      ┌────────────────▼─────────────┐
│ ADAPTADORES DE SALIDA        │      │ evo-persist (SQLite + TOML)  │
│  evo-usb (rusb) / evo-mock   │      └──────────────────────────────┘
└───────────────┬──────────────┘
                │ USB control/interrupt/bulk transfers
         ┌──────▼───────┐
         │ Audient EVO  │ (kernel: snd-usb-audio gestiona el audio)
         └──────────────┘
```

### 1.4 Separación de Responsabilidades

| Capa | Responsabilidad | Prohibido |
| --- | --- | --- |
| **Frontend (Desktop y Móvil)** | Renderizado, gestos, visualización de medidores, estado optimista local transitorio | Lógica de negocio, formato de paquetes USB, límites de seguridad |
| **Backend Rust (núcleo)** | Validación, políticas de seguridad, estado canónico, orquestación, sincronización, persistencia | Dependencias directas de UI |
| **Adaptador USB** | Serializar comandos en paquetes específicos del modelo, parsear respuestas y medidores, reconexión | Decisiones de negocio |
| **Sistema operativo** | `snd-usb-audio` maneja el flujo de audio; udev concede permisos; firewall controla el acceso de red | La aplicación no reemplaza ni interfiere con el stream de audio |

**Principio crítico de coexistencia con el kernel:** la aplicación es un *plano de control*; el *plano de datos* de audio permanece en ALSA/PipeWire. El adaptador USB **no debe reclamar (claim) la interfaz de streaming de audio** ni desvincular `snd-usb-audio`; debe operar exclusivamente mediante transferencias de control sobre el endpoint 0 y, de ser necesario, la interfaz de control/interrupt específica. Esto debe validarse en la Fase 1 (ver Pregunta 1).

### 1.5 Abstracción de Dispositivos (Strategy + Registry + Capabilities)

Se refina el trait `EvoDevice` del documento funcional para incorporar el modelo de **capacidades**, evitando una jerarquía rígida por modelo:

```rust
pub trait EvoDevice: Send {
    fn profile(&self) -> &DeviceProfile;          // canales, capacidades, rangos
    fn apply(&mut self, cmd: DeviceCommand) -> Result<Vec<DeviceEvent>, DeviceError>;
    fn poll_meters(&mut self) -> Result<MeterFrame, DeviceError>;
    fn read_state(&mut self) -> Result<DeviceSnapshot, DeviceError>; // resincronización
    fn poll_hw_events(&mut self) -> Result<Vec<DeviceEvent>, DeviceError>; // perillas físicas
}

pub struct DeviceProfile {
    pub model: ModelId,                 // Evo4 | Evo8 | Evo16
    pub inputs: u8,
    pub outputs: u8,
    pub caps: Capabilities,             // bitflags: PHANTOM_PER_CH, SMARTGAIN, LOOPBACK, ADAT, ...
    pub gain_range_db: RangeInclusive<f32>,
}

pub enum DeviceCommand {
    SetGain { ch: u8, db: f32 },
    SetMute { ch: u8, on: bool },
    SetPhantom { ch: u8, on: bool },
    SetMonitorMix { out: u8, level_db: f32, pan: f32 },
    SetLoopback { on: bool },
    TriggerSmartGain,
}
```

- **`DriverRegistry`**: tabla `(VID, PID) → Box<dyn DriverFactory>`. Añadir un modelo = añadir un módulo y una entrada de registro (Principio Abierto/Cerrado).
- **Matriz de capacidades**: la UI y el servidor remoto consultan `Capabilities` para renderizar o rechazar controles, evitando condicionales por modelo en el frontend.
- **Mejora respecto al spec:** `toggle_phantom` se reemplaza por `SetPhantom{on}` (idempotente), eliminando errores por estados desincronizados.

### 1.6 Patrones Complementarios

- **CQRS ligero:** los *Commands* mutan el hardware; las *Queries/Snapshots* leen el estado del store. Los medidores son un canal separado de alta frecuencia que nunca pasa por el flujo de comandos.
- **Actor Pattern:** un `DeviceActor` posee en exclusiva el handle USB; todas las demás partes se comunican por mensajes (`mpsc`). Elimina bloqueos compartidos sobre el dispositivo.
- **Observer/Pub-Sub:** `tokio::sync::broadcast` para eventos de estado; `watch` para medidores (solo importa el último valor).
- **State Machine (typestate):** el ciclo de SmartGain `Idle → Listening → Calibrating → Success | Error` y el ciclo de conexión `Disconnected → Enumerating → Connected → Degraded` se modelan como enumeraciones con transiciones explícitas.

---

## 2. Estrategia de Escalabilidad y Rendimiento

### 2.1 Modelo de Concurrencia

```text
[Thread dedicado USB (bloqueante)] ◄── mpsc<Command> ── [Command Bus tokio]
        │ ▲                                                   ▲   ▲
        │ └── comandos aplicados + eventos hw                 │   │
        ▼                                                     │   │
 [StateStore (single-writer)] ──► broadcast<StateEvent> ──► [Tauri bridge] [WS sessions]
        │
        └──► watch<MeterFrame> (30–60 Hz) ──► [Meter fanout con throttling por cliente]
```

**Decisiones clave:**

1. **`rusb` es bloqueante.** Se aísla en un hilo OS dedicado (`std::thread`), no dentro del runtime async, para impedir que una transferencia lenta bloquee el reactor tokio. La comunicación con tokio se hace con canales.
2. **Single-writer sobre el estado:** solo el `DeviceActor` y el `StateStore` mutan estado; los clientes envían intenciones, no estado.
3. **Backpressure explícito:** colas acotadas. Si una cola de comandos se satura, se aplica *coalescing*, no crecimiento ilimitado.

### 2.2 Mitigación de Cuellos de Botella Específicos

| Cuello de botella | Riesgo | Mitigación |
| --- | --- | --- |
| **Ráfagas de comandos de ganancia** (arrastre de slider = cientos de eventos/s) | Saturar el bus USB (los control transfers tienen latencia de \~1 ms cada uno) | *Coalescing last-write-wins* por clave `(tipo, canal)` con ventana de 8–16 ms; el actor solo envía el valor más reciente |
| **Medidores** | 8 canales × 60 Hz × N clientes | Lectura única del hardware (30–60 Hz) → `watch`; cada cliente recibe a su propia cadencia (móvil: 15–20 Hz); frames binarios compactos (\~1 byte por canal) en lugar de JSON |
| **Renderizado de medidores en UI** | Reflow y GC en el WebView | Dibujo en `<canvas>` con `requestAnimationFrame`; sin re-renderizar el árbol de componentes; actualizaciones solo de buffers tipados |
| **Latencia WebSocket** | Objetivo < 50 ms | `TCP_NODELAY`, sin compresión *permessage-deflate* para mensajes pequeños, mensajes de comando < 100 bytes, sin I/O de disco en la ruta crítica |
| **Escrituras a SQLite** | Bloqueo de la ruta de control | Persistencia *fuera* de la ruta crítica: writer asíncrono con *debounce* (500 ms–2 s); modo WAL; la ruta de control jamás espera al disco |
| **Reconexión/hot-plug** | Estado inconsistente tras desconexión | Hotplug de libusb (con *fallback* a monitor udev); al reconectar: `read_state()` → reconciliar contra el último estado conocido |
| **Muchos clientes móviles** | Memoria por sesión | Límite configurable de sesiones (por defecto 8), *heartbeat* y expulsión de clientes lentos (cola de salida acotada) |

### 2.3 Presupuesto de Latencia (Objetivo p95)

```text
Toque en móvil → WS frame               ~ 5–15 ms (Wi-Fi)
WS parse + validación + coalescing      ~ 1–3 ms
Control transfer USB                    ~ 1–5 ms
Evento de confirmación → broadcast      ~ 1–2 ms
Broadcast → pintado en otros clientes   ~ 5–20 ms
────────────────────────────────────────────────────
Total objetivo                          < 50 ms (p95), < 30 ms (p50)
```

### 2.4 Estrategia de Manejo de Estado

**Modelo: fuente única de verdad en el backend, réplicas de solo lectura con actualización optimista en cliente.**

- **Estado canónico** = `DeviceSnapshot` versionado con un contador monótono `rev: u64`, mantenido por el `StateStore`.
- **Sincronización:** al conectarse, el cliente recibe un `Snapshot{rev}` completo; luego recibe *deltas* `StateEvent{rev, patch}`. Si detecta un salto de `rev`, solicita un `Resync`.
- **UI optimista:** al arrastrar un control, la UI actualiza localmente y marca el control como *pendiente*. Los eventos entrantes del servidor para ese control se ignoran mientras el usuario lo manipula (*interaction lock*) y se aceptan al soltarlo.
- **Resolución de conflictos:** *last-write-wins* a nivel de parámetro, ordenado por el orden de llegada al actor (no por reloj del cliente).
- **Cambios físicos (perillas/botones del hardware):** se leen por *interrupt endpoint* o *polling* (según lo que revele el protocolo) y entran al mismo bus como `DeviceEvent`, garantizando que todos los clientes los vean.
- **Estado efímero vs. persistente:** medidores y estado de conexión son efímeros (solo memoria); presets, configuración y clientes autorizados son persistentes.

### 2.5 Escalabilidad Funcional (Modelos EVO 8 / EVO 16)

- Los canales se modelan como colecciones indexadas (`Vec<ChannelState>`) dimensionadas por `DeviceProfile`, no como campos fijos.
- El enrutamiento se representa como una **matriz dispersa** `(fuente, destino) → ganancia/estado`, que escala de 2×2 a la matriz extendida con ADAT del EVO 16 sin cambiar el esquema.
- La UI renderiza dinámicamente a partir del perfil (lista de canales, pestañas) con virtualización cuando hay más de 8 tiras de canal.
- **Pruebas de contrato:** una suite genérica se ejecuta contra `evo-mock` parametrizado por modelo y contra cada driver real, garantizando que todo adaptador cumple el contrato del trait.

---

## 3. Diseño de Base de Datos y Persistencia

### 3.1 Estrategia Híbrida de Persistencia

| Dato | Almacén | Razón |
| --- | --- | --- |
| Preferencias de aplicación (puerto, bind, tema, idioma) | `config.toml` (o JSON) en `$XDG_CONFIG_HOME/evo-suite/` | Legible/editable por el usuario, sin esquema relacional |
| Presets, escenas, perfiles por dispositivo | SQLite | Relacional, consultable, transaccional |
| Clientes remotos emparejados (tokens) | SQLite | Revocación y auditoría |
| Registro de eventos / diagnóstico | SQLite (con retención) y/o archivo rotativo | Consulta por rango temporal |

Ubicaciones XDG: datos en `$XDG_DATA_HOME/evo-suite/evo.db`, logs en `$XDG_STATE_HOME`.

**Parámetros SQLite:** `journal_mode=WAL`, `synchronous=NORMAL`, `foreign_keys=ON`, `busy_timeout=3000`. Migraciones versionadas (`refinery` o `sqlx migrate`) y `PRAGMA user_version`.

### 3.2 Esquema Relacional Normalizado (DDL)

```sql
-- Catálogo de modelos soportados (poblado desde el DriverRegistry en el arranque)
CREATE TABLE device_profile (
    id             INTEGER PRIMARY KEY,
    vendor_id      INTEGER NOT NULL,            -- 0x2708
    product_id     INTEGER NOT NULL,
    model_code     TEXT    NOT NULL,            -- 'evo4' | 'evo8' | 'evo16'
    display_name   TEXT    NOT NULL,
    input_count    INTEGER NOT NULL CHECK (input_count  > 0),
    output_count   INTEGER NOT NULL CHECK (output_count > 0),
    capabilities   INTEGER NOT NULL DEFAULT 0,  -- bitflags
    UNIQUE (vendor_id, product_id)
);

-- Unidades físicas vistas por la aplicación
CREATE TABLE device (
    id            INTEGER PRIMARY KEY,
    profile_id    INTEGER NOT NULL REFERENCES device_profile(id),
    serial        TEXT    NOT NULL,
    alias         TEXT,
    first_seen_at INTEGER NOT NULL,             -- epoch ms
    last_seen_at  INTEGER NOT NULL,
    UNIQUE (profile_id, serial)
);

-- Preset: instantánea nombrada del estado
CREATE TABLE preset (
    id          INTEGER PRIMARY KEY,
    profile_id  INTEGER NOT NULL REFERENCES device_profile(id),
    device_id   INTEGER REFERENCES device(id) ON DELETE SET NULL, -- NULL = aplicable a todo el modelo
    name        TEXT    NOT NULL,
    is_default  INTEGER NOT NULL DEFAULT 0 CHECK (is_default IN (0,1)),
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL,
    UNIQUE (profile_id, name)
);

CREATE TABLE preset_channel (
    preset_id   INTEGER NOT NULL REFERENCES preset(id) ON DELETE CASCADE,
    channel     INTEGER NOT NULL,
    gain_db     REAL    NOT NULL,
    muted       INTEGER NOT NULL DEFAULT 0,
    phantom     INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (preset_id, channel)
) WITHOUT ROWID;

CREATE TABLE preset_routing (
    preset_id   INTEGER NOT NULL REFERENCES preset(id) ON DELETE CASCADE,
    source      INTEGER NOT NULL,
    dest        INTEGER NOT NULL,
    level_db    REAL    NOT NULL,
    pan         REAL    NOT NULL DEFAULT 0,
    enabled     INTEGER NOT NULL DEFAULT 1,
    PRIMARY KEY (preset_id, source, dest)
) WITHOUT ROWID;

-- Clientes remotos emparejados (nunca se guarda el token en claro)
CREATE TABLE remote_client (
    id            INTEGER PRIMARY KEY,
    label         TEXT    NOT NULL,             -- ej. 'iPhone de Ana'
    token_hash    BLOB    NOT NULL UNIQUE,      -- SHA-256 del token
    role          TEXT    NOT NULL CHECK (role IN ('admin','operator','viewer')),
    created_at    INTEGER NOT NULL,
    last_seen_at  INTEGER,
    last_ip       TEXT,
    revoked_at    INTEGER
);

-- Registro de eventos/auditoría con retención
CREATE TABLE event_log (
    id         INTEGER PRIMARY KEY,
    ts         INTEGER NOT NULL,
    level      INTEGER NOT NULL,                -- 0=trace .. 4=error
    source     TEXT    NOT NULL,                -- 'usb' | 'remote' | 'ui' | 'core'
    kind       TEXT    NOT NULL,                -- 'phantom_on', 'client_paired', 'usb_error'...
    device_id  INTEGER REFERENCES device(id) ON DELETE SET NULL,
    client_id  INTEGER REFERENCES remote_client(id) ON DELETE SET NULL,
    payload    TEXT                              -- JSON compacto
);

-- Último estado conocido para restauración al reconectar
CREATE TABLE device_last_state (
    device_id   INTEGER PRIMARY KEY REFERENCES device(id) ON DELETE CASCADE,
    rev         INTEGER NOT NULL,
    snapshot    TEXT    NOT NULL,               -- JSON del DeviceSnapshot
    updated_at  INTEGER NOT NULL
);
```

### 3.3 Estrategia de Índices

```sql
CREATE INDEX idx_device_last_seen    ON device(last_seen_at DESC);
CREATE INDEX idx_preset_profile_name ON preset(profile_id, name);
CREATE INDEX idx_preset_device       ON preset(device_id) WHERE device_id IS NOT NULL;
CREATE UNIQUE INDEX idx_preset_default_per_profile
    ON preset(profile_id) WHERE is_default = 1;      -- a lo sumo un default por modelo
CREATE INDEX idx_client_active       ON remote_client(token_hash) WHERE revoked_at IS NULL;
CREATE INDEX idx_event_ts            ON event_log(ts DESC);
CREATE INDEX idx_event_kind_ts       ON event_log(kind, ts DESC);
CREATE INDEX idx_event_device_ts     ON event_log(device_id, ts DESC) WHERE device_id IS NOT NULL;
```

**Justificación:**

- Las claves compuestas `WITHOUT ROWID` en `preset_channel` y `preset_routing` evitan un índice adicional y compactan el almacenamiento.
- Los índices parciales (`revoked_at IS NULL`, `is_default = 1`) reducen tamaño y aplican invariantes de negocio a nivel de base de datos.
- `idx_client_active` acelera la autenticación en el *handshake* del WebSocket; además se mantiene un **caché en memoria** de tokens activos para no tocar disco en cada conexión.

### 3.4 Crecimiento de Datos y Mantenimiento

- **Retención de `event_log`:** purga periódica (por defecto 30 días o 50 000 filas, lo que ocurra primero) en una tarea de baja prioridad; los eventos `trace/debug` no se persisten en producción (solo van al log de archivo).
- **`VACUUM` incremental** (`auto_vacuum=INCREMENTAL`) y `PRAGMA optimize` al cierre.
- **Respaldo/exportación:** presets exportables a JSON versionado (`schema_version`) para compartir entre usuarios y distribuciones.
- **Escrituras desacopladas de la ruta crítica** (ver 2.2); `device_last_state` se escribe con *debounce* y al apagado limpio.
- **Migraciones:** hacia adelante, idempotentes, con copia de seguridad automática previa a cada migración mayor.

---

## 4. Seguridad y Flujos de Integración

### 4.1 Modelo de Amenazas (resumen)

| Amenaza | Impacto | Control |
| --- | --- | --- |
| Dispositivo en la misma LAN controla el audio sin autorización | Alto (activar phantom, silenciar una grabación en curso) | Emparejamiento + tokens + roles |
| Comando que daña equipos (phantom con micrófonos de cinta, ganancia extrema) | Crítico (daño físico) | `SafetyPolicy` en el núcleo, no en el cliente |
| Escalada local por udev `MODE="0666"` | Medio | Usar `TAG+="uaccess"` (ver 4.4) |
| XSS/inyección en WebView de escritorio | Alto | CSP estricta, capabilities mínimas de Tauri |
| *Cross-site WebSocket hijacking* desde una web maliciosa en el navegador del móvil | Medio | Validación de `Origin`, token fuera de cookies |
| DoS en LAN (inundación de WS) | Bajo-medio | Límite de tasa, tamaño máximo de mensaje, límite de sesiones |

### 4.2 Autenticación y Autorización del Control Remoto

**Emparejamiento (pairing) mediante QR:**

1. El escritorio genera un **secreto de un solo uso** (128 bits, CSPRNG) con expiración corta (por defecto 120 s) y lo incrusta en la URL del QR: `http://192.168.1.50:8080/pair#s=<secret>` (en el *fragmento*, para que no viaje en logs ni cabeceras).
2. El móvil hace `POST /api/pair` con el secreto y un nombre de dispositivo. El servidor lo consume (uso único) y devuelve un **token de sesión de larga duración** (256 bits).
3. El servidor guarda solo `SHA-256(token)` en `remote_client`. El token se almacena en el móvil (`localStorage`) y se presenta en el *handshake* WS mediante el subprotocolo `Sec-WebSocket-Protocol` o un mensaje `auth` inicial (nunca en la query string).
4. **Confirmación explícita en el escritorio:** cada emparejamiento nuevo requiere aprobar en la UI de escritorio.
5. Revocación inmediata desde la UI: marca `revoked_at` y cierra las sesiones activas.

**Autorización por roles:**

| Rol | Medidores | Ganancia/Mute/Mix | Phantom | SmartGain | Gestión de clientes/presets |
| --- | --- | --- | --- | --- | --- |
| `viewer` | Sí | No | No | No | No |
| `operator` | Sí | Sí | No por defecto (opcional) | Sí | No |
| `admin` | Sí | Sí | Sí (con confirmación) | Sí | Sí (solo desde escritorio) |

La autorización se evalúa **en el servidor por comando**, nunca confiando en el estado de la UI.

### 4.3 Protección de Datos Críticos y Seguridad del Hardware (`SafetyPolicy`)

Implementada en `evo-core`, aplicada a **todos** los orígenes (escritorio, móvil, presets):

- **Validación de rangos:** clamp de ganancia a `profile.gain_range_db`; rechazo de valores `NaN/Inf`.
- **Phantom power (+48 V):** cambio sujeto a *confirmación de doble paso* en remoto y recomendación de silenciar la salida antes de conmutar; opción de **bloqueo global** de phantom desde el escritorio; ventana de enfriamiento (*cooldown*) mínima entre conmutaciones para evitar *pops* y estrés en micrófonos.
- **Restauración de presets:** nunca activa phantom automáticamente al cargar un preset sin confirmación (salvo configuración explícita del admin).
- **SmartGain:** una sola ejecución concurrente (máquina de estados); cancelable; *timeout* de seguridad.
- **Limitación de tasa:** por cliente (ej. 200 msg/s de control) y tamaño máximo de mensaje (4 KB).
- **Datos en reposo:** SQLite no contiene secretos usables (solo hashes). El archivo de configuración tiene permisos `0600`.
- **Sin telemetría:** ningún dato sale de la red local.

### 4.4 Seguridad a Nivel de Sistema Operativo (udev y Red)

**Regla udev recomendada (en lugar de `MODE="0666"`, que otorga acceso a cualquier usuario del sistema):**

```text
# /etc/udev/rules.d/70-audient-evo.rules
SUBSYSTEM=="usb", ATTR{idVendor}=="2708", TAG+="uaccess"
```

`uaccess` concede acceso solo al usuario con sesión activa en el asiento local (systemd-logind). Se documenta `MODE="0660", GROUP="audio"` como alternativa para sistemas sin logind. Se recomienda además **restringir por PID** conocido una vez confirmados los identificadores de EVO 4/8/16, ya que el filtro solo por VID cubre dispositivos futuros no auditados.

**Red:**

- Por defecto el servidor escucha **solo en la interfaz LAN seleccionada** (no `0.0.0.0` si hay varias interfaces); la UI permite elegir interfaz y puerto.
- El servidor remoto está **desactivado por defecto** y se habilita explícitamente (*opt-in*).
- Rechazo de peticiones cuya IP de origen no sea privada (RFC 1918/ULA/link-local) salvo configuración explícita.
- TLS: HTTP plano es aceptable en una LAN doméstica de confianza con el modelo de tokens; se ofrece **TLS opcional** con certificado autofirmado y huella (*fingerprint*) incluida en el QR.
- Descubrimiento opcional vía **mDNS** (`evo-suite.local`).
- Firewall: la aplicación documenta (y opcionalmente asiste) con comandos `ufw` / `firewall-cmd` para abrir únicamente el puerto elegido y solo para la subred local.

**Tauri:** capabilities mínimas (solo los comandos IPC que la UI necesita), CSP estricta (`default-src 'self'`), sin `eval`, y validación de todos los argumentos de comandos IPC con tipos `serde` estrictos.

### 4.5 Diagrama Lógico de Comunicación

```text
 Móvil (navegador)                          UI Escritorio (WebView)
   │  HTTP: GET /, /assets, POST /api/pair      │  Tauri IPC (invoke / emit)
   │  WebSocket: JSON ctrl + binario meters     │
   ▼                                            ▼
 ┌──────────────────────┐              ┌────────────────────┐
 │ evo-remote (axum)    │              │ tauri_bridge       │
 │ auth · roles · rate  │              │                    │
 └──────────┬───────────┘              └─────────┬──────────┘
            └──────────────┬─────────────────────┘
                           ▼  Command (mpsc)
            ┌─────────────────────────────────┐
            │ evo-core                        │
            │ SafetyPolicy → DeviceActor      │
            │ StateStore → EventBus/watch     │
            └───────┬─────────────────┬───────┘
          trait EvoDevice         trait StateRepo
                    ▼                 ▼
            ┌─────────────┐    ┌──────────────┐
            │ evo-usb     │    │ SQLite/TOML  │
            │ (hilo USB)  │    └──────────────┘
            └──────┬──────┘
                   │ USB control / interrupt
            ┌──────▼──────┐
            │ Audient EVO │
            └─────────────┘
```

### 4.6 Contrato de Mensajes (WebSocket / IPC)

Se define un único contrato en `evo-ipc` (tipos `serde`), generando tipos TypeScript con `ts-rs` o `specta` para ambos frontends, de modo que escritorio y móvil compartan exactamente el mismo protocolo.

```jsonc
// Cliente → Servidor
{ "t": "cmd",   "id": 42, "op": "set_gain", "ch": 0, "db": 31.5 }
{ "t": "cmd",   "id": 43, "op": "trigger_smartgain" }
{ "t": "resync" }

// Servidor → Cliente
{ "t": "snapshot", "rev": 1204, "profile": { ... }, "state": { ... } }
{ "t": "patch",    "rev": 1205, "ops": [ { "path": "ch/0/gain", "v": 31.5 } ] }
{ "t": "ack",      "id": 42, "ok": true }
{ "t": "err",      "id": 43, "code": "FORBIDDEN", "msg": "role viewer" }
// Medidores: frame binario [0x01][seq:u16][n:u8][peak_i8 × n]
```

Versionado del protocolo (`proto_version` en el *handshake*) para tolerar que un móvil con una página en caché hable con una versión más nueva del servidor.

---

## 5. Estrategia de Despliegue e Infraestructura

### 5.1 Pipeline CI/CD (GitHub Actions)

```text
PR / push ─► [Lint & Static] ─► [Test] ─► [Build matrix] ─► [Package] ─► [Release (tag)]
```

| Etapa | Contenido |
| --- | --- |
| **Lint & Static** | `cargo fmt --check`, `cargo clippy -D warnings`, ESLint/Prettier + `tsc --noEmit` en ambos frontends |
| **Seguridad de dependencias** | `cargo audit`, `cargo deny` (licencias compatibles con proyecto OSS), `npm audit`, Dependabot/Renovate |
| **Tests** | Unitarios del dominio; **tests de contrato** contra `evo-mock` para cada modelo; tests de integración WS (cliente simulado); *property tests* (`proptest`) sobre `SafetyPolicy` y coalescing; tests de migración de SQLite |
| **Hardware-in-the-loop (opcional)** | *Runner* auto-alojado con un EVO 4 conectado para pruebas de humo semanales (no bloqueante para PRs de terceros) |
| **Build matrix** | `ubuntu-22.04` (x86\_64) y runner ARM64 para portátiles/SBC ARM; dependencias `libwebkit2gtk-4.1`, `libusb-1.0` |
| **Empaquetado** | `.deb`, `.rpm`, **AppImage** (Tauri bundler), **Flatpak** (con `--device=usb` y `--share=network`), y `PKGBUILD` para AUR |
| **Release** | Etiqueta semántica → artefactos firmados (minisign/GPG), `SHA256SUMS`, changelog automático (conventional commits + `git-cliff`), SBOM (`cargo-cyclonedx`) |
| **Actualizaciones** | Tauri updater con firma para AppImage; los demás formatos se actualizan vía gestor de paquetes |

**Consideraciones de empaquetado:**

- La regla udev debe instalarse con los paquetes `.deb/.rpm` (scriptlet `postinst` + `udevadm control --reload`). En AppImage/Flatpak se entrega como archivo aparte con script de instalación documentado, porque un sandbox no puede escribir en `/etc/udev`.
- En Flatpak, el acceso USB sigue requiriendo la regla udev del host.
- El frontend remoto se compila y **se embebe en el binario** (`rust-embed`/`include_dir`), eliminando dependencias de archivos externos en tiempo de ejecución.
- Compilación reproducible (`Cargo.lock` versionado, `--locked`), caché con `Swatinem/rust-cache`.

### 5.2 Estrategia de Ramas y Versionado

Trunk-based con ramas cortas; SemVer; canal `nightly` (AppImage automático desde `main`) y canal `stable`. Las expansiones para EVO 8/16 se liberan tras una *feature flag* de compilación (`--features evo8`) o de ejecución («modo experimental») hasta contar con auditoría de paquetes por la comunidad.

### 5.3 Observabilidad: Logging, Monitoreo y Errores

**Logging estructurado (`tracing` + `tracing-subscriber`):**

- Capas: consola (desarrollo), **archivo rotativo** (`tracing-appender`) en `$XDG_STATE_HOME`, y `journald` opcional.
- Spans por comando (`cmd_id`, `client_id`, `device`), de modo que una acción pueda trazarse de extremo a extremo (WS → actor → USB → ack).
- Nivel configurable en tiempo de ejecución (`RUST_LOG` o UI de Ajustes); por defecto `info`.
- **Sin datos sensibles** en logs (nunca tokens ni secretos de emparejamiento).

**Manejo global de errores:**

```rust
#[derive(thiserror::Error, Debug)]
pub enum AppError {
    #[error("dispositivo no disponible")] DeviceUnavailable,
    #[error("permisos USB insuficientes (revisar udev)")] UsbPermission,
    #[error("transferencia USB fallida: {0}")] UsbTransfer(#[from] rusb::Error),
    #[error("comando rechazado por política de seguridad: {0}")] Safety(String),
    #[error("no autorizado")] Forbidden,
    #[error("error de persistencia: {0}")] Storage(String),
}
```

- Taxonomía de errores con códigos estables que la UI traduce a mensajes accionables (ej. `UsbPermission` → asistente que muestra la regla udev a instalar).
- **Panic hook** global: registra el *backtrace*, intenta apagado ordenado (liberar USB, vaciar la persistencia) y presenta un diálogo con la opción de abrir el reporte.
- **Supervisión de tareas:** el hilo USB y las tareas tokio críticas se reinician con *backoff* exponencial antes de pasar a estado `Degraded`, notificado a todos los clientes.
- **Degradación elegante:** si SQLite falla, la aplicación sigue controlando el hardware; si el servidor remoto falla al enlazar el puerto, el escritorio funciona y se informa del error.

**Monitoreo local (sin telemetría remota):**

- Métricas internas: latencia de comando (histograma), tasa de errores USB, mensajes WS/s, sesiones activas, *lag* de cola.
- Panel de **Diagnóstico** en la UI: estado del dispositivo, versión de firmware, rutas de log y botón **«Exportar paquete de diagnóstico»** (logs recientes + versiones + salida de `lsusb`) para adjuntar a *issues* de GitHub.
- Endpoint `/healthz` local (solo desde loopback) para scripts y *health checks* de systemd.

### 5.4 Modos de Ejecución

- **Interactivo (por defecto):** aplicación Tauri completa.
- **Headless (extensión recomendada):** el núcleo + `evo-remote` como *daemon* de usuario (`systemd --user`), útil cuando el equipo de grabación está en otra sala y no se desea mantener la ventana abierta. La separación por crates lo habilita sin cambios arquitectónicos.

### 5.5 Hoja de Ruta Técnica Alineada a las Fases

| Fase | Entregables de arquitectura |
| --- | --- |
| **1: Core + EVO 4** | Workspace, `evo-domain`, `evo-usb` (EVO 4), actor USB, `StateStore`, `evo-remote` con pairing, medidores, UI de escritorio y móvil, regla udev. Definir los puertos desde el día 1 aunque solo exista una implementación. |
| **2: Abstracción** | Consolidación de `DriverRegistry`, `Capabilities`, tests de contrato, `evo-mock`, presets en SQLite |
| **3: EVO 8/16** | Matriz de enrutamiento extendida, UI dinámica por perfil, canal de auditoría comunitaria de paquetes, banderas experimentales |

---

## 6. Riesgos Técnicos Principales

| Riesgo | Probabilidad | Impacto | Mitigación |
| --- | --- | --- | --- |
| El protocolo de control del EVO no está documentado oficialmente (requiere ingeniería inversa) | Alta | Alta | Capa de adaptador aislada; capturas con `usbmon`/Wireshark; *feature flag* experimental; documentar el protocolo en el repositorio |
| Conflicto con `snd-usb-audio` al acceder al dispositivo | Media | Alta | Operar solo vía endpoint 0; no reclamar interfaces de audio; pruebas con PipeWire activo |
| Cambios de firmware de Audient rompen paquetes | Media | Media | Detección de versión de firmware; modo seguro de solo lectura ante firmware desconocido |
| Latencia Wi-Fi variable | Media | Media | Estado optimista + resync; indicador de calidad de conexión en la UI móvil |
| Dependencia de WebKitGTK entre distribuciones | Media | Media | Fijar versión mínima; AppImage con dependencias empaquetadas; matriz de pruebas multi-distro |

---

## 7. Preguntas Críticas de Refinamiento

1. **Protocolo USB y coexistencia con el kernel:** ¿se dispone ya de documentación o capturas (`usbmon`) del protocolo de control del EVO 4? En particular, ¿los controles (ganancia, phantom, SmartGain) se pueden enviar solo con transferencias de control en el endpoint 0 sin reclamar interfaces, o requieren desvincular `snd-usb-audio`? Esto determina si el audio puede seguir funcionando mientras la aplicación está activa y condiciona todo el diseño del adaptador USB.
2. **Fuente de verdad y lectura de estado del hardware:** ¿el dispositivo permite leer su estado actual y notifica cambios físicos (perilla de ganancia, botones) por un *interrupt endpoint*, o hay que hacer *polling*? ¿Existe un medidor de nivel real disponible por USB, o los medidores deben derivarse del flujo de audio vía PipeWire/ALSA? Esto afecta la frecuencia de polling, el consumo de CPU y la estrategia de sincronización.
3. **Modelo de red y de confianza del control remoto:** ¿se asume siempre una LAN doméstica de confianza, o debe soportarse redes compartidas (estudios, coworking)? ¿Se requieren múltiples clientes simultáneos con roles distintos, y hace falta TLS obligatorio? Define el nivel de seguridad por defecto, la gestión de certificados y la complejidad de la capa de autenticación.
4. **Alcance de persistencia y modo de ejecución:** ¿los presets/escenas y el historial son requisitos de la v1.0 o posteriores? ¿Debe el servidor remoto funcionar con la ventana de escritorio cerrada (modo *daemon* headless)? Esto determina si SQLite entra en la Fase 1 o basta con TOML/JSON, y si el empaquetado debe incluir una unidad `systemd --user`.

---

## 8. Addendum: Decisiones de Refinamiento y Alcance v1.0

*Esta sección incorpora las respuestas del equipo a las preguntas de la sección 7 y **prevalece** sobre cualquier punto contradictorio de las secciones 1 a 6. Lo que se difiere queda como evolución prevista (v1.x), no como descarte de la arquitectura.*

### 8.1 Resumen de Impacto

| Tema | Decisión del equipo | Ajuste arquitectónico |
| --- | --- | --- |
| Protocolo USB | Control por endpoint 0; `snd-usb-audio` sigue gestionando el audio | El adaptador **no reclama interfaces ni desvincula el kernel driver**. Se valida en Fase 1 con PipeWire activo (criterio de aceptación). |
| Fuente de verdad | Estado interno del software; el hardware no notifica cambios físicos | El `StateStore` es la única verdad. Se elimina `poll_hw_events` de la ruta v1.0 (queda opcional tras *feature flag*). |
| Medidores | Derivados del flujo de audio vía ALSA/PipeWire | Nuevo puerto `MeterSource` independiente de `EvoDevice`. Se retira `poll_meters` del trait de dispositivo. |
| Red | LAN doméstica de confianza, HTTP/WS plano, sin roles | Se simplifica la sección 4.2 (ver 8.4). |
| Persistencia | Sin SQLite en v1.0; TOML/JSON | `StateRepo` con implementación de archivo (ver 8.5). |
| Ejecución | Daemon headless indispensable | Se separa `evo-daemon` de la UI Tauri (ver 8.3). |

### 8.2 Medidores desde PipeWire/ALSA

```rust
// evo-ports
pub trait MeterSource: Send {
    fn start(&mut self, profile: &DeviceProfile) -> Result<(), MeterError>;
    fn latest(&self) -> MeterFrame;   // picos dBFS por canal, sin bloqueo
}
```

- Implementación `evo-meter-pipewire` (crate `pipewire` o captura ALSA de solo lectura) que abre un *stream* de monitorización sobre el nodo del EVO y calcula pico por bloque (ventana de \~16–33 ms) con decaimiento (*peak hold*) en el hilo de audio, publicando en un `AtomicU32`/`watch` sin asignaciones.
- **Mapeo de canales:** la correspondencia entre canales PipeWire y canales lógicos del `DeviceProfile` se define por modelo en el driver (tabla estática), no en el núcleo.
- **Riesgos a controlar:** (a) un stream de captura puede alterar la política de recursos/latencia de PipeWire (usar modo pasivo/monitor y no fijar *quantum*); (b) los medidores reflejan la señal *post-preamp digitalizada*, no el estado analógico; (c) si PipeWire no está presente (ALSA puro) se usa el *backend* ALSA o se muestra "medidores no disponibles" sin afectar el control.
- La ruta de distribución a clientes no cambia: `watch<MeterFrame>` → frames binarios con *throttling* por cliente (sección 2.2).

### 8.3 Daemon Headless como Pieza Central

Al ser requisito indispensable, la topología pasa de "app Tauri que contiene el servidor" a **"daemon propietario del hardware + clientes"**:

```text
┌────────────────────────── evo-daemon (systemd --user) ──────────────────────────┐
│ evo-core · evo-usb · MeterSource · evo-persist(TOML) · evo-remote (axum)        │
│   ▲ propietario único del dispositivo USB                                        │
└───┼──────────────────────────────┬───────────────────────────────────────────────┘
    │ WS en 127.0.0.1 / Unix socket │ WS en IP LAN
┌───┴─────────────┐          ┌──────┴──────────┐
│ UI Tauri        │          │ Móvil (navegador)│
│ (cliente fino)  │          └─────────────────┘
└─────────────────┘
```

- **Un solo propietario del handle USB:** el daemon. La UI Tauri es un cliente más (mismo contrato `evo-ipc`), conectado por Unix socket o loopback. Así se evita la disputa por el dispositivo entre dos procesos y la ventana puede cerrarse sin cortar el remoto.
- **Modo embebido (fallback):** si no hay daemon en ejecución, Tauri puede arrancar el núcleo en proceso (misma biblioteca). Se usa un *lock file* en `$XDG_RUNTIME_DIR` para garantizar instancia única.
- **Unidad systemd de usuario:**

```ini
# ~/.config/systemd/user/evo-daemon.service
[Unit]
Description=EVO Control Daemon
After=pipewire.service

[Service]
ExecStart=%h/.local/bin/evo-daemon
Restart=on-failure
RestartSec=2

[Install]
WantedBy=default.target
```

- **Sesión y permisos:** `TAG+="uaccess"` solo concede acceso con sesión activa; el daemon de usuario arranca con la sesión, por lo que es coherente. En arranques sin sesión gráfica (headless real) se requiere la alternativa `GROUP="audio"` y `loginctl enable-linger`.
- **Cambios en CI/CD:** el paquete incluye `evo-daemon`, `evo-ui` y la unidad systemd; el instalador ofrece "habilitar al iniciar sesión" (`systemctl --user enable --now evo-daemon`).

### 8.4 Seguridad Simplificada para v1.0

Se adopta un modelo **sin cuentas ni roles**: cualquier cliente que alcance el servidor tiene acceso completo. Para que sea una decisión consciente y no una brecha, se mantienen estos controles de bajo costo:

- **Servidor remoto opt-in** y enlazado únicamente a la IP de la interfaz LAN elegida (más loopback); nunca `0.0.0.0` por defecto. Como la IP LAN puede cambiar por DHCP, el daemon detecta cambios de interfaz y reenlaza, y el QR se regenera; se recomienda reserva DHCP o mDNS (`evo-suite.local`).
- **Filtro por origen:** rechazo de peticiones de IP no privada (RFC 1918/ULA/link-local) y validación de `Origin` en el WebSocket, que mitiga la manipulación desde páginas web abiertas en el mismo móvil.
- **`SafetyPolicy` sigue activa para todos los orígenes** (rangos de ganancia, *cooldown* y confirmación en la UI antes de conmutar +48 V, límites de tasa y tamaño de mensaje). Al no haber roles, esta es la defensa principal frente a comandos que dañen equipos.
- **Riesgo aceptado y documentado:** cualquier dispositivo de la LAN puede abrir la URL y cambiar parámetros. Se recomienda no usarlo en redes compartidas. Evolución v1.x prevista sin cambios de núcleo: PIN/token simple en el QR (el contrato ya reserva el *handshake* `auth`), y luego roles (secciones 4.2 y 4.3).
- Las tablas `remote_client`, roles y el flujo de emparejamiento quedan **fuera del alcance v1.0**.

### 8.5 Persistencia TOML/JSON (sin SQLite)

Se mantiene el puerto `StateRepo`; la implementación v1.0 es de archivo, de modo que migrar a SQLite en v1.x no toca el núcleo.

```toml
# ~/.config/evo-control/config.toml
schema_version = 1

[server]
enabled = false
interface = "wlan0"
port = 8080

[ui]
theme = "system"

[[presets]]
name = "Voz + guitarra"
model = "evo4"

  [[presets.channels]]
  ch = 0
  gain_db = 32.0
  muted = false
  phantom = false   # nunca se aplica automáticamente sin confirmación
```

- **Escritura atómica:** serializar a archivo temporal en el mismo directorio, `fsync` y `rename`; conservar `config.toml.bak` de la versión anterior.
- **Un único escritor:** el daemon. La UI no escribe el archivo directamente; envía comandos al daemon.
- **Tolerancia a corrupción:** si el parseo falla, se restaura el `.bak` o se arranca con valores por defecto y se registra el error, sin impedir el control del hardware.
- **Estado de dispositivo:** al no haber lectura desde el hardware, se guarda el último estado conocido (con *debounce*) para **reaplicarlo al reconectar**, previa política de seguridad (phantom desactivado hasta confirmar). Se debe informar en la UI de que el estado mostrado es el *último comandado*.
- **Límite práctico:** adecuado para decenas de presets; si el historial/auditoría o el número de presets crece, se activa el adaptador SQLite de las secciones 3.2 y 3.3 mediante migración desde TOML.

### 8.6 Implicaciones del Estado sin Lectura de Hardware

- **Divergencia silenciosa:** si el usuario gira la perilla física, la app no lo sabrá. Mitigaciones: indicador "estado comandado" en la UI, botón **Reaplicar estado** y reaplicación automática al reconectar.
- **Verificación opcional:** ante la posibilidad de que algún registro permita lectura por transferencia de control, `read_state()` se mantiene en el trait con implementación por defecto "no soportado"; se investiga en la Fase 1 con capturas `usbmon`, sin comprometer el diseño.
- **Hot-plug:** al detectar reconexión, se reaplica el último estado comandado de forma escalonada (ganancia y mute primero, phantom solo tras confirmación).

### 8.7 Supuestos a Validar en la Fase 1 (Criterios de Aceptación)

1. Control por endpoint 0 con PipeWire reproduciendo/grabando simultáneamente, sin cortes ni *xruns* atribuibles a la app.
2. Consumo de CPU del daemon en reposo y con medidores activos dentro del presupuesto definido.
3. Latencia móvil → hardware < 50 ms (p95) en una LAN Wi-Fi típica.
4. Medidores PipeWire correctamente mapeados por canal en EVO 4 y sin alterar la latencia configurada del servidor de audio.
5. Reenlace del servidor y regeneración del QR ante cambio de IP.
