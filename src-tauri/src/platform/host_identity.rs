const FALLBACK_COMPUTER_NAME: &str = "CodePet Host";

pub fn computer_name() -> String {
    resolve_computer_name(native_computer_name)
}

fn resolve_computer_name(reader: impl FnOnce() -> Option<String>) -> String {
    reader()
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| FALLBACK_COMPUTER_NAME.to_string())
}

#[cfg(target_os = "macos")]
fn native_computer_name() -> Option<String> {
    macos::computer_name()
}

#[cfg(target_os = "windows")]
fn native_computer_name() -> Option<String> {
    windows::computer_name()
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn native_computer_name() -> Option<String> {
    None
}

#[cfg(target_os = "windows")]
mod windows {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetComputerNameW(buffer: *mut u16, size: *mut u32) -> i32;
    }

    pub(super) fn computer_name() -> Option<String> {
        // MAX_COMPUTERNAME_LENGTH is 15 UTF-16 units, plus the terminator.
        let mut buffer = [0u16; 16];
        let mut size = buffer.len() as u32;
        // Both pointers remain valid for the call; size declares buffer capacity.
        if unsafe { GetComputerNameW(buffer.as_mut_ptr(), &mut size) } == 0 {
            return None;
        }
        String::from_utf16(buffer.get(..size as usize)?).ok()
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use core_foundation::base::TCFType;
    use core_foundation::string::{CFString, CFStringRef};
    use std::ffi::c_void;
    use std::ptr;

    #[link(name = "SystemConfiguration", kind = "framework")]
    unsafe extern "C" {
        fn SCDynamicStoreCopyComputerName(
            store: *const c_void,
            name_encoding: *mut u32,
        ) -> CFStringRef;
    }

    pub(super) fn computer_name() -> Option<String> {
        let value = unsafe {
            SCDynamicStoreCopyComputerName(ptr::null(), ptr::null_mut())
        };
        if value.is_null() {
            return None;
        }
        Some(unsafe { CFString::wrap_under_create_rule(value) }.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::{resolve_computer_name, FALLBACK_COMPUTER_NAME};

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_reads_native_computer_name() {
        let name = super::native_computer_name().expect("Windows computer name");
        assert!(!name.trim().is_empty());
        assert!(!name.contains('\0'));
        assert_eq!(super::computer_name(), name.trim());
        if let Ok(expected) = std::env::var("COMPUTERNAME") {
            assert_eq!(name.to_uppercase(), expected.to_uppercase());
        }
    }

    #[test]
    fn computer_name_resolution_uses_trimmed_native_value_and_safe_fallback() {
        assert_eq!(
            resolve_computer_name(|| Some("  Studio Mac  ".to_string())),
            "Studio Mac"
        );
        assert_eq!(resolve_computer_name(|| None), FALLBACK_COMPUTER_NAME);
        assert_eq!(
            resolve_computer_name(|| Some(" \n\t".to_string())),
            FALLBACK_COMPUTER_NAME
        );
    }
}
