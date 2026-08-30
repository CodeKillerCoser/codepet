use crate::{HostError, HostResult};
use std::collections::BTreeSet;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};

const ROUTE_PROBE_TARGET: SocketAddr = SocketAddr::new(
    IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1)),
    9,
);

/// Selects the single active local IPv4 endpoint that the App may advertise.
///
/// UDP `connect` only asks the operating system to select a route; this function
/// never writes a datagram. An explicit host is still checked against active local
/// interfaces and the same publishability rules.
pub fn select_remote_lan_ipv4(configured_host: Option<&str>) -> HostResult<Ipv4Addr> {
    let active = if_addrs::get_if_addrs()
        .map_err(|error| {
            HostError::new(
                "remote_lan_interface_query_failed",
                format!("enumerate active local interfaces: {error}"),
            )
            .retryable(true)
        })?
        .into_iter()
        .filter(|interface| interface.is_oper_up())
        .filter_map(|interface| match interface.ip() {
            IpAddr::V4(address) => Some((interface.name, address)),
            IpAddr::V6(_) => None,
        })
        .collect::<Vec<_>>();

    let probed = if configured_host.is_some() {
        Err(route_probe_unusable())
    } else {
        route_probe_ipv4()
    };
    select_remote_lan_ipv4_from_facts(configured_host, &active, probed)
}

fn route_probe_ipv4() -> HostResult<Ipv4Addr> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).map_err(|error| {
        HostError::new(
            "remote_lan_route_probe_failed",
            format!("bind no-send IPv4 route probe: {error}"),
        )
        .retryable(true)
    })?;
    socket.connect(ROUTE_PROBE_TARGET).map_err(|error| {
        HostError::new(
            "remote_lan_route_probe_failed",
            format!("select the local IPv4 route for LAN discovery: {error}"),
        )
        .retryable(true)
    })?;
    match socket.local_addr().map(|address| address.ip()) {
        Ok(IpAddr::V4(address)) => Ok(address),
        Ok(IpAddr::V6(_)) => Err(route_probe_unusable()),
        Err(error) => Err(HostError::new(
            "remote_lan_route_probe_failed",
            format!("read the no-send IPv4 route probe result: {error}"),
        )
        .retryable(true)),
    }
}

fn select_remote_lan_ipv4_from_facts(
    configured_host: Option<&str>,
    active: &[(String, Ipv4Addr)],
    probed: HostResult<Ipv4Addr>,
) -> HostResult<Ipv4Addr> {
    let candidate = match configured_host {
        Some(host) => host.trim().parse::<Ipv4Addr>().map_err(|_| {
            HostError::new(
                "invalid_remote_lan_advertised_host",
                "CODEPET_REMOTE_ADVERTISED_HOST must be one concrete local IPv4 address",
            )
        })?,
        None => probed?,
    };
    if !is_publishable_ipv4(candidate) {
        if configured_host.is_none() {
            return Err(route_probe_unusable());
        }
        return Err(HostError::new(
            "invalid_remote_lan_advertised_host",
            "Remote LAN advertised IPv4 address is not a publishable unicast endpoint",
        ));
    }

    let matching_interfaces = active
        .iter()
        .filter(|(_, address)| *address == candidate)
        .map(|(name, _)| name.clone())
        .collect::<BTreeSet<_>>();
    match matching_interfaces.len() {
        1 => Ok(candidate),
        0 => Err(HostError::new(
            "remote_lan_advertised_host_not_local",
            "Remote LAN advertised IPv4 address is not assigned to an active local interface",
        )
        .retryable(configured_host.is_none())),
        _ => Err(HostError::new(
            "remote_lan_advertised_host_ambiguous",
            "Remote LAN advertised IPv4 address is assigned to multiple active interfaces",
        )
        .retryable(true)),
    }
}

fn route_probe_unusable() -> HostError {
    HostError::new(
        "remote_lan_route_probe_failed",
        "The no-send route probe did not select a concrete IPv4 endpoint",
    )
    .retryable(true)
}

fn is_publishable_ipv4(address: Ipv4Addr) -> bool {
    let octets = address.octets();
    !address.is_unspecified()
        && !address.is_multicast()
        && octets[0] != 0
        && octets[0] < 240
        && !matches!(
            octets,
            [192, 0, 2, _] | [198, 51, 100, _] | [203, 0, 113, _]
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn active(values: &[(&str, &str)]) -> Vec<(String, Ipv4Addr)> {
        values
            .iter()
            .map(|(name, address)| ((*name).to_string(), address.parse().unwrap()))
            .collect()
    }

    #[test]
    fn explicit_and_route_selected_ipv4_must_be_unambiguous_active_local_addresses() {
        let interfaces = active(&[("lo0", "127.0.0.1"), ("en0", "192.168.1.8")]);
        assert_eq!(
            select_remote_lan_ipv4_from_facts(
                Some("127.0.0.1"),
                &interfaces,
                Err(route_probe_unusable()),
            )
            .unwrap(),
            Ipv4Addr::LOCALHOST
        );
        assert_eq!(
            select_remote_lan_ipv4_from_facts(
                None,
                &interfaces,
                Ok("192.168.1.8".parse().unwrap()),
            )
            .unwrap(),
            "192.168.1.8".parse::<Ipv4Addr>().unwrap()
        );
        assert_eq!(
            select_remote_lan_ipv4_from_facts(None, &interfaces, Err(route_probe_unusable()))
                .unwrap_err()
                .code,
            "remote_lan_route_probe_failed"
        );
    }

    #[test]
    fn nonlocal_nonpublishable_and_duplicate_interface_addresses_fail_closed() {
        let interfaces = active(&[("en0", "192.168.1.8"), ("bridge0", "192.168.1.8")]);
        assert_eq!(
            select_remote_lan_ipv4_from_facts(
                Some("192.168.1.8"),
                &interfaces,
                Ok(Ipv4Addr::LOCALHOST),
            )
            .unwrap_err()
            .code,
            "remote_lan_advertised_host_ambiguous"
        );
        assert_eq!(
            select_remote_lan_ipv4_from_facts(
                Some("192.0.2.1"),
                &interfaces,
                Ok(Ipv4Addr::LOCALHOST),
            )
            .unwrap_err()
            .code,
            "invalid_remote_lan_advertised_host"
        );
        assert_eq!(
            select_remote_lan_ipv4_from_facts(
                Some("192.168.1.9"),
                &interfaces,
                Ok(Ipv4Addr::LOCALHOST),
            )
            .unwrap_err()
            .code,
            "remote_lan_advertised_host_not_local"
        );
        assert_eq!(
            select_remote_lan_ipv4_from_facts(
                None,
                &interfaces,
                Ok(Ipv4Addr::UNSPECIFIED),
            )
            .unwrap_err()
            .code,
            "remote_lan_route_probe_failed"
        );
    }
}
