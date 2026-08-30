use crate::{HostError, HostResult, RemoteAccessManager, RemoteLanServerHandle};
use codepet_gateway_sdk::{RemoteHostIdentity, PROTOCOL_VERSION};
use mdns_sd::{
    DaemonStatus, Error as MdnsError, IfKind, ServiceDaemon, ServiceInfo, UnregisterStatus,
};
use ring::digest::{digest, SHA256};
use std::fmt::Write;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

pub const REMOTE_LAN_MDNS_SERVICE_TYPE: &str = "_codepet._tcp.local.";

const INSTANCE_LABEL_MAX_BYTES: usize = 63;
const INSTANCE_SUFFIX_BYTES: usize = 6;
const DAEMON_RESPONSE_TIMEOUT: Duration = Duration::from_secs(2);

/// Publishes one running Remote LAN listener through DNS-SD.
///
/// Discovery is intentionally not an identity or trust boundary. Stable Host identity
/// remains the `id` TXT value, while TLS trust is established by the pairing flow.
pub struct RemoteLanMdnsAdvertiser {
    backend: Box<dyn MdnsBackend>,
    service: MdnsServiceSpec,
    fullname: String,
    pairing_available: bool,
    service_registered: bool,
    daemon_stopped: bool,
}

impl RemoteLanMdnsAdvertiser {
    pub fn start(
        remote_access: &RemoteAccessManager,
        listener: &RemoteLanServerHandle,
        pairing_available: bool,
    ) -> HostResult<Self> {
        let service = MdnsServiceSpec::from_listener(
            &remote_access.remote_host_identity(),
            listener.advertised_host(),
            listener.local_addr(),
        )?;
        let backend = Box::new(ServiceDaemonBackend::new()?);
        Self::start_with_backend(service, pairing_available, backend)
    }

    /// Re-announces the same service with only the `pair` TXT value changed.
    pub fn update_pairing_available(&mut self, pairing_available: bool) -> HostResult<()> {
        if self.daemon_stopped || !self.service_registered {
            return Err(HostError::new(
                "remote_lan_mdns_not_running",
                "Remote LAN mDNS advertiser is not running",
            ));
        }
        if self.pairing_available == pairing_available {
            return Ok(());
        }

        self.backend
            .register(self.service.service_info(pairing_available)?)
            .map_err(|message| mdns_backend_error("register", message))?;
        self.pairing_available = pairing_available;
        Ok(())
    }

    /// Gracefully unregisters the service and then stops its daemon.
    ///
    /// Repeated successful calls are no-ops. If either backend step fails, a later call
    /// safely retries only the work that may still be outstanding.
    pub fn shutdown(&mut self) -> HostResult<()> {
        if self.daemon_stopped {
            return Ok(());
        }

        let mut first_error = None;
        if self.service_registered {
            match self.backend.unregister(&self.fullname) {
                Ok(()) => self.service_registered = false,
                Err(message) => {
                    first_error = Some(mdns_backend_error("unregister", message));
                }
            }
        }

        match self.backend.shutdown() {
            Ok(()) => {
                self.service_registered = false;
                self.daemon_stopped = true;
            }
            Err(message) if first_error.is_none() => {
                first_error = Some(mdns_backend_error("shutdown", message));
            }
            Err(_) => {}
        }

        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    fn start_with_backend(
        service: MdnsServiceSpec,
        pairing_available: bool,
        mut backend: Box<dyn MdnsBackend>,
    ) -> HostResult<Self> {
        let service_info = match service.service_info(pairing_available) {
            Ok(service_info) => service_info,
            Err(error) => {
                let _ = backend.shutdown();
                return Err(error);
            }
        };
        let fullname = service_info.get_fullname().to_string();
        if let Err(message) = backend.register(service_info) {
            let _ = backend.shutdown();
            return Err(mdns_backend_error("register", message));
        }

        Ok(Self {
            backend,
            service,
            fullname,
            pairing_available,
            service_registered: true,
            daemon_stopped: false,
        })
    }
}

impl Drop for RemoteLanMdnsAdvertiser {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

#[derive(Clone, Debug)]
struct MdnsServiceSpec {
    device_id: String,
    display_name: String,
    instance_name: String,
    hostname: String,
    address: IpAddr,
    port: u16,
}

impl MdnsServiceSpec {
    fn from_listener(
        identity: &RemoteHostIdentity,
        advertised_host: &str,
        local_addr: SocketAddr,
    ) -> HostResult<Self> {
        if identity.device_id.trim().is_empty() || identity.display_name.trim().is_empty() {
            return Err(HostError::new(
                "invalid_remote_lan_mdns_identity",
                "Remote LAN mDNS identity requires a device id and display name",
            ));
        }
        if local_addr.port() == 0 {
            return Err(invalid_mdns_endpoint(
                "Remote LAN listener has no bound TLS port",
            ));
        }

        let address = advertised_host.parse::<IpAddr>().map_err(|_| {
            invalid_mdns_endpoint(
                "Remote LAN mDNS requires an explicit concrete IP advertised by the listener",
            )
        })?;
        validate_endpoint_address(address, local_addr.ip())?;

        let suffix = identity_suffix(&identity.device_id);
        let instance_name = instance_name(&identity.display_name, &suffix);
        let hostname = format!("codepet-{suffix}.local.");

        Ok(Self {
            device_id: identity.device_id.clone(),
            display_name: identity.display_name.clone(),
            instance_name,
            hostname,
            address,
            port: local_addr.port(),
        })
    }

