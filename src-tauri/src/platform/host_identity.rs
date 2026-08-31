#[cfg(target_os = "macos")]
const FALLBACK_COMPUTER_NAME: &str = "CodePet Host";

#[cfg(not(target_os = "macos"))]
const FALLBACK_COMPUTER_NAME: &str = "This Device";

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

#[cfg(not(target_os = "macos"))]
fn native_computer_name() -> Option<String> {
    None
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
