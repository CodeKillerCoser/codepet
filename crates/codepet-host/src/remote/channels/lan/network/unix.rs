use super::Interface;
pub(super) fn physical(interface: &Interface) -> bool {
    // netdev uses the platform interface type/flags (and Linux device metadata).
    interface.is_physical() && !interface.is_tun() && !interface.is_loopback()
}
