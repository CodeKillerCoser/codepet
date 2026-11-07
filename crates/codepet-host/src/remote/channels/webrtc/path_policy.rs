//! Signed, per-attempt ICE policy. Legacy offers retain mixed ICE behavior.
use super::rtc_error;
use crate::HostResult;
use serde::{Deserialize, Serialize};
use std::net::IpAddr;
use webrtc::{
    api::setting_engine::SettingEngine,
    ice::network_type::NetworkType,
    peer_connection::{
        configuration::RTCConfiguration, policy::ice_transport_policy::RTCIceTransportPolicy,
    },
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(super) enum PathPolicy {
    #[default]
    Auto,
    Ipv6Direct,
    Ipv4Direct,
    Relay,
}

impl PathPolicy {
    pub(super) fn configure(self, configuration: &mut RTCConfiguration) -> SettingEngine {
        let mut settings = SettingEngine::default();
        match self {
            Self::Ipv6Direct => {
                settings.set_network_types(vec![NetworkType::Udp6]);
                configuration.ice_servers.clear();
            }
            Self::Ipv4Direct => settings.set_network_types(vec![NetworkType::Udp4]),
            Self::Relay => configuration.ice_transport_policy = RTCIceTransportPolicy::Relay,
            Self::Auto => return settings,
        }
        for server in &mut configuration.ice_servers {
            server.urls.retain(|url| match self {
                Self::Ipv4Direct => url.starts_with("stun:") || url.starts_with("stuns:"),
                Self::Relay => url.starts_with("turn:") || url.starts_with("turns:"),
                _ => false,
            });
        }
        configuration.ice_servers.retain(|s| !s.urls.is_empty());
        settings
    }

    pub(super) fn accepts(self, line: &str) -> bool {
        if self == Self::Auto {
            return true;
        }
        let fields: Vec<_> = line.split_whitespace().collect();
        if fields.len() < 8 || fields[6] != "typ" || !fields[2].eq_ignore_ascii_case("udp") {
            return false;
        }
        if self == Self::Relay {
            return fields[7] == "relay";
        }
        if !matches!(fields[7], "host" | "srflx" | "prflx") {
            return false;
        }
        match fields[4].parse::<IpAddr>() {
            Ok(IpAddr::V4(ip)) => {
                self == Self::Ipv4Direct && !ip.is_loopback() && !ip.is_unspecified()
            }
            Ok(IpAddr::V6(ip)) => {
                self == Self::Ipv6Direct
                    && !ip.is_loopback()
                    && !ip.is_unspecified()
                    && !ip.is_unicast_link_local()
                    && ip.to_ipv4_mapped().is_none()
            }
            _ => false,
        }
    }

    pub(super) fn filter_sdp(self, sdp: &str) -> HostResult<String> {
        self.filter_sdp_with_empty(sdp, false)
    }
    pub(super) fn filter_sdp_with_empty(self, sdp: &str, allow_empty: bool) -> HostResult<String> {
        if self == Self::Auto {
            return Ok(sdp.to_owned());
        }
        let mut count = 0;
        let lines: Vec<_> = sdp
            .lines()
            .filter(|line| {
                if line.starts_with("a=remote-candidates:") {
                    return false;
                }
                if !line.starts_with("a=candidate:") {
                    return true;
                }
                let accepted = self.accepts(line);
                count += usize::from(accepted);
                accepted
            })
            .collect();
        if count == 0 && !allow_empty {
            return Err(rtc_error("no ICE candidates for requested path"));
        }
        Ok(lines.join("\r\n") + "\r\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn paths_are_disjoint_and_keep_sdp_identity() {
        let sdp = "v=0\r\na=fingerprint:sha-256 identity\r\na=candidate:1 1 udp 1 2001:db8::1 1234 typ host\r\na=candidate:2 1 udp 1 192.0.2.1 1234 typ srflx\r\na=candidate:3 1 udp 1 192.0.2.2 1234 typ relay\r\na=candidate:4 1 udp 1 fe80::1 1234 typ host\r\n";
        for (policy, address) in [
            (PathPolicy::Ipv6Direct, "2001:db8::1"),
            (PathPolicy::Ipv4Direct, "192.0.2.1"),
            (PathPolicy::Relay, "192.0.2.2"),
        ] {
            let filtered = policy.filter_sdp(sdp).unwrap();
            assert_eq!(filtered.matches("a=candidate:").count(), 1);
            assert!(filtered.contains(address));
            assert!(filtered.contains("a=fingerprint:sha-256 identity"));
        }
        assert!(PathPolicy::Ipv6Direct.filter_sdp("v=0\r\n").is_err());
        assert_eq!(PathPolicy::Auto.filter_sdp(sdp).unwrap(), sdp);
    }
    #[test]
    fn direct_never_allocates_turn_and_relay_is_forced() {
        use webrtc::ice_transport::ice_server::RTCIceServer;
        for policy in [
            PathPolicy::Ipv6Direct,
            PathPolicy::Ipv4Direct,
            PathPolicy::Relay,
        ] {
            let mut config = RTCConfiguration {
                ice_servers: vec![RTCIceServer {
                    urls: vec!["stun:example:3478".into(), "turn:example:3478".into()],
                    ..Default::default()
                }],
                ..Default::default()
            };
            policy.configure(&mut config);
            match policy {
                PathPolicy::Ipv6Direct => assert!(config.ice_servers.is_empty()),
                PathPolicy::Ipv4Direct => {
                    assert_eq!(config.ice_servers[0].urls, ["stun:example:3478"])
                }
                PathPolicy::Relay => {
                    assert_eq!(config.ice_transport_policy, RTCIceTransportPolicy::Relay);
                    assert_eq!(config.ice_servers[0].urls, ["turn:example:3478"]);
                }
                _ => unreachable!(),
            }
        }
    }
}
