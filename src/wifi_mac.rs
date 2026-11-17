//! Per-profile Wi-Fi MAC preparation. No driver overrides or forced reconnects.
use crate::network::{NetworkConnection, NetworkIdentity};
mod profile;
#[cfg(windows)]
mod windows;

#[derive(Clone, PartialEq, Eq)]
pub struct Target {
    adapter_id: String,
    interface_index: u32,
    profile: Vec<u16>,
    ssid: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Query,
    Permission,
    Managed,
    CustomData,
    UnknownMac,
    Changed,
    Write,
    Verify,
}

#[derive(Clone)]
struct Snapshot {
    target: Target,
    current: [u8; 6],
    permanent: [u8; 6],
    disabled: bool,
    managed: bool,
    custom_data: bool,
}

trait Backend {
    fn inspect(&mut self) -> Result<Snapshot, Error>;
    fn disable(&mut self, expected: &Snapshot) -> Result<(), Error>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Readiness {
    Ready,
    WaitingForReconnect,
    Manual(Error),
}

#[derive(Default)]
pub struct Guard {
    failed: Option<(Target, Error)>,
    proof: Option<Target>,
}

impl Guard {
    pub fn proof(&self) -> Option<Target> {
        self.proof.clone()
    }

    pub fn prepare(
        &mut self,
        identity: &NetworkIdentity,
        connection: NetworkConnection,
    ) -> Readiness {
        self.proof = None;
        if identity.ssid.is_none() {
            return Readiness::Ready;
        }
        #[cfg(windows)]
        {
            self.prepare_with(&mut windows::Native {
                identity,
                connection,
            })
        }
        #[cfg(not(windows))]
        {
            let _ = connection;
            Readiness::Manual(Error::Query)
        }
    }

