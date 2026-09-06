//! Keep the host available while Code Pet runs, without preventing display sleep or lock.
#[cfg(target_os = "macos")]
mod macos {
    use core_foundation::{base::TCFType, string::{CFString, CFStringRef}};

    #[link(name = "IOKit", kind = "framework")]
    extern "C" {
        fn IOPMAssertionCreateWithName(kind: CFStringRef, level: u32, reason: CFStringRef, id: *mut u32) -> i32;
        fn IOPMAssertionRelease(id: u32) -> i32;
    }

    pub struct AwakeGuard(u32);

    impl AwakeGuard {
        pub fn acquire() -> Result<Self, String> {
            let kind = CFString::new("PreventUserIdleSystemSleep");
            let reason = CFString::new("Code Pet keeps remote connections and agent tasks available");
            let mut id = 0;
            // CF strings remain alive for this call. macOS releases process-owned assertions on exit.
            let result = unsafe { IOPMAssertionCreateWithName(kind.as_concrete_TypeRef(), 255, reason.as_concrete_TypeRef(), &mut id) };
            if result == 0 { Ok(Self(id)) } else { Err(format!("IOPMAssertionCreateWithName failed: {result}")) }
        }
    }

    impl Drop for AwakeGuard {
        fn drop(&mut self) { unsafe { IOPMAssertionRelease(self.0); } }
    }
}

#[cfg(target_os = "macos")]
pub use macos::AwakeGuard;

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::AwakeGuard;

    #[test]
    fn native_assertion_is_registered_and_released() {
        let held = || {
            let output = std::process::Command::new("/usr/bin/pmset").args(["-g", "assertions"]).output().unwrap();
            assert!(output.status.success());
            let pid = format!("pid {}(", std::process::id());
            String::from_utf8_lossy(&output.stdout).lines().any(|line|
                line.contains(&pid) && line.contains("PreventUserIdleSystemSleep")
                    && line.contains("Code Pet keeps remote connections"))
        };
        let guard = AwakeGuard::acquire().unwrap();
        assert!(held(), "macOS must report the process-owned idle sleep assertion");
        drop(guard);
        assert!(!held(), "releasing the guard must restore normal idle sleep policy");
    }
}
