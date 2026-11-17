use serde::{Deserialize, Serialize};
use std::io;
use std::net::Ipv4Addr;

/// A network fingerprint saved during setup. SSIDs are raw bytes, not display text.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkIdentity {
    pub adapter_id: String,
    pub gateway: Ipv4Addr,
    pub ssid: Option<Vec<u8>>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Adapter {
    pub name: String,
    pub ipv4: Ipv4Addr,
    pub interface_index: u32,
    pub identity: NetworkIdentity,
}
fn match_adapter(expected: &NetworkIdentity, adapters: &[Adapter]) -> Option<NetworkConnection> {
    adapters
        .iter()
        .find(|adapter| adapter.identity == *expected)
        .map(|adapter| NetworkConnection {
            ipv4: adapter.ipv4,
            interface_index: adapter.interface_index,
        })
}

/// Enumerates connected physical Ethernet and Wi-Fi adapters with an IPv4 gateway.
/// A wireless adapter is omitted if Windows does not permit reading its SSID.
pub fn list_adapters() -> io::Result<Vec<Adapter>> {
    platform::list_adapters()
}

/// Connection details are refreshed every time; interface indices are not stable across boots.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NetworkConnection {
    pub ipv4: Ipv4Addr,
    pub interface_index: u32,
}

/// Verify identity and a route to the fixed campus portal through the selected interface.
pub fn matched_connection(expected: &NetworkIdentity) -> io::Result<Option<NetworkConnection>> {
    let Some(connection) = match_adapter(expected, &list_adapters()?) else {
        return Ok(None);
    };
    if platform::route_matches(connection, expected.gateway)? {
        Ok(Some(connection))
    } else {
        Ok(None)
    }
}

pub fn matched_ipv4(expected: &NetworkIdentity) -> io::Result<Option<Ipv4Addr>> {
    Ok(matched_connection(expected)?.map(|connection| connection.ipv4))
}

