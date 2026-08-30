use crate::{HostError, HostResult, RemoteLanServerHandle};
use codepet_gateway_sdk::{RemoteHostIdentity, PROTOCOL_VERSION};
use mdns_sd::{
    DaemonEvent, DaemonStatus, Error as MdnsError, IfKind, Receiver, RecvTimeoutError,
    ServiceDaemon, ServiceInfo, TryRecvError, UnregisterStatus,
};
use ring::digest::{digest, SHA256};
use std::fmt::Write;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::{Duration, Instant};

pub const REMOTE_LAN_MDNS_SERVICE_TYPE: &str = "_codepet._tcp.local.";

const INSTANCE_LABEL_MAX_BYTES: usize = 63;
const INSTANCE_SUFFIX_BYTES: usize = 6;
const DAEMON_RESPONSE_TIMEOUT: Duration = Duration::from_secs(2);
const ANNOUNCE_CONFIRMATION_TIMEOUT: Duration = Duration::from_secs(3);

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
        listener: &RemoteLanServerHandle,
        pairing_available: bool,
    ) -> HostResult<Self> {
        let service = MdnsServiceSpec::from_listener(
            listener.remote_host_identity(),
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
            return self.observe_backend_health();
        }

        let service = self.service.service_info(pairing_available)?;
        if let Err(message) = self.backend.register(service) {
            return Err(self.fail_closed("register", message));
        }
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

        let mut first_error = self
            .backend
            .health()
            .err()
            .map(|message| mdns_backend_error("monitor", message));
        if self.service_registered {
            match self.backend.unregister(&self.fullname) {
                Ok(()) => self.service_registered = false,
                Err(message) if first_error.is_none() => {
                    first_error = Some(mdns_backend_error("unregister", message));
                }
                Err(_) => {}
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

    fn observe_backend_health(&mut self) -> HostResult<()> {
        match self.backend.health() {
            Ok(()) => Ok(()),
            Err(message) => Err(self.fail_closed("monitor", message)),
        }
    }

    fn fail_closed(&mut self, operation: &str, message: String) -> HostError {
        let error = mdns_backend_error(operation, message);
        if self.service_registered && self.backend.unregister(&self.fullname).is_ok() {
            self.service_registered = false;
        }
        if self.backend.shutdown().is_ok() {
            self.service_registered = false;
            self.daemon_stopped = true;
        }
        error
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
            let _ = backend.unregister(&fullname);
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
        let local_addresses = if_addrs::get_if_addrs()
            .map_err(|error| {
                mdns_backend_error(
                    "interface_query",
                    format!("enumerate local network interfaces: {error}"),
                )
            })?
            .into_iter()
            .filter(|interface| interface.is_oper_up())
            .map(|interface| interface.ip())
            .collect::<Vec<_>>();
        Self::from_listener_with_local_addresses(
            identity,
            advertised_host,
            local_addr,
            &local_addresses,
        )
    }

    fn from_listener_with_local_addresses(
        identity: &RemoteHostIdentity,
        advertised_host: &str,
        local_addr: SocketAddr,
        local_addresses: &[IpAddr],
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
        if !local_addresses.contains(&address) {
            return Err(invalid_mdns_endpoint(
                "Remote LAN mDNS advertised IP is not assigned to an active local interface",
            ));
        }

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
    /// Returns only after the target fullname has been announced by the daemon.
    fn register(&mut self, service: ServiceInfo) -> Result<(), String>;
    fn health(&mut self) -> Result<(), String>;
    fn unregister(&mut self, fullname: &str) -> Result<(), String>;
    fn shutdown(&mut self) -> Result<(), String>;
}

struct ServiceDaemonBackend {
    daemon: ServiceDaemon,
    monitor: Receiver<DaemonEvent>,
}

impl ServiceDaemonBackend {
    fn new() -> HostResult<Self> {
        let daemon = ServiceDaemon::new().map_err(|error| {
            mdns_backend_error("start", format!("create service daemon: {error}"))
        })?;
        let monitor = match daemon.monitor() {
            Ok(monitor) => monitor,
            Err(error) => {
                if let Ok(receiver) = daemon.shutdown() {
                    let _ = receiver.recv_timeout(DAEMON_RESPONSE_TIMEOUT);
                }
                return Err(mdns_backend_error(
                    "start",
                    format!("create daemon monitor: {error}"),
                ));
            }
        };
        Ok(Self { daemon, monitor })
    }

    fn monitor_health(&mut self) -> Result<(), String> {
        loop {
            match self.monitor.try_recv() {
                Ok(DaemonEvent::Error(error)) => {
                    return Err(format!("daemon reported an error: {error}"));
                }
                Ok(_) => {}
                Err(TryRecvError::Empty) => return Ok(()),
                Err(TryRecvError::Disconnected) => {
                    return Err("daemon monitor disconnected because the daemon exited".to_string());
                }
            }
        }
    }

    fn wait_for_announce(&mut self, fullname: &str) -> Result<(), String> {
        let deadline = Instant::now() + ANNOUNCE_CONFIRMATION_TIMEOUT;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(format!(
                    "timed out waiting for daemon announcement of {fullname}"
                ));
            }
            match self.monitor.recv_timeout(remaining) {
                Ok(DaemonEvent::Announce(announced, _)) if announced == fullname => return Ok(()),
                Ok(DaemonEvent::Error(error)) => {
                    return Err(format!("daemon reported an error: {error}"));
                }
                Ok(_) => {}
                Err(RecvTimeoutError::Timeout) => {
                    return Err(format!(
                        "timed out waiting for daemon announcement of {fullname}"
                    ));
                }
                Err(RecvTimeoutError::Disconnected) => {
                    return Err("daemon monitor disconnected because the daemon exited".to_string());
                }
            }
        }
    }
}

impl MdnsBackend for ServiceDaemonBackend {
    fn register(&mut self, service: ServiceInfo) -> Result<(), String> {
        self.monitor_health()?;
        let fullname = service.get_fullname().to_string();
        self.daemon
            .register(service)
            .map_err(|error| error.to_string())?;
        self.wait_for_announce(&fullname)
    }

    fn health(&mut self) -> Result<(), String> {
        self.monitor_health()
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
    use std::collections::{BTreeMap, VecDeque};
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

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum RegisterOutcome {
        Announced,
        DaemonError,
        Timeout,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum IdleFailure {
        DaemonError,
        Disconnected,
    }

    #[derive(Debug, Default)]
    struct FakeBackendState {
        calls: Vec<BackendCall>,
        register_outcomes: VecDeque<RegisterOutcome>,
        idle_failure: Option<IdleFailure>,
        fail_unregister: bool,
    }

    struct FakeBackend {
        state: Arc<Mutex<FakeBackendState>>,
    }

    impl FakeBackend {
        fn new(state: Arc<Mutex<FakeBackendState>>) -> Self {
            Self { state }
        }
    }

    impl MdnsBackend for FakeBackend {
        fn register(&mut self, service: ServiceInfo) -> Result<(), String> {
            let mut state = self.state.lock().unwrap();
            state.calls.push(BackendCall::Register(service.into()));
            if let Some(failure) = state.idle_failure.take() {
                return Err(idle_failure_message(failure));
            }
            match state
                .register_outcomes
                .pop_front()
                .unwrap_or(RegisterOutcome::Announced)
            {
                RegisterOutcome::Announced => Ok(()),
                RegisterOutcome::DaemonError => {
                    Err("daemon reported an error before announce".to_string())
                }
                RegisterOutcome::Timeout => {
                    Err("timed out waiting for target fullname announce".to_string())
                }
            }
        }

        fn health(&mut self) -> Result<(), String> {
            match self.state.lock().unwrap().idle_failure.take() {
                Some(failure) => Err(idle_failure_message(failure)),
                None => Ok(()),
            }
        }

        fn unregister(&mut self, fullname: &str) -> Result<(), String> {
            let mut state = self.state.lock().unwrap();
            state
                .calls
                .push(BackendCall::Unregister(fullname.to_string()));
            if state.fail_unregister {
                Err("unregister failed".to_string())
            } else {
                Ok(())
            }
        }

        fn shutdown(&mut self) -> Result<(), String> {
            self.state.lock().unwrap().calls.push(BackendCall::Shutdown);
            Ok(())
        }
    }

    fn idle_failure_message(failure: IdleFailure) -> String {
        match failure {
            IdleFailure::DaemonError => "daemon reported an error while idle".to_string(),
            IdleFailure::Disconnected => {
                "daemon monitor disconnected because the daemon exited".to_string()
            }
        }
    }

    fn fake_backend(
        register_outcomes: impl IntoIterator<Item = RegisterOutcome>,
    ) -> (Box<dyn MdnsBackend>, Arc<Mutex<FakeBackendState>>) {
        let state = Arc::new(Mutex::new(FakeBackendState {
            register_outcomes: register_outcomes.into_iter().collect(),
            ..FakeBackendState::default()
        }));
        (Box::new(FakeBackend::new(state.clone())), state)
    }

    fn identity(device_id: &str, display_name: &str) -> RemoteHostIdentity {
        RemoteHostIdentity {
            device_id: device_id.to_string(),
            display_name: display_name.to_string(),
            identity_fingerprint: "must-not-leak-fingerprint".to_string(),
        }
    }

    fn service_spec(device_id: &str, display_name: &str) -> MdnsServiceSpec {
        let address = "192.168.1.23".parse::<IpAddr>().unwrap();
        MdnsServiceSpec::from_listener_with_local_addresses(
            &identity(device_id, display_name),
            "192.168.1.23",
            "0.0.0.0:43123".parse().unwrap(),
            &[address],
        )
        .unwrap()
    }

    #[test]
    fn advertiser_start_api_accepts_identity_provenance_only_from_listener_handle() {
        let _start: fn(
            &RemoteLanServerHandle,
            bool,
        ) -> HostResult<RemoteLanMdnsAdvertiser> = RemoteLanMdnsAdvertiser::start;
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
        let dns_error = MdnsServiceSpec::from_listener_with_local_addresses(
            &host,
            "listener.local",
            "0.0.0.0:43123".parse().unwrap(),
            &[],
        )
        .unwrap_err();
        assert_eq!(dns_error.code, "invalid_remote_lan_mdns_endpoint");

        let unspecified_error = MdnsServiceSpec::from_listener_with_local_addresses(
            &host,
            "0.0.0.0",
            "0.0.0.0:43123".parse().unwrap(),
            &[],
        )
        .unwrap_err();
        assert_eq!(
            unspecified_error.code,
            "invalid_remote_lan_mdns_endpoint"
        );

        let mismatch_error = MdnsServiceSpec::from_listener_with_local_addresses(
            &host,
            "192.168.1.23",
            "127.0.0.1:43123".parse().unwrap(),
            &["192.168.1.23".parse().unwrap()],
        )
        .unwrap_err();
        assert_eq!(mismatch_error.code, "invalid_remote_lan_mdns_endpoint");

        let nonlocal_error = MdnsServiceSpec::from_listener_with_local_addresses(
            &host,
            "192.168.1.23",
            "0.0.0.0:43123".parse().unwrap(),
            &["192.168.1.24".parse().unwrap()],
        )
        .unwrap_err();
        assert_eq!(nonlocal_error.code, "invalid_remote_lan_mdns_endpoint");

        let loopback_address = "127.0.0.1".parse::<IpAddr>().unwrap();
        let loopback = MdnsServiceSpec::from_listener_with_local_addresses(
            &host,
            "127.0.0.1",
            "127.0.0.1:43123".parse().unwrap(),
            &[loopback_address],
        )
        .unwrap();
        assert_eq!(loopback.address, loopback_address);

        let first = service_spec("device-alpha", &"客厅桌宠".repeat(20));
        let second = service_spec("device-beta", &"客厅桌宠".repeat(20));
        assert_ne!(first.instance_name, second.instance_name);
        assert_ne!(first.hostname, second.hostname);
        assert!(first.instance_name.len() <= INSTANCE_LABEL_MAX_BYTES);
        assert!(first.instance_name.starts_with("客厅桌宠"));
    }

    #[test]
    fn pairing_flip_reregisters_same_service_and_shutdown_is_idempotent() {
        let (backend, state) = fake_backend([
            RegisterOutcome::Announced,
            RegisterOutcome::Announced,
        ]);
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

        let state = state.lock().unwrap();
        let calls = &state.calls;
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
    fn announce_error_and_timeout_start_paths_clean_up_fail_closed() {
        for outcome in [RegisterOutcome::DaemonError, RegisterOutcome::Timeout] {
            let (backend, state) = fake_backend([outcome]);
            let error = RemoteLanMdnsAdvertiser::start_with_backend(
                service_spec("device-alpha", "Living Room"),
                false,
                backend,
            )
            .err()
            .unwrap();
            assert_eq!(error.code, "remote_lan_mdns_register_failed");
            assert!(matches!(
                state.lock().unwrap().calls.as_slice(),
                [
                    BackendCall::Register(_),
                    BackendCall::Unregister(_),
                    BackendCall::Shutdown
                ]
            ));
        }
    }

    #[test]
    fn update_announce_timeout_cleans_up_and_stops_the_advertiser() {
        let (backend, state) = fake_backend([
            RegisterOutcome::Announced,
            RegisterOutcome::Timeout,
        ]);
        let mut advertiser = RemoteLanMdnsAdvertiser::start_with_backend(
            service_spec("device-alpha", "Living Room"),
            false,
            backend,
        )
        .unwrap();

        let error = advertiser.update_pairing_available(true).unwrap_err();
        assert_eq!(error.code, "remote_lan_mdns_register_failed");
        advertiser.shutdown().unwrap();
        assert!(matches!(
            state.lock().unwrap().calls.as_slice(),
            [
                BackendCall::Register(_),
                BackendCall::Register(_),
                BackendCall::Unregister(_),
                BackendCall::Shutdown
            ]
        ));
    }

    #[test]
    fn idle_daemon_failure_is_observed_by_update_and_shutdown() {
        for (failure, observe_with_update) in [
            (IdleFailure::DaemonError, true),
            (IdleFailure::Disconnected, false),
        ] {
            let (backend, state) = fake_backend([RegisterOutcome::Announced]);
            let mut advertiser = RemoteLanMdnsAdvertiser::start_with_backend(
                service_spec("device-alpha", "Living Room"),
                false,
                backend,
            )
            .unwrap();
            state.lock().unwrap().idle_failure = Some(failure);

            let error = if observe_with_update {
                advertiser.update_pairing_available(false).unwrap_err()
            } else {
                advertiser.shutdown().unwrap_err()
            };
            assert_eq!(error.code, "remote_lan_mdns_monitor_failed");
            advertiser.shutdown().unwrap();
            assert!(matches!(
                state.lock().unwrap().calls.as_slice(),
                [
                    BackendCall::Register(_),
                    BackendCall::Unregister(_),
                    BackendCall::Shutdown
                ]
            ));
        }
    }

    #[test]
    fn unregister_failure_still_stops_the_daemon_and_repeated_shutdown_is_safe() {
        let (backend, state) = fake_backend([RegisterOutcome::Announced]);
        state.lock().unwrap().fail_unregister = true;
        let mut advertiser = RemoteLanMdnsAdvertiser::start_with_backend(
            service_spec("device-alpha", "Living Room"),
            false,
            backend,
        )
        .unwrap();
        let error = advertiser.shutdown().unwrap_err();
        assert_eq!(error.code, "remote_lan_mdns_unregister_failed");
        advertiser.shutdown().unwrap();
        assert!(matches!(
            state.lock().unwrap().calls.as_slice(),
            [
                BackendCall::Register(_),
                BackendCall::Unregister(_),
                BackendCall::Shutdown
            ]
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_real_backend_confirms_start_and_pair_update_announces_then_shutdown() {
        let device_id = format!("device-smoke-{}", uuid::Uuid::new_v4());
        let service = MdnsServiceSpec::from_listener(
            &identity(&device_id, "CodePet Smoke"),
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
