//! Resolución de la IP de LAN a la que enlazar el servidor remoto (SAD §8.4).
//!
//! El servidor remoto **nunca** escucha en `0.0.0.0`: se enlaza a la IP concreta
//! de la interfaz elegida (más loopback). Como esa IP puede cambiar por DHCP, el
//! daemon la resuelve al arrancar por **nombre de interfaz**, no por dirección.
//!
//! La lógica de selección es pura y se prueba sin tocar el sistema; solo el
//! proveedor real ([`lan_ipv4`]) consulta las interfaces.

use std::net::IpAddr;
use std::net::Ipv4Addr;

use thiserror::Error;

/// Error al resolver la interfaz de LAN.
#[derive(Debug, Error)]
pub enum NetError {
    /// No se pudieron enumerar las interfaces del sistema.
    #[error("no se pudieron enumerar las interfaces de red: {0}")]
    Enumerate(#[source] std::io::Error),
    /// La interfaz no existe o no tiene una IPv4 utilizable.
    #[error("la interfaz `{0}` no existe o no tiene una IPv4 utilizable")]
    NoAddress(String),
}

/// IPv4 no-loopback y no-comodín de la interfaz indicada.
///
/// Ignora loopback (`127.0.0.0/8`) y comodín (`0.0.0.0`) a propósito: enlazar el
/// control remoto a ellos sería, respectivamente, redundante o peligroso
/// (SAD §8.4).
pub fn lan_ipv4(interface: &str) -> Result<Ipv4Addr, NetError> {
    let interfaces = if_addrs::get_if_addrs().map_err(NetError::Enumerate)?;
    let addrs: Vec<(String, IpAddr)> = interfaces
        .into_iter()
        .map(|iface| {
            let ip = match iface.addr {
                if_addrs::IfAddr::V4(v4) => IpAddr::V4(v4.ip),
                if_addrs::IfAddr::V6(v6) => IpAddr::V6(v6.ip),
            };
            (iface.name, ip)
        })
        .collect();

    select_ipv4(&addrs, interface).ok_or_else(|| NetError::NoAddress(interface.to_string()))
}

/// Selección pura: primera IPv4 utilizable de la interfaz indicada.
///
/// Separada de la enumeración para poder probarla con una lista fabricada.
fn select_ipv4(addrs: &[(String, IpAddr)], interface: &str) -> Option<Ipv4Addr> {
    addrs.iter().find_map(|(name, ip)| match ip {
        IpAddr::V4(v4) if name == interface && !v4.is_loopback() && !v4.is_unspecified() => {
            Some(*v4)
        }
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addrs() -> Vec<(String, IpAddr)> {
        vec![
            ("lo".to_string(), IpAddr::V4(Ipv4Addr::LOCALHOST)),
            (
                "wlan0".to_string(),
                IpAddr::V4(Ipv4Addr::new(192, 168, 1, 42)),
            ),
            ("eth0".to_string(), IpAddr::V4(Ipv4Addr::UNSPECIFIED)),
            ("wlan0".to_string(), IpAddr::V6("fe80::1".parse().unwrap())),
        ]
    }

    #[test]
    fn elige_la_ipv4_de_la_interfaz_pedida() {
        assert_eq!(
            select_ipv4(&addrs(), "wlan0"),
            Some(Ipv4Addr::new(192, 168, 1, 42))
        );
    }

    #[test]
    fn ignora_loopback_y_comodin() {
        assert_eq!(select_ipv4(&addrs(), "lo"), None, "loopback no sirve");
        assert_eq!(
            select_ipv4(&addrs(), "eth0"),
            None,
            "0.0.0.0 no es una dirección de enlace válida"
        );
    }

    #[test]
    fn una_interfaz_ausente_no_tiene_direccion() {
        assert_eq!(select_ipv4(&addrs(), "wlan1"), None);
    }
}
