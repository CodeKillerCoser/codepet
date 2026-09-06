use super::RemoteLanInterface;
use netdev::interface::types::InterfaceType;
use netdev::Interface;
#[cfg(windows)]
#[path = "windows.rs"]
mod implementation;
#[cfg(unix)]
#[path = "unix.rs"]
mod implementation;

pub(super) fn interfaces() -> Vec<RemoteLanInterface> {
    netdev::get_interfaces()
        .into_iter()
        .flat_map(|interface| {
            let physical = implementation::physical(&interface);
            let available = interface.is_up() && interface.is_running() && interface.is_oper_up();
            let kind = match interface.if_type {
                InterfaceType::Ethernet => "ethernet",
                InterfaceType::Wireless80211 => "wifi",
                _ => "other",
            };
            interface
                .ipv4
                .iter()
                .map(|address| RemoteLanInterface {
                    index: interface.index,
                    name: interface
                        .friendly_name
                        .clone()
                        .unwrap_or_else(|| interface.name.clone()),
                    kind: kind.into(),
                    ipv4: address.addr(),
                    physical,
                    available,
                })
                .collect::<Vec<_>>()
        })
        .collect()
}
