//! Tests de persistencia: contrato del puerto, escritura atómica y recuperación.
//!
//! Se trabaja siempre sobre un directorio temporal, nunca sobre la configuración
//! real del usuario.

use std::fs;

use evo_domain::{DeviceProfile, DeviceState, ModelId, PersistedState, RepoError, ServerConfig};
use evo_persist::FileStateRepo;
use evo_ports::StateRepo;

/// Documento de ejemplo con diferencias respecto al estado inicial, para que el
/// *round-trip* tenga algo que probar de verdad.
fn sample() -> PersistedState {
    let profile = DeviceProfile::evo8();
    let mut state = DeviceState::initial(&profile);
    state.channels[0].gain_db = 24.5;
    state.channels[0].muted = true;
    state.channels[1].phantom = true;
    PersistedState::new(profile, state, 1_700_000_000_000)
}

#[test]
fn el_documento_va_y_vuelve_intacto() {
    let dir = tempfile::tempdir().expect("directorio temporal");
    let repo = FileStateRepo::new(dir.path().join("config.toml"));

    let doc = sample();
    repo.save(&doc).expect("guardar");

    let loaded = repo.load().expect("cargar").expect("hay documento");
    assert_eq!(loaded, doc, "el TOML debe preservar el documento entero");
}

#[test]
fn sin_documento_previo_no_hay_error() {
    // El primer arranque no debe tratarse como fallo (SAD §8.5).
    let dir = tempfile::tempdir().expect("directorio temporal");
    let repo = FileStateRepo::new(dir.path().join("no-existe.toml"));
    assert_eq!(repo.load().expect("sin documento"), None);
}

#[test]
fn cada_guardado_deja_la_version_anterior_como_respaldo() {
    let dir = tempfile::tempdir().expect("directorio temporal");
    let path = dir.path().join("config.toml");
    let repo = FileStateRepo::new(&path);

    let mut first = sample();
    first.saved_at_ms = 1;
    repo.save(&first).expect("primero");
    assert!(!path.with_file_name("config.toml.bak").exists());

    let mut second = sample();
    second.saved_at_ms = 2;
    repo.save(&second).expect("segundo");

    let backup = fs::read_to_string(path.with_file_name("config.toml.bak")).expect("respaldo");
    let backup: PersistedState = toml::from_str(&backup).expect("respaldo parseable");
    assert_eq!(backup.saved_at_ms, 1, "el respaldo es la versión anterior");

    let current = repo.load().expect("cargar").expect("actual");
    assert_eq!(current.saved_at_ms, 2);
}

#[test]
fn un_documento_corrupto_se_recupera_desde_el_respaldo() {
    let dir = tempfile::tempdir().expect("directorio temporal");
    let path = dir.path().join("config.toml");
    let repo = FileStateRepo::new(&path);

    repo.save(&sample()).expect("primero");
    let mut second = sample();
    second.saved_at_ms = 99;
    repo.save(&second).expect("segundo");

    // El documento principal se corrompe (disco lleno, corte a mitad de write…).
    fs::write(&path, "esto no es toml {{{").expect("corromper");

    let recovered = repo
        .load()
        .expect("debe recuperarse, no propagar un error")
        .expect("el respaldo sirve");
    assert_eq!(
        recovered.saved_at_ms, 1_700_000_000_000,
        "se recupera la versión del respaldo"
    );
}

#[test]
fn con_principal_y_respaldo_rotos_se_degrada_a_sin_estado() {
    // Preferible arrancar sin estado y poder controlar el hardware que impedir
    // el arranque (SAD §8.5).
    let dir = tempfile::tempdir().expect("directorio temporal");
    let path = dir.path().join("config.toml");
    let repo = FileStateRepo::new(&path);

    repo.save(&sample()).expect("primero");
    repo.save(&sample()).expect("segundo");
    fs::write(&path, "roto").expect("corromper principal");
    fs::write(path.with_file_name("config.toml.bak"), "roto también").expect("corromper respaldo");

    let err = repo.load();
    // Ambos ilegibles: aquí sí se informa, para que quede constancia en el log.
    assert!(matches!(err, Err(RepoError::Corrupt(_))));
}