#[cfg(not(windows))]
mod platform {
    use super::*;
    pub fn route_matches(_: NetworkConnection, _: Ipv4Addr) -> io::Result<bool> {
        Ok(false)
    }
    pub fn list_adapters() -> io::Result<Vec<Adapter>> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Windows required",
        ))
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use std::{
        ffi::{c_void, CStr},
        mem::{size_of, zeroed},
        ptr::{null, null_mut},
    };
    use windows_sys::core::GUID;
    use windows_sys::Win32::NetworkManagement::IpHelper::{
        GetAdaptersAddresses, GetBestRoute2, GetIfEntry2, GAA_FLAG_INCLUDE_GATEWAYS,
        IP_ADAPTER_ADDRESSES_LH, MIB_IF_ROW2, MIB_IPFORWARD_ROW2,
    };
    use windows_sys::Win32::NetworkManagement::Ndis::IfOperStatusUp;
    use windows_sys::Win32::NetworkManagement::WiFi::{
        wlan_interface_state_connected, wlan_intf_opcode_current_connection, WlanCloseHandle,
        WlanFreeMemory, WlanOpenHandle, WlanQueryInterface, WLAN_CONNECTION_ATTRIBUTES,
    };
    use windows_sys::Win32::Networking::WinSock::{
        AF_INET, SOCKADDR_IN, SOCKADDR_INET, SOCKET_ADDRESS,
    };

    const ERROR_BUFFER_OVERFLOW: u32 = 111;
    const ETHERNET: u32 = 6;
    const WIFI: u32 = 71;

    unsafe fn ip(address: &SOCKET_ADDRESS) -> Option<Ipv4Addr> {
        if address.lpSockaddr.is_null()
            || address.iSockaddrLength < size_of::<SOCKADDR_IN>() as i32
            || (*address.lpSockaddr).sa_family != AF_INET
        {
            return None;
        }
        let addr = &*(address.lpSockaddr as *const SOCKADDR_IN);
        let bytes = addr.sin_addr.S_un.S_addr.to_ne_bytes();
        let result = Ipv4Addr::from(bytes);
        (!result.is_unspecified() && !result.is_loopback() && !result.is_link_local())
            .then_some(result)
    }

    unsafe fn wide(ptr: *const u16) -> String {
        if ptr.is_null() {
            return String::new();
        }
        let mut len = 0;
        while *ptr.add(len) != 0 {
            len += 1;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(ptr, len))
    }

    unsafe fn wifi_ssid(guid: &GUID) -> Option<Vec<u8>> {
        let mut version = 0;
        let mut handle = null_mut();
        if WlanOpenHandle(2, null(), &mut version, &mut handle) != 0 {
            return None;
        }
        let mut data = null_mut::<c_void>();
        let mut size = 0;
        let result = WlanQueryInterface(
            handle,
            guid,
            wlan_intf_opcode_current_connection,
            null(),
            &mut size,
            &mut data,
            null_mut(),
        );
        let value = if result == 0
            && !data.is_null()
            && size as usize >= size_of::<WLAN_CONNECTION_ATTRIBUTES>()
        {
            let connection = &*(data as *const WLAN_CONNECTION_ATTRIBUTES);
            let ssid = &connection.wlanAssociationAttributes.dot11Ssid;
            let len = ssid.uSSIDLength as usize;
            if connection.isState == wlan_interface_state_connected
                && len > 0
                && len <= ssid.ucSSID.len()
            {
                Some(ssid.ucSSID[..len].to_vec())
            } else {
                None
            }
        } else {
            None
        };
        if !data.is_null() {
            WlanFreeMemory(data);
        }
        WlanCloseHandle(handle, null());
        value
    }

    pub fn route_matches(connection: NetworkConnection, gateway: Ipv4Addr) -> io::Result<bool> {
        unsafe {
            if connection.interface_index == 0 {
                return Ok(false);
            }
            let mut destination: SOCKADDR_INET = zeroed();
            destination.Ipv4.sin_family = AF_INET;
            destination.Ipv4.sin_addr.S_un.S_addr = u32::from_ne_bytes([172, 30, 21, 100]);
            let mut route: MIB_IPFORWARD_ROW2 = zeroed();
            let mut source: SOCKADDR_INET = zeroed();
            let status = GetBestRoute2(
                null(),
                connection.interface_index,
                null(),
                &destination,
                0,
                &mut route,
                &mut source,
            );
            if status != 0 {
                return Ok(false);
            }
            if source.si_family != AF_INET || route.NextHop.si_family != AF_INET {
                return Ok(false);
            }
            let selected_source = Ipv4Addr::from(source.Ipv4.sin_addr.S_un.S_addr.to_ne_bytes());
            let next_hop = Ipv4Addr::from(route.NextHop.Ipv4.sin_addr.S_un.S_addr.to_ne_bytes());
            Ok(route_is_consistent(
                connection,
                gateway,
                route.InterfaceIndex,
                selected_source,
                next_hop,
            ))
        }
    }

    pub fn list_adapters() -> io::Result<Vec<Adapter>> {
        unsafe {
            let mut size = 15_000u32;
            let mut result;
            let mut storage;
            let mut attempts = 0;
            loop {
                // u64 ensures the buffer is suitably aligned for Windows structures.
                storage = vec![0u64; (size as usize + 7) / 8];
                result = GetAdaptersAddresses(
                    AF_INET as u32,
                    GAA_FLAG_INCLUDE_GATEWAYS,
                    null(),
                    storage.as_mut_ptr() as *mut IP_ADAPTER_ADDRESSES_LH,
                    &mut size,
                );
                if result != ERROR_BUFFER_OVERFLOW {
                    break;
                }
                attempts += 1;
                if attempts >= 3 || size > 1_000_000 {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "adapter list too large",
                    ));
                }
            }
            if result != 0 {
                return Err(io::Error::from_raw_os_error(result as i32));
            }
            let mut output = Vec::new();
            let mut current = storage.as_ptr() as *const IP_ADAPTER_ADDRESSES_LH;
            while !current.is_null() {
                let adapter = &*current;
                current = adapter.Next;
                if adapter.OperStatus != IfOperStatusUp
                    || (adapter.IfType != ETHERNET && adapter.IfType != WIFI)
                    || adapter.AdapterName.is_null()
                {
                    continue;
                }
                let mut row: MIB_IF_ROW2 = zeroed();
                row.InterfaceLuid = adapter.Luid;
                // HardwareInterface is bit 0 of InterfaceAndOperStatusFlags.
                if GetIfEntry2(&mut row) != 0 || row.InterfaceAndOperStatusFlags._bitfield & 1 == 0
                {
                    continue;
                }
                let ssid = if adapter.IfType == WIFI {
                    match wifi_ssid(&row.InterfaceGuid) {
                        Some(s) => Some(s),
                        None => continue,
                    }
                } else {
                    None
                };
                let mut unicast = adapter.FirstUnicastAddress;
                let mut locals = Vec::new();
                while !unicast.is_null() {
                    let item = &*unicast;
                    if item.DadState == 4 {
                        if let Some(address) = ip(&item.Address) {
                            locals.push(address);
                        }
                    }
                    unicast = item.Next;
                }
                let mut gateways = adapter.FirstGatewayAddress;
                let mut gateway_addresses = Vec::new();
                while !gateways.is_null() {
                    let item = &*gateways;
                    if let Some(address) = ip(&item.Address) {
                        gateway_addresses.push(address);
                    }
                    gateways = item.Next;
                }
                let local = only_one(&locals);
                let gateway = only_one(&gateway_addresses);
                if let (Some(ipv4), Some(gateway)) = (local, gateway) {
                    let adapter_id = CStr::from_ptr(adapter.AdapterName as *const i8)
                        .to_string_lossy()
                        .into_owned();
                    output.push(Adapter {
                        name: wide(adapter.FriendlyName),
                        ipv4,
                        interface_index: row.InterfaceIndex,
                        identity: NetworkIdentity {
                            adapter_id,
                            gateway,
                            ssid,
                        },
                    });
                }
            }
            Ok(output)
        }
    }
}

