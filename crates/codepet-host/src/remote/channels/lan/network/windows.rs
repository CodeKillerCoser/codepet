use super::Interface;
use windows_sys::Win32::NetworkManagement::IpHelper::{GetIfEntry2, MIB_IF_ROW2};
pub(super) fn physical(interface: &Interface) -> bool {
    let mut row: MIB_IF_ROW2 = unsafe { std::mem::zeroed() };
    row.InterfaceIndex = interface.index;
    // HardwareInterface is an OS flag, independent of driver names such as Meta or Wintun.
    unsafe { GetIfEntry2(&mut row) == 0 && row.InterfaceAndOperStatusFlags._bitfield & 1 != 0 }
}