#[test]
fn un_esquema_mas_nuevo_se_trata_como_corrupto_y_no_se_usa() {
    let dir = tempfile::tempdir().expect("directorio temporal");
    let path = dir.path().join("config.toml");
    let repo = FileStateRepo::new(&path);

    let mut doc = sample();
    doc.schema_version = evo_domain::state::SCHEMA_VERSION + 1;
    // Se escribe a mano porque `save` no valida: interesa simular el archivo que
    // deja una versión futura del programa.
    fs::write(&path, toml::to_string(&doc).expect("serializar")).expect("escribir");

    // Sin respaldo, degrada a "sin estado" en lugar de leer un documento que no
    // entiende.
    assert_eq!(repo.load().expect("degradación limpia"), None);
}

#[test]
fn la_escritura_no_deja_temporales_huerfanos() {
    let dir = tempfile::tempdir().expect("directorio temporal");
    let path = dir.path().join("config.toml");
    let repo = FileStateRepo::new(&path);

    repo.save(&sample()).expect("guardar");

    let leftovers: Vec<_> = fs::read_dir(dir.path())
        .expect("leer directorio")
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| name.contains(".tmp-"))
        .collect();
    assert!(leftovers.is_empty(), "quedaron temporales: {leftovers:?}");
}

#[test]
fn el_guardado_crea_el_directorio_si_no_existe() {
    let dir = tempfile::tempdir().expect("directorio temporal");
    let nested = dir.path().join("evo-suite").join("anidado");
    let repo = FileStateRepo::new(nested.join("config.toml"));

    repo.save(&sample()).expect("crea la ruta");
    assert!(repo.load().expect("cargar").is_some());
}

#[test]
fn el_modelo_se_conserva_por_codigo_estable() {
    // La clave de modelo en TOML es el código estable (`'evo8'`), no un número
    // de variante, para que reordenar el enum no invalide el archivo (SAD §3.2).
    let dir = tempfile::tempdir().expect("directorio temporal");
    let path = dir.path().join("config.toml");
    let repo = FileStateRepo::new(&path);
    repo.save(&sample()).expect("guardar");

    let text = fs::read_to_string(&path).expect("leer");
    assert!(
        text.contains("evo8"),
        "esperaba el código del modelo: {text}"
    );
    assert!(text.contains("schema_version"));
    assert!(text.contains("saved_at_ms"));

    let loaded = repo.load().expect("cargar").expect("documento");
    assert_eq!(loaded.profile.model, ModelId::Evo8);
}

#[test]
fn el_bloque_server_sobrevive_al_round_trip() {
    // El daemon es el único escritor (SAD §8.5). Si el bloque `[server]` no se
    // reemitiera, cada guardado de estado de dispositivo borraría los ajustes
    // del servidor remoto.
    let dir = tempfile::tempdir().expect("directorio temporal");
    let path = dir.path().join("config.toml");
    let repo = FileStateRepo::new(&path);

    let server = ServerConfig {
        enabled: true,
        interface: Some("wlan0".to_string()),
        port: 9000,
    };
    let doc = sample().with_server(server.clone());
    repo.save(&doc).expect("guardar");

    let text = fs::read_to_string(&path).expect("leer");
    assert!(text.contains("[server]"), "esperaba la tabla: {text}");

    let loaded = repo.load().expect("cargar").expect("documento");
    assert_eq!(loaded.server, server, "los ajustes deben volver intactos");
}

#[test]
fn un_documento_sin_server_parsea_con_los_valores_por_defecto() {
    // Compatibilidad hacia atrás: los `config.toml` escritos antes de que
    // existiera `[server]` deben seguir cargando (SAD §8.5).
    let dir = tempfile::tempdir().expect("directorio temporal");
    let path = dir.path().join("config.toml");
    let repo = FileStateRepo::new(&path);

    // Se serializa un documento normal y se elimina su tabla `[server]`, que es
    // exactamente la forma de un archivo de una versión anterior.
    let full = toml::to_string(&sample()).expect("serializar");
    let sin_server = strip_table(&full, "server");
    assert!(
        !sin_server.contains("[server]"),
        "el recorte debe quitar la tabla: {sin_server}"
    );
    fs::write(&path, sin_server).expect("escribir");

    let loaded = repo.load().expect("cargar").expect("documento");
    assert_eq!(
        loaded.server,
        ServerConfig::default(),
        "sin `[server]` se asume el valor por defecto"
    );
}

/// Elimina del TOML la tabla indicada y sus claves, hasta la siguiente tabla.
fn strip_table(toml_text: &str, table: &str) -> String {
    let header = format!("[{table}]");
    let mut out = String::new();
    let mut skipping = false;
    for line in toml_text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('[') {
            skipping = trimmed == header;
        }
        if !skipping {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}