    fn service_info(&self, pairing_available: bool) -> HostResult<ServiceInfo> {
        let version = PROTOCOL_VERSION.to_string();
        let pairing = if pairing_available { "1" } else { "0" };
        let properties = [
            ("id", self.device_id.as_str()),
            ("name", self.display_name.as_str()),
            ("vmin", version.as_str()),
            ("vmax", version.as_str()),
            ("pair", pairing),
        ];
        let mut service = ServiceInfo::new(
            REMOTE_LAN_MDNS_SERVICE_TYPE,
            &self.instance_name,
            &self.hostname,
            self.address,
            self.port,
            &properties[..],
        )
        .map_err(|error| {
            HostError::new(
                "invalid_remote_lan_mdns_service",
                format!("construct Remote LAN mDNS service: {error}"),
            )
        })?;
        service.set_interfaces(vec![IfKind::Addr(self.address)]);
        Ok(service)
    }
}

fn validate_endpoint_address(address: IpAddr, bind_address: IpAddr) -> HostResult<()> {
    if !is_publishable_unicast(address) {
        return Err(invalid_mdns_endpoint(
            "Remote LAN mDNS advertised IP must be a concrete unicast endpoint",
        ));
    }
    if bind_address.is_unspecified() {
        if address.is_ipv4() != bind_address.is_ipv4() {
            return Err(invalid_mdns_endpoint(
                "Remote LAN listener and advertised IP address families do not match",
            ));
        }
    } else if address != bind_address {
        return Err(invalid_mdns_endpoint(
            "Remote LAN advertised IP does not match the listener bind address",
        ));
    }
    Ok(())
}

fn is_publishable_unicast(address: IpAddr) -> bool {
    if address.is_unspecified() || address.is_multicast() {
        return false;
    }
    match address {
        IpAddr::V4(address) => {
            let octets = address.octets();
            octets[0] != 0
                && octets[0] < 240
                && !is_documentation_v4(address)
        }
        IpAddr::V6(address) => {
            let segments = address.segments();
            !(segments[0] == 0x2001 && segments[1] == 0x0db8)
        }
    }
}

fn is_documentation_v4(address: Ipv4Addr) -> bool {
    let octets = address.octets();
    matches!(
        octets,
        [192, 0, 2, _] | [198, 51, 100, _] | [203, 0, 113, _]
    )
}

fn invalid_mdns_endpoint(message: impl Into<String>) -> HostError {
    HostError::new("invalid_remote_lan_mdns_endpoint", message)
}

fn identity_suffix(device_id: &str) -> String {
    let digest = digest(&SHA256, device_id.as_bytes());
    let mut suffix = String::with_capacity(INSTANCE_SUFFIX_BYTES * 2);
    for byte in &digest.as_ref()[..INSTANCE_SUFFIX_BYTES] {
        write!(&mut suffix, "{byte:02x}").expect("writing to a String cannot fail");
    }
    suffix
}

fn instance_name(display_name: &str, suffix: &str) -> String {
    let readable = display_name
        .chars()
        .map(|character| match character {
            '.' | '\\' => ' ',
            character if character.is_control() => ' ',
            character => character,
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let readable = if readable.is_empty() {
        "CodePet"
    } else {
        readable.as_str()
    };
    let suffix = format!(" ({suffix})");
    let display_bytes = INSTANCE_LABEL_MAX_BYTES.saturating_sub(suffix.len());
    let display = truncate_utf8(readable, display_bytes).trim_end();
    format!("{display}{suffix}")
}

fn truncate_utf8(value: &str, max_bytes: usize) -> &str {
    if value.len() <= max_bytes {
        return value;
    }
    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

fn mdns_backend_error(operation: &str, message: String) -> HostError {
    HostError::new(
        format!("remote_lan_mdns_{operation}_failed"),
        format!("Remote LAN mDNS {operation} failed: {message}"),
    )
    .retryable(true)
}

trait MdnsBackend: Send {
    fn register(&mut self, service: ServiceInfo) -> Result<(), String>;
    fn unregister(&mut self, fullname: &str) -> Result<(), String>;
    fn shutdown(&mut self) -> Result<(), String>;
}

struct ServiceDaemonBackend {
    daemon: ServiceDaemon,
}

impl ServiceDaemonBackend {
    fn new() -> HostResult<Self> {
        let daemon = ServiceDaemon::new().map_err(|error| {
            mdns_backend_error("start", format!("create service daemon: {error}"))
        })?;
        Ok(Self { daemon })
    }
}

impl MdnsBackend for ServiceDaemonBackend {
    fn register(&mut self, service: ServiceInfo) -> Result<(), String> {
        self.daemon
            .register(service)
            .map_err(|error| error.to_string())
    }

    fn unregister(&mut self, fullname: &str) -> Result<(), String> {
        let receiver = match self.daemon.unregister(fullname) {
            Ok(receiver) => receiver,
            Err(MdnsError::DaemonShutdown) => return Ok(()),
            Err(error) => return Err(error.to_string()),
        };
        match receiver.recv_timeout(DAEMON_RESPONSE_TIMEOUT) {
            Ok(UnregisterStatus::OK | UnregisterStatus::NotFound) => Ok(()),
            Err(error) => Err(format!("wait for service unregister: {error}")),
        }
    }

    fn shutdown(&mut self) -> Result<(), String> {
        let receiver = match self.daemon.shutdown() {
            Ok(receiver) => receiver,
            Err(MdnsError::DaemonShutdown) => return Ok(()),
            Err(error) => return Err(error.to_string()),
        };
        match receiver.recv_timeout(DAEMON_RESPONSE_TIMEOUT) {
            Ok(DaemonStatus::Shutdown) => Ok(()),
            Ok(status) => Err(format!("unexpected daemon shutdown status: {status:?}")),
            Err(error) => Err(format!("wait for daemon shutdown: {error}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct ServiceSnapshot {
        service_type: String,
        fullname: String,
        hostname: String,
        addresses: Vec<IpAddr>,
        port: u16,
        properties: BTreeMap<String, String>,
    }

    impl From<ServiceInfo> for ServiceSnapshot {
        fn from(service: ServiceInfo) -> Self {
            let mut addresses = service.get_addresses().iter().copied().collect::<Vec<_>>();
            addresses.sort();
            let properties = service
                .get_properties()
                .iter()
                .map(|property| (property.key().to_string(), property.val_str().to_string()))
                .collect();
            Self {
                service_type: service.get_type().to_string(),
                fullname: service.get_fullname().to_string(),
                hostname: service.get_hostname().to_string(),
                addresses,
                port: service.get_port(),
                properties,
            }
        }
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    enum BackendCall {
        Register(ServiceSnapshot),
        Unregister(String),
        Shutdown,
    }

    struct FakeBackend {
        calls: Arc<Mutex<Vec<BackendCall>>>,
        fail_register: bool,
        fail_unregister: bool,
    }

    impl FakeBackend {
        fn new(calls: Arc<Mutex<Vec<BackendCall>>>) -> Self {
            Self {
                calls,
                fail_register: false,
                fail_unregister: false,
            }
        }
    }

    impl MdnsBackend for FakeBackend {
        fn register(&mut self, service: ServiceInfo) -> Result<(), String> {
            self.calls
                .lock()
                .unwrap()
                .push(BackendCall::Register(service.into()));
            if self.fail_register {
                Err("register failed".to_string())
            } else {
                Ok(())
            }
        }

        fn unregister(&mut self, fullname: &str) -> Result<(), String> {
            self.calls
                .lock()
                .unwrap()
                .push(BackendCall::Unregister(fullname.to_string()));
            if self.fail_unregister {
                Err("unregister failed".to_string())
            } else {
                Ok(())
            }
        }

        fn shutdown(&mut self) -> Result<(), String> {
            self.calls.lock().unwrap().push(BackendCall::Shutdown);
            Ok(())
        }
    }

    fn identity(device_id: &str, display_name: &str) -> RemoteHostIdentity {
        RemoteHostIdentity {
            device_id: device_id.to_string(),
            display_name: display_name.to_string(),
            identity_fingerprint: "must-not-leak-fingerprint".to_string(),
        }
    }

    fn service_spec(device_id: &str, display_name: &str) -> MdnsServiceSpec {
        MdnsServiceSpec::from_listener(
            &identity(device_id, display_name),
            "192.168.1.23",
            "0.0.0.0:43123".parse().unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn service_info_uses_exact_discovery_contract_and_listener_port() {
        let service = service_spec("device-alpha", "Living Room")
            .service_info(false)
            .unwrap();
        let snapshot = ServiceSnapshot::from(service);

        assert_eq!(snapshot.service_type, REMOTE_LAN_MDNS_SERVICE_TYPE);
        assert_eq!(
            snapshot.addresses,
            vec!["192.168.1.23".parse::<IpAddr>().unwrap()]
        );
        assert_eq!(snapshot.port, 43123);
        assert!(snapshot.fullname.starts_with("Living Room ("));
        assert_eq!(
            snapshot.properties,
            BTreeMap::from([
                ("id".to_string(), "device-alpha".to_string()),
                ("name".to_string(), "Living Room".to_string()),
                ("pair".to_string(), "0".to_string()),
                ("vmax".to_string(), PROTOCOL_VERSION.to_string()),
                ("vmin".to_string(), PROTOCOL_VERSION.to_string()),
            ])
        );
        let published = format!("{snapshot:?}");
        assert!(!published.contains("must-not-leak-fingerprint"));
        assert!(!published.contains("secret"));
        assert!(!published.contains("token"));
        assert!(!published.contains("provider"));
        assert!(!published.contains("project"));
        assert!(!published.contains("conversation"));
    }

    #[test]
    fn endpoint_validation_fails_closed_and_instance_names_are_collision_safe() {
        let host = identity("device-alpha", "Very.long\\ Living\nRoom");
        let dns_error = MdnsServiceSpec::from_listener(
            &host,
            "listener.local",
            "0.0.0.0:43123".parse().unwrap(),
        )
        .unwrap_err();
        assert_eq!(dns_error.code, "invalid_remote_lan_mdns_endpoint");

        let unspecified_error = MdnsServiceSpec::from_listener(
            &host,
            "0.0.0.0",
            "0.0.0.0:43123".parse().unwrap(),
        )
        .unwrap_err();
        assert_eq!(
            unspecified_error.code,
            "invalid_remote_lan_mdns_endpoint"
        );

        let mismatch_error = MdnsServiceSpec::from_listener(
            &host,
            "192.168.1.23",
            "127.0.0.1:43123".parse().unwrap(),
        )
        .unwrap_err();
        assert_eq!(mismatch_error.code, "invalid_remote_lan_mdns_endpoint");

        let loopback = MdnsServiceSpec::from_listener(
            &host,
            "127.0.0.1",
            "127.0.0.1:43123".parse().unwrap(),
        )
        .unwrap();
        assert_eq!(loopback.address, "127.0.0.1".parse::<IpAddr>().unwrap());

        let first = service_spec("device-alpha", &"客厅桌宠".repeat(20));
        let second = service_spec("device-beta", &"客厅桌宠".repeat(20));
        assert_ne!(first.instance_name, second.instance_name);
        assert_ne!(first.hostname, second.hostname);
        assert!(first.instance_name.len() <= INSTANCE_LABEL_MAX_BYTES);
        assert!(first.instance_name.starts_with("客厅桌宠"));
    }

    #[test]
    fn pairing_flip_reregisters_same_service_and_shutdown_is_idempotent() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let backend = Box::new(FakeBackend::new(calls.clone()));
        let mut advertiser = RemoteLanMdnsAdvertiser::start_with_backend(
            service_spec("device-alpha", "Living Room"),
            false,
            backend,
        )
        .unwrap();

        advertiser.update_pairing_available(true).unwrap();
        advertiser.update_pairing_available(true).unwrap();
        advertiser.shutdown().unwrap();
        advertiser.shutdown().unwrap();
        drop(advertiser);

        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 4);
        let (first, second) = match (&calls[0], &calls[1]) {
            (BackendCall::Register(first), BackendCall::Register(second)) => (first, second),
            other => panic!("unexpected register calls: {other:?}"),
        };
        assert_eq!(first.fullname, second.fullname);
        assert_eq!(first.hostname, second.hostname);
        assert_eq!(first.addresses, second.addresses);
        assert_eq!(first.port, second.port);
        assert_eq!(first.properties.get("pair").unwrap(), "0");
        assert_eq!(second.properties.get("pair").unwrap(), "1");
        let mut expected = first.properties.clone();
        expected.insert("pair".to_string(), "1".to_string());
        assert_eq!(second.properties, expected);
        assert_eq!(calls[2], BackendCall::Unregister(first.fullname.clone()));
        assert_eq!(calls[3], BackendCall::Shutdown);
    }

    #[test]
    fn lifecycle_failures_still_stop_the_daemon() {
        let register_calls = Arc::new(Mutex::new(Vec::new()));
        let mut failing_register = FakeBackend::new(register_calls.clone());
        failing_register.fail_register = true;
        let error = RemoteLanMdnsAdvertiser::start_with_backend(
            service_spec("device-alpha", "Living Room"),
            false,
            Box::new(failing_register),
        )
        .err()
        .unwrap();
        assert_eq!(error.code, "remote_lan_mdns_register_failed");
        assert!(matches!(
            register_calls.lock().unwrap().as_slice(),
            [BackendCall::Register(_), BackendCall::Shutdown]
        ));

        let shutdown_calls = Arc::new(Mutex::new(Vec::new()));
        let mut failing_unregister = FakeBackend::new(shutdown_calls.clone());
        failing_unregister.fail_unregister = true;
        let mut advertiser = RemoteLanMdnsAdvertiser::start_with_backend(
            service_spec("device-alpha", "Living Room"),
            false,
            Box::new(failing_unregister),
        )
        .unwrap();
        let error = advertiser.shutdown().unwrap_err();
        assert_eq!(error.code, "remote_lan_mdns_unregister_failed");
        advertiser.shutdown().unwrap();
        assert!(matches!(
            shutdown_calls.lock().unwrap().as_slice(),
            [
                BackendCall::Register(_),
                BackendCall::Unregister(_),
                BackendCall::Shutdown
            ]
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_real_backend_start_update_shutdown_smoke() {
        let service = MdnsServiceSpec::from_listener(
            &identity("device-smoke", "CodePet Smoke"),
            "127.0.0.1",
            "127.0.0.1:43123".parse().unwrap(),
        )
        .unwrap();
        let backend = Box::new(ServiceDaemonBackend::new().unwrap());
        let mut advertiser =
            RemoteLanMdnsAdvertiser::start_with_backend(service, false, backend).unwrap();

        advertiser.update_pairing_available(true).unwrap();
        advertiser.shutdown().unwrap();
        advertiser.shutdown().unwrap();
    }
}