fn only_one(values: &[Ipv4Addr]) -> Option<Ipv4Addr> {
    match values {
        [value] => Some(*value),
        _ => None,
    }
}

fn route_is_consistent(
    connection: NetworkConnection,
    gateway: Ipv4Addr,
    route_index: u32,
    source: Ipv4Addr,
    next_hop: Ipv4Addr,
) -> bool {
    route_index == connection.interface_index
        && source == connection.ipv4
        && (next_hop.is_unspecified() || next_hop == gateway)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn adapter(id: &str, gateway: &str, ssid: Option<&[u8]>, ip: &str) -> Adapter {
        Adapter {
            name: id.into(),
            ipv4: ip.parse().unwrap(),
            interface_index: 1,
            identity: NetworkIdentity {
                adapter_id: id.into(),
                gateway: gateway.parse().unwrap(),
                ssid: ssid.map(|s| s.to_vec()),
            },
        }
    }
    #[test]
    fn refuses_ambiguous_addresses() {
        let first = "10.0.0.1".parse().unwrap();
        let second = "10.0.0.2".parse().unwrap();
        assert_eq!(only_one(&[first]), Some(first));
        assert_eq!(only_one(&[]), None);
        assert_eq!(only_one(&[first, second]), None);
    }
    #[test]
    fn route_must_use_selected_interface_source_and_gateway() {
        let connection = NetworkConnection {
            ipv4: "10.0.0.2".parse().unwrap(),
            interface_index: 3,
        };
        let gateway = "10.0.0.1".parse().unwrap();
        assert!(route_is_consistent(
            connection,
            gateway,
            3,
            connection.ipv4,
            gateway
        ));
        assert!(route_is_consistent(
            connection,
            gateway,
            3,
            connection.ipv4,
            Ipv4Addr::UNSPECIFIED
        ));
        assert!(!route_is_consistent(
            connection,
            gateway,
            4,
            connection.ipv4,
            gateway
        ));
        assert!(!route_is_consistent(
            connection,
            gateway,
            3,
            "10.0.0.3".parse().unwrap(),
            gateway
        ));
        assert!(!route_is_consistent(
            connection,
            gateway,
            3,
            connection.ipv4,
            "10.0.0.4".parse().unwrap()
        ));
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "reads local Windows adapter and routing state"]
    fn actual_adapter_enumeration_is_read_only() {
        let adapters = list_adapters().expect("adapter enumeration should succeed");
        for adapter in &adapters {
            assert!(adapter.interface_index > 0);
            let _ = matched_connection(&adapter.identity).expect("route query should not fail");
        }
        println!(
            "read-only adapter check completed; adapter count: {}",
            adapters.len()
        );
    }

    #[test]
    fn selects_only_original_adapter_and_network() {
        let saved = adapter("wifi", "10.0.0.1", Some(b"WHUT-ISP"), "10.0.0.5");
        let vpn = adapter("vpn", "10.0.0.1", Some(b"WHUT-ISP"), "10.0.0.7");
        let renewed = adapter("wifi", "10.0.0.1", Some(b"WHUT-ISP"), "10.0.0.9");
        assert_eq!(
            match_adapter(&saved.identity, &[vpn, renewed]),
            Some(NetworkConnection {
                ipv4: "10.0.0.9".parse().unwrap(),
                interface_index: 1
            })
        );
    }
    #[test]
    fn refuses_changed_gateway_or_wireless_network() {
        let saved = adapter("wifi", "10.0.0.1", Some(b"WHUT-ISP"), "10.0.0.5");
        let changed_gateway = adapter("wifi", "10.1.0.1", Some(b"WHUT-ISP"), "10.1.0.5");
        let changed_ssid = adapter("wifi", "10.0.0.1", Some(b"OTHER"), "10.0.0.5");
        let unknown_ssid = adapter("wifi", "10.0.0.1", None, "10.0.0.5");
        for network in [changed_gateway, changed_ssid, unknown_ssid] {
            assert_eq!(match_adapter(&saved.identity, &[network]), None);
        }
    }

    #[test]
    fn wifi_reconnection_refreshes_ip_and_interface_index_without_reconfiguring() {
        let saved = adapter("wifi", "10.0.0.1", Some(b"WHUT-ISP"), "10.0.0.5");
        assert_eq!(match_adapter(&saved.identity, &[]), None);
        let wired = adapter("ethernet", "10.0.0.1", None, "10.0.0.6");
        let mut reconnected = saved.clone();
        reconnected.ipv4 = "10.0.0.9".parse().unwrap();
        reconnected.interface_index = 42;
        assert_eq!(
            match_adapter(&saved.identity, &[wired, reconnected]),
            Some(NetworkConnection {
                ipv4: "10.0.0.9".parse().unwrap(),
                interface_index: 42,
            })
        );
    }

    #[test]
    fn wireless_identity_uses_raw_ssid_bytes_instead_of_lossy_display_names() {
        let saved = adapter("wifi", "10.0.0.1", Some(b"WHUT-ISP\xff"), "10.0.0.5");
        let other = adapter("wifi", "10.0.0.1", Some(b"WHUT-ISP\xfe"), "10.0.0.5");
        assert_eq!(
            String::from_utf8_lossy(saved.identity.ssid.as_ref().unwrap()),
            String::from_utf8_lossy(other.identity.ssid.as_ref().unwrap())
        );
        assert_eq!(match_adapter(&saved.identity, &[other]), None);
    }

    #[test]
    fn saved_wired_profile_does_not_silently_authorize_a_wireless_network() {
        let wired = adapter("ethernet", "10.0.0.1", None, "10.0.0.5");
        let wifi = adapter("wifi", "10.0.0.1", Some(b"WHUT-ISP"), "10.0.0.6");
        assert_eq!(match_adapter(&wired.identity, &[wifi.clone()]), None);
        assert!(match_adapter(&wifi.identity, &[wifi.clone()]).is_some());
    }

    #[test]
    fn wireless_identity_guard_blocks_password_posts_and_recovers_on_return() {
        use crate::controller::{Action, Controller};
        use crate::portal::{
            test_support::{LoginOutcome, MockPortal, RedialOutcome, TEST_PASSWORD, TEST_USERNAME},
            DialState, PortalError, PortalStatus,
        };
        use std::sync::{Arc, Mutex};
        use std::time::Instant;

        // The identity is synthetic. All HTTP traffic stays on loopback; no
        // Windows WLAN API, routing state, saved config or credential is used.
        let campus = adapter("test-wifi", "10.0.0.1", Some(b"WHUT-ISP"), "127.0.0.1");
        let expected = campus.identity.clone();
        let adapters = Arc::new(Mutex::new(vec![campus.clone()]));
        let current = Arc::clone(&adapters);
        let mock = MockPortal::new(
            PortalStatus::Offline,
            LoginOutcome::Success,
            RedialOutcome::SuccessAfterPending(0),
        );
        let client = mock.client_with_guard(move || {
            match_adapter(&expected, &current.lock().unwrap()).is_some()
        });
        let mut controller = Controller::new(Instant::now());

        for replacement in [
            None,
            Some(adapter(
                "test-wifi",
                "10.0.0.1",
                Some(b"OTHER"),
                "127.0.0.1",
            )),
            Some(adapter("test-wifi", "10.0.0.1", None, "127.0.0.1")),
            Some(adapter(
                "test-wifi",
                "10.1.0.1",
                Some(b"WHUT-ISP"),
                "127.0.0.1",
            )),
        ] {
            *adapters.lock().unwrap() = replacement.into_iter().collect();
            assert_eq!(
                client.login(TEST_USERNAME, TEST_PASSWORD),
                Err(PortalError::NetworkChanged)
            );
            assert_eq!(client.redial(), Err(PortalError::NetworkChanged));
        }
        assert!(mock
            .requests()
            .iter()
            .all(|request| !request.starts_with("POST ")));

        // Returning to the explicitly saved Wi-Fi uses the same login/redial
        // controller as wired Ethernet, without needing to save again.
        *adapters.lock().unwrap() = vec![campus];
        controller.network_changed(Instant::now());
        for _ in 0..4 {
            let now = controller.next();
            let action = controller.on_status(now, client.status());
            match action {
                Action::Login => controller.on_action_result(
                    now,
                    action,
                    client.login(TEST_USERNAME, TEST_PASSWORD).map(Some),
                ),
                Action::Redial => {
                    controller.on_action_result(now, action, client.redial().map(|()| None))
                }
                Action::None | Action::NetworkChanged => {}
            }
        }
        assert_eq!(
            client.status(),
            Ok(PortalStatus::Online(DialState::Connected))
        );
        assert_eq!(controller.message(), "校园网在线");
        assert_eq!(
            mock.requests()
                .iter()
                .filter(|r| r.as_str() == "POST /api/account/login")
                .count(),
            1
        );
        assert_eq!(
            mock.requests()
                .iter()
                .filter(|r| r.as_str() == "POST /api/account/redial")
                .count(),
            1
        );
    }
}
