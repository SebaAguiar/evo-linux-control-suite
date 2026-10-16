//! Filtros de origen del control remoto (SAD §8.4).
//!
//! En v1.0 no hay cuentas ni roles: cualquier cliente que alcanza el servidor
//! tiene acceso completo. Como eso debe ser una decisión consciente y no una
//! brecha, se mantienen dos controles de bajo costo:
//!
//! - **Filtro de IP de origen:** se rechazan peticiones cuya IP no sea privada
//!   (RFC 1918 / ULA / link-local), salvo que se relaje explícitamente.
//! - **Validación de `Origin`:** una página web abierta en el mismo móvil podría
//!   intentar abrir el WebSocket; exigir que el `Origin` apunte al propio host o a
//!   una IP privada lo evita.

use std::net::IpAddr;

/// ¿La IP de origen es aceptable para el control remoto?
///
/// Acepta loopback, rangos privados (RFC 1918), link-local y ULA IPv6. Con
/// `allow_public` se acepta cualquier origen (solo para desarrollo o redes de
/// confianza explícitas; **nunca** es el valor por defecto, SAD §8.4).
pub fn is_allowed_peer(ip: IpAddr, allow_public: bool) -> bool {
    if allow_public {
        return true;
    }
    match ip {
        IpAddr::V4(v4) => v4.is_loopback() || v4.is_private() || v4.is_link_local(),
        IpAddr::V6(v6) => {
            // IPv4 mapeada (::ffff:a.b.c.d) se evalúa como su IPv4.
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_allowed_peer(IpAddr::V4(v4), false);
            }
            v6.is_loopback() || v6.is_unique_local() || v6.is_unicast_link_local()
        }
    }
}

/// Política de validación de la cabecera `Origin`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum OriginPolicy {
    /// Acepta si no hay `Origin` (clientes nativos) o si su host es el propio
    /// host local, loopback o una IP privada. Es el valor por defecto.
    #[default]
    SameHost,
    /// Acepta cualquier `Origin`. Desactiva la mitigación; solo para desarrollo.
    AllowAny,
}

/// Valida la cabecera `Origin` según la política.
///
/// `allowed` es una lista explícita de orígenes completos (p. ej.
/// `"http://evo-suite.local:8080"`) para nombres mDNS que no son IPs.
pub fn is_allowed_origin(origin: Option<&str>, policy: OriginPolicy, allowed: &[String]) -> bool {
    match policy {
        OriginPolicy::AllowAny => true,
        OriginPolicy::SameHost => match origin {
            // Sin `Origin`: cliente nativo (Tauri, `wscat`). Lo acepta el filtro
            // de IP de origen, que es la defensa principal.
            None => true,
            Some(o) => {
                if allowed.iter().any(|a| a.eq_ignore_ascii_case(o)) {
                    return true;
                }
                match origin_host(o) {
                    Some(host) => {
                        host.eq_ignore_ascii_case("localhost")
                            || host
                                .parse::<IpAddr>()
                                .map(|ip| is_allowed_peer(ip, false))
                                .unwrap_or(false)
                    }
                    None => false,
                }
            }
        },
    }
}

/// Extrae el host de un `Origin` (`esquema://host[:puerto]`), sin asignar.
fn origin_host(origin: &str) -> Option<&str> {
    let after_scheme = origin.split_once("://").map_or(origin, |(_, rest)| rest);
    let authority = after_scheme.split(['/', '?', '#']).next()?;
    if authority.is_empty() {
        return None;
    }
    // IPv6 en corchetes: "[::1]:8080" → "::1".
    if let Some(rest) = authority.strip_prefix('[') {
        return rest.split(']').next();
    }
    authority.split(':').next()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};

    #[test]
    fn acepta_los_rangos_privados_y_loopback() {
        assert!(is_allowed_peer(IpAddr::V4(Ipv4Addr::LOCALHOST), false));
        assert!(is_allowed_peer("192.168.1.10".parse().unwrap(), false));
        assert!(is_allowed_peer("10.0.0.5".parse().unwrap(), false));
        assert!(is_allowed_peer("172.16.4.4".parse().unwrap(), false));
        assert!(is_allowed_peer("169.254.1.1".parse().unwrap(), false));
        assert!(is_allowed_peer(IpAddr::V6(Ipv6Addr::LOCALHOST), false));
        assert!(is_allowed_peer("fd00::1".parse().unwrap(), false));
    }

    #[test]
    fn rechaza_ips_publicas_salvo_relajacion_explicita() {
        assert!(!is_allowed_peer("8.8.8.8".parse().unwrap(), false));
        assert!(!is_allowed_peer("2001:4860::8888".parse().unwrap(), false));
        assert!(is_allowed_peer("8.8.8.8".parse().unwrap(), true));
    }

    #[test]
    fn evalúa_ipv4_mapeada_como_ipv4() {
        assert!(is_allowed_peer(
            "::ffff:192.168.0.1".parse().unwrap(),
            false
        ));
        assert!(!is_allowed_peer("::ffff:8.8.8.8".parse().unwrap(), false));
    }

    #[test]
    fn origin_ausente_o_local_se_acepta() {
        assert!(is_allowed_origin(None, OriginPolicy::SameHost, &[]));
        assert!(is_allowed_origin(
            Some("http://192.168.1.10:8080"),
            OriginPolicy::SameHost,
            &[]
        ));
        assert!(is_allowed_origin(
            Some("http://localhost:8080"),
            OriginPolicy::SameHost,
            &[]
        ));
        assert!(is_allowed_origin(
            Some("http://[::1]:8080"),
            OriginPolicy::SameHost,
            &[]
        ));
    }

    #[test]
    fn origin_publico_o_desconocido_se_rechaza() {
        assert!(!is_allowed_origin(
            Some("http://evil.example.com"),
            OriginPolicy::SameHost,
            &[]
        ));
        assert!(!is_allowed_origin(
            Some("http://8.8.8.8"),
            OriginPolicy::SameHost,
            &[]
        ));
    }

    #[test]
    fn origin_explicito_en_la_lista_se_acepta() {
        let allowed = vec!["http://evo-suite.local:8080".to_string()];
        assert!(is_allowed_origin(
            Some("http://evo-suite.local:8080"),
            OriginPolicy::SameHost,
            &allowed
        ));
    }

    #[test]
    fn allow_any_acepta_cualquier_origin() {
        assert!(is_allowed_origin(
            Some("http://evil.example.com"),
            OriginPolicy::AllowAny,
            &[]
        ));
    }
}
