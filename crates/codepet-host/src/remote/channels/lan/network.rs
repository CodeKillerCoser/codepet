use crate::{HostError, HostResult};
use std::collections::BTreeSet;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};

const ROUTE_PROBE_TARGET: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1)), 9);

#[path = "network/platform.rs"]
mod platform;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteLanInterface {
    pub index: u32,
    pub name: String,
    pub kind: String,
    pub ipv4: Ipv4Addr,
    pub physical: bool,
    pub available: bool,
}

/// Platform metadata is normalized here; callers never inspect adapter names or OS flags.
pub fn remote_lan_interfaces() -> Vec<RemoteLanInterface> {
    platform::interfaces()
}

pub fn remote_lan_interface_for(address: &str) -> Option<RemoteLanInterface> {
    let address = address.parse::<Ipv4Addr>().ok()?;
    remote_lan_interfaces()
        .into_iter()
        .find(|interface| interface.available && interface.ipv4 == address)
}

pub fn select_remote_lan_ipv4(configured_host: Option<&str>) -> HostResult<Ipv4Addr> {
    let interfaces = remote_lan_interfaces();
    select_interface(configured_host, &interfaces, route_probe_ipv4().ok())
        .map(|interface| interface.ipv4)
}

fn select_interface(
    configured_host: Option<&str>,
    interfaces: &[RemoteLanInterface],
    route: Option<Ipv4Addr>,
) -> HostResult<RemoteLanInterface> {
    let active = interfaces
        .iter()
        .filter(|interface| interface.available)
        .map(|interface| (interface.index.to_string(), interface.ipv4))
        .collect::<Vec<_>>();
    if let Some(host) = configured_host {
        let address =
            select_remote_lan_ipv4_from_facts(Some(host), &active, Err(route_probe_unusable()))?;
        return Ok(interfaces
            .iter()
            .find(|interface| interface.available && interface.ipv4 == address)
            .unwrap()
            .clone());
    }
    let mut candidates = interfaces
        .iter()
        .filter(|interface| {
            interface.available
                && interface.physical
                && matches!(interface.kind.as_str(), "ethernet" | "wifi")
                && is_publishable_ipv4(interface.ipv4)
                && !interface.ipv4.is_loopback()
                && !interface.ipv4.is_link_local()
                && !(interface.ipv4.octets()[0] == 198
                    && matches!(interface.ipv4.octets()[1], 18 | 19))
        })
        .collect::<Vec<_>>();
    // A physical default route wins; VPN routes cannot compete. Stable ordering avoids flapping.
    candidates.sort_by_key(|interface| {
        (
            Some(interface.ipv4) != route,
            interface.kind != "ethernet",
            interface.index,
            interface.ipv4,
        )
    });
    let selected = candidates.first().ok_or_else(|| {
        HostError::new(
            "remote_lan_no_physical_interface",
            "No connected physical Wi-Fi or Ethernet interface has a usable IPv4 address",
        )
        .retryable(true)
    })?;
    select_remote_lan_ipv4_from_facts(
        Some(&selected.ipv4.to_string()),
        &active,
        Err(route_probe_unusable()),
    )?;
    Ok((*selected).clone())
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

    fn adapter(
        index: u32,
        kind: &str,
        ip: &str,
        physical: bool,
        available: bool,
    ) -> RemoteLanInterface {
        RemoteLanInterface {
            index,
            name: format!("adapter-{index}"),
            kind: kind.into(),
            ipv4: ip.parse().unwrap(),
            physical,
            available,
        }
    }

    #[test]
    fn virtual_default_route_cannot_displace_physical_wifi() {
        let interfaces = vec![
            adapter(4, "ethernet", "198.18.0.1", false, true),
            adapter(12, "wifi", "192.168.0.106", true, true),
        ];
        assert_eq!(
            select_interface(None, &interfaces, Some("198.18.0.1".parse().unwrap()))
                .unwrap()
                .index,
            12
        );
        assert_eq!(select_interface(None, &interfaces, None).unwrap().index, 12);
    }

    #[test]
    fn physical_routes_rank_first_and_fallback_is_stable() {
        let mut interfaces = vec![
            adapter(12, "wifi", "192.168.0.106", true, true),
            adapter(8, "ethernet", "10.0.0.8", true, true),
        ];
        assert_eq!(
            select_interface(None, &interfaces, Some(interfaces[0].ipv4))
                .unwrap()
                .index,
            12
        );
        assert_eq!(select_interface(None, &interfaces, None).unwrap().index, 8);
        interfaces.reverse();
        assert_eq!(select_interface(None, &interfaces, None).unwrap().index, 8);
        interfaces[0].available = false;
        assert_eq!(select_interface(None, &interfaces, None).unwrap().index, 12);
    }

    #[test]
    fn disconnected_virtual_and_non_lan_addresses_are_not_automatic_candidates() {
        for interface in [
            adapter(1, "ethernet", "10.0.0.1", false, true),
            adapter(2, "wifi", "10.0.0.2", true, false),
            adapter(3, "wifi", "169.254.1.2", true, true),
            adapter(4, "other", "10.0.0.4", true, true),
            adapter(5, "ethernet", "198.18.0.1", true, true),
            adapter(6, "ethernet", "127.0.0.1", true, true),
        ] {
            assert_eq!(
                select_interface(None, &[interface], None).unwrap_err().code,
                "remote_lan_no_physical_interface"
            );
        }
    }

    #[test]
    fn duplicate_ip_is_rejected_even_when_only_one_adapter_is_physical() {
        let interfaces = vec![
            adapter(1, "ethernet", "10.0.0.1", true, true),
            adapter(2, "ethernet", "10.0.0.1", false, true),
        ];
        assert_eq!(
            select_interface(None, &interfaces, None).unwrap_err().code,
            "remote_lan_advertised_host_ambiguous"
        );
    }

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
            select_remote_lan_ipv4_from_facts(None, &interfaces, Ok(Ipv4Addr::UNSPECIFIED),)
                .unwrap_err()
                .code,
            "remote_lan_route_probe_failed"
        );
    }
}