    fn prepare_with(&mut self, backend: &mut impl Backend) -> Readiness {
        self.proof = None;
        let mut snapshot = match backend.inspect() {
            Ok(snapshot) => snapshot,
            Err(error) => return Readiness::Manual(error),
        };
        if !valid_mac(snapshot.current) || !valid_mac(snapshot.permanent) {
            return Readiness::Manual(Error::UnknownMac);
        }
        if !snapshot.disabled {
            if snapshot.managed {
                return Readiness::Manual(Error::Managed);
            }
            if snapshot.custom_data {
                return Readiness::Manual(Error::CustomData);
            }
            if let Some((target, error)) = &self.failed {
                if *target == snapshot.target {
                    return Readiness::Manual(*error);
                }
            }
            if let Err(error) = backend.disable(&snapshot) {
                self.failed = Some((snapshot.target, error));
                return Readiness::Manual(error);
            }
            let verified = match backend.inspect() {
                Ok(verified) if verified.target == snapshot.target && verified.disabled => verified,
                _ => {
                    self.failed = Some((snapshot.target, Error::Verify));
                    return Readiness::Manual(Error::Verify);
                }
            };
            snapshot = verified;
        }
        if !snapshot_ready(&snapshot) {
            return Readiness::WaitingForReconnect;
        }
        self.failed = None;
        self.proof = Some(snapshot.target);
        Readiness::Ready
    }
}

fn valid_mac(mac: [u8; 6]) -> bool {
    mac != [0; 6] && mac[0] & 1 == 0
}

fn snapshot_ready(snapshot: &Snapshot) -> bool {
    snapshot.disabled && valid_mac(snapshot.permanent) && snapshot.current == snapshot.permanent
}

fn proof_matches(snapshot: Result<Snapshot, Error>, proof: &Option<Target>) -> bool {
    snapshot
        .is_ok_and(|snapshot| snapshot_ready(&snapshot) && proof.as_ref() == Some(&snapshot.target))
}

/// Re-read immediately before each authenticated POST; never trust a cached Ready bit.
pub fn authentication_ready(
    identity: &NetworkIdentity,
    connection: NetworkConnection,
    proof: &Option<Target>,
) -> bool {
    if identity.ssid.is_none() {
        return proof.is_none();
    }
    #[cfg(windows)]
    {
        proof_matches(
            windows::Native {
                identity,
                connection,
            }
            .inspect(),
            proof,
        )
    }
    #[cfg(not(windows))]
    {
        let _ = (connection, proof);
        false
    }
}

impl Readiness {
    pub fn message(self) -> &'static str {
        match self {
            Self::Ready => "校园 Wi-Fi 固定 MAC 已确认",
            Self::WaitingForReconnect => "校园 Wi-Fi 随机 MAC 已关闭；等待下次连接后确认固定地址。自动认证暂缓，不主动断开当前网络。",
            Self::Manual(Error::Permission) => "需要人工处理：无权限检查或关闭校园 Wi-Fi 随机 MAC，请在 Windows 的此网络属性中关闭随机硬件地址。",
            Self::Manual(Error::Managed) => "需要人工处理：校园 Wi-Fi 配置受策略管理，无法自动关闭随机 MAC，请联系网络管理员。",
            Self::Manual(Error::CustomData) => "需要人工处理：校园 Wi-Fi 配置含附加用户数据，未自动覆盖；请在 Windows 的此网络属性中关闭随机硬件地址。",
            Self::Manual(Error::UnknownMac) => "需要人工处理：无法确认 Wi-Fi 的有效硬件地址，自动认证已暂停。",
            Self::Manual(Error::Changed) => "网络或 Wi-Fi 配置已变化，正在等待重新确认固定 MAC。",
            Self::Manual(Error::Write | Error::Verify) => "需要人工处理：校园 Wi-Fi 随机 MAC 设置未能确认关闭，请在 Windows 的此网络属性中关闭后重新检查。",
            Self::Manual(Error::Query) => "需要人工处理：无法检查校园 Wi-Fi 配置或 MAC，请检查 Windows 位置权限和无线网卡状态。",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake {
        snapshot: Snapshot,
        writes: usize,
        error: Option<Error>,
        ignore_write: bool,
    }
    impl Backend for Fake {
        fn inspect(&mut self) -> Result<Snapshot, Error> {
            Ok(self.snapshot.clone())
        }
        fn disable(&mut self, expected: &Snapshot) -> Result<(), Error> {
            assert!(expected.target == self.snapshot.target);
            self.writes += 1;
            if let Some(error) = self.error {
                return Err(error);
            }
            if !self.ignore_write {
                self.snapshot.disabled = true;
            }
            Ok(())
        }
    }
    fn fake() -> Fake {
        Fake {
            snapshot: Snapshot {
                target: Target {
                    adapter_id: "fake-guid".into(),
                    interface_index: 8,
                    profile: "Campus actual profile".encode_utf16().collect(),
                    ssid: b"WHUT-ISP".to_vec(),
                },
                current: [2, 1, 2, 3, 4, 5],
                permanent: [0, 1, 2, 3, 4, 5],
                disabled: false,
                managed: false,
                custom_data: false,
            },
            writes: 0,
            error: None,
            ignore_write: false,
        }
    }

    #[test]
    fn random_mac_is_disabled_once_then_auth_waits_for_hardware_mac() {
        let mut backend = fake();
        let mut guard = Guard::default();
        assert_eq!(
            guard.prepare_with(&mut backend),
            Readiness::WaitingForReconnect
        );
        assert_eq!(backend.writes, 1);
        assert!(guard.proof.is_none());
        assert_eq!(
            guard.prepare_with(&mut backend),
            Readiness::WaitingForReconnect
        );
        assert_eq!(backend.writes, 1);
        // A natural reconnect can retain its IP and interface index.
        backend.snapshot.current = backend.snapshot.permanent;
        assert_eq!(guard.prepare_with(&mut backend), Readiness::Ready);
        assert!(guard.proof.is_some());
    }

    #[test]
    fn fixed_profile_is_a_noop_and_managed_profiles_can_be_readied_without_writes() {
        let mut backend = fake();
        backend.snapshot.disabled = true;
        backend.snapshot.current = backend.snapshot.permanent;
        backend.snapshot.managed = true;
        backend.snapshot.custom_data = true;
        let mut guard = Guard::default();
        assert_eq!(guard.prepare_with(&mut backend), Readiness::Ready);
        assert_eq!(backend.writes, 0);
        assert!(guard.proof.is_some());
    }

    #[test]
    fn permission_failure_does_not_retry_writes_on_every_poll() {
        let mut backend = fake();
        backend.error = Some(Error::Permission);
        let mut guard = Guard::default();
        for _ in 0..20 {
            assert_eq!(
                guard.prepare_with(&mut backend),
                Readiness::Manual(Error::Permission)
            );
        }
        assert_eq!(backend.writes, 1);
        backend.snapshot.disabled = true;
        backend.snapshot.current = backend.snapshot.permanent;
        assert_eq!(guard.prepare_with(&mut backend), Readiness::Ready);
    }

    #[test]
    fn managed_and_custom_data_profiles_are_never_overwritten() {
        for custom in [false, true] {
            let mut backend = fake();
            backend.snapshot.managed = !custom;
            backend.snapshot.custom_data = custom;
            assert_eq!(
                Guard::default().prepare_with(&mut backend),
                Readiness::Manual(if custom {
                    Error::CustomData
                } else {
                    Error::Managed
                })
            );
            assert_eq!(backend.writes, 0);
        }
    }

    #[test]
    fn successful_write_must_be_read_back_before_authentication() {
        let mut backend = fake();
        backend.ignore_write = true;
        backend.snapshot.current = backend.snapshot.permanent;
        assert_eq!(
            Guard::default().prepare_with(&mut backend),
            Readiness::Manual(Error::Verify)
        );
    }

    #[test]
    fn invalid_hardware_mac_never_allows_authentication_or_profile_write() {
        for mac in [[0; 6], [255; 6], [1, 2, 3, 4, 5, 6]] {
            let mut backend = fake();
            backend.snapshot.permanent = mac;
            backend.snapshot.current = mac;
            assert_eq!(
                Guard::default().prepare_with(&mut backend),
                Readiness::Manual(Error::UnknownMac)
            );
            assert_eq!(backend.writes, 0);
        }
    }

    #[test]
    fn final_mac_guard_blocks_real_loopback_posts_when_profile_or_address_changes() {
        use crate::portal::{
            test_support::{LoginOutcome, MockPortal, RedialOutcome, TEST_PASSWORD, TEST_USERNAME},
            PortalError, PortalStatus,
        };
        use std::sync::{Arc, Mutex};
        let mut backend = fake();
        backend.snapshot.disabled = true;
        backend.snapshot.current = backend.snapshot.permanent;
        let mut guard = Guard::default();
        assert_eq!(guard.prepare_with(&mut backend), Readiness::Ready);
        let proof = guard.proof();
        let state = Arc::new(Mutex::new(Ok(backend.snapshot.clone())));
        let latest = state.clone();
        let portal = MockPortal::new(
            PortalStatus::Offline,
            LoginOutcome::Success,
            RedialOutcome::SuccessAfterPending(0),
        );
        let client =
            portal.client_with_guard(move || proof_matches(latest.lock().unwrap().clone(), &proof));
        for mode in 0..4 {
            let mut changed = backend.snapshot.clone();
            match mode {
                0 => changed.target.profile = "Other profile".encode_utf16().collect(),
                1 => changed.current = [2, 8, 8, 8, 8, 8],
                2 => changed.disabled = false,
                _ => {}
            }
            *state.lock().unwrap() = if mode == 3 {
                Err(Error::Permission)
            } else {
                Ok(changed)
            };
            assert_eq!(
                client.login(TEST_USERNAME, TEST_PASSWORD),
                Err(PortalError::NetworkChanged)
            );
            assert_eq!(client.redial(), Err(PortalError::NetworkChanged));
        }
        assert!(portal.requests().iter().all(|r| !r.starts_with("POST ")));
        *state.lock().unwrap() = Ok(backend.snapshot);
        assert!(client.login(TEST_USERNAME, TEST_PASSWORD).is_ok());
        assert!(client.redial().is_ok());
        for path in ["POST /api/account/login", "POST /api/account/redial"] {
            assert_eq!(
                portal
                    .requests()
                    .iter()
                    .filter(|r| r.as_str() == path)
                    .count(),
                1
            );
        }
    }

    #[test]
    fn wired_configuration_never_enters_the_wifi_backend() {
        let identity = NetworkIdentity {
            adapter_id: "fake-wired".into(),
            gateway: "10.0.0.1".parse().unwrap(),
            ssid: None,
        };
        let connection = NetworkConnection {
            ipv4: "10.0.0.2".parse().unwrap(),
            interface_index: 999_999,
        };
        let mut guard = Guard::default();
        assert_eq!(guard.prepare(&identity, connection), Readiness::Ready);
        assert!(authentication_ready(&identity, connection, &guard.proof()));
    }
}
