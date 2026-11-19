use crate::network::NetworkConnection;
use std::{io, net::Ipv4Addr};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RouteTarget {
    pub connection: NetworkConnection,
    pub gateway: Ipv4Addr,
}

#[derive(Default)]
pub struct RoutePreference {
    #[cfg(any(windows, test))]
    state: Option<RouteState>,
}

impl RoutePreference {
    /// Prefer the first interface's IPv4 default route over the second one.
    /// An absent pair releases any route metric owned by the application.
    pub fn update(
        &mut self,
        preferred_and_backup: Option<(RouteTarget, RouteTarget)>,
    ) -> io::Result<bool> {
        #[cfg(windows)]
        {
            self.update_routes(preferred_and_backup, &mut windows::NativeRouteTable)
        }
        #[cfg(not(windows))]
        {
            match preferred_and_backup {
                None => Ok(false),
                Some(_) => Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "Windows required",
                )),
            }
        }
    }
}

#[cfg(any(windows, test))]
#[derive(Clone, Copy)]
enum RouteState {
    Owned {
        preferred: RouteKey,
        applied: DefaultRoute,
        original_metric: u32,
    },
    Yielded {
        preferred: RouteKey,
        backup: RouteKey,
    },
}

#[cfg(any(windows, test))]
impl RouteState {
    fn pair(self) -> (RouteKey, RouteKey) {
        match self {
            Self::Owned {
                preferred, applied, ..
            } => (preferred, applied.key),
            Self::Yielded { preferred, backup } => (preferred, backup),
        }
    }
}

#[cfg(any(windows, test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RouteKey {
    luid: u64,
    gateway: Ipv4Addr,
}

#[cfg(any(windows, test))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DefaultRoute {
    key: RouteKey,
    protocol: i32,
    origin: i32,
    metric: u32,
}

#[cfg(any(windows, test))]
#[derive(Clone, Copy)]
struct RoutedInterface {
    route: DefaultRoute,
    metric: u32,
}

#[cfg(any(windows, test))]
trait RouteTable {
    fn find(&self, target: RouteTarget) -> io::Result<Option<RoutedInterface>>;
    fn set_metric(&mut self, expected: DefaultRoute, metric: u32) -> io::Result<bool>;
}

#[cfg(any(windows, test))]
impl RoutePreference {
    fn restore(&mut self, table: &mut impl RouteTable) -> io::Result<()> {
        if let Some(RouteState::Owned {
            applied,
            original_metric,
            ..
        }) = self.state
        {
            table.set_metric(applied, original_metric)?;
        }
        self.state = None;
        Ok(())
    }

    fn update_routes(
        &mut self,
        pair: Option<(RouteTarget, RouteTarget)>,
        table: &mut impl RouteTable,
    ) -> io::Result<bool> {
        let Some((preferred_target, backup_target)) = pair else {
            self.restore(table)?;
            return Ok(false);
        };
        let Some((mut preferred, mut backup)) = find_pair(table, preferred_target, backup_target)?
        else {
            self.restore(table)?;
            return Ok(false);
        };
        if self
            .state
            .is_some_and(|state| state.pair() != (preferred.route.key, backup.route.key))
        {
            self.restore(table)?;
            // Either interface may have been the old backup; refresh after restoring it.
            let Some(routes) = find_pair(table, preferred_target, backup_target)? else {
                return Ok(false);
            };
            (preferred, backup) = routes;
        }

        let preferred_key = preferred.route.key;
        let yielded = RouteState::Yielded {
            preferred: preferred_key,
            backup: backup.route.key,
        };
        let preferred_metric = effective_metric(preferred.metric, preferred.route.metric)?;
        let original_metric = match self.state {
            Some(RouteState::Owned {
                applied,
                original_metric,
                ..
            }) if applied == backup.route => original_metric,
            Some(_) => {
                // Preserve somebody else's route edit for as long as this pair remains active.
                self.state = Some(yielded);
                return Ok(preferred_metric < effective_metric(backup.metric, backup.route.metric)?);
            }
            None => backup.route.metric,
        };
        let original_effective_metric = effective_metric(backup.metric, original_metric)?;
        let desired_metric = if original_effective_metric > preferred_metric {
            original_metric
        } else {
            checked_metric(preferred_metric.checked_add(1))? - backup.metric
        };

        if desired_metric == backup.route.metric {
            return Ok(true);
        }
        if !table.set_metric(backup.route, desired_metric)? {
            self.state = Some(yielded);
            return Ok(false);
        }
        self.state = (desired_metric != original_metric).then_some(RouteState::Owned {
            preferred: preferred_key,
            applied: DefaultRoute {
                metric: desired_metric,
                ..backup.route
            },
            original_metric,
        });
        Ok(true)
    }
}

#[cfg(any(windows, test))]
fn find_pair(
    table: &impl RouteTable,
    preferred: RouteTarget,
    backup: RouteTarget,
) -> io::Result<Option<(RoutedInterface, RoutedInterface)>> {
    let Some(preferred) = table.find(preferred)? else {
        return Ok(None);
    };
    Ok(table
        .find(backup)?
        .filter(|backup| preferred.route.key.luid != backup.route.key.luid)
        .map(|backup| (preferred, backup)))
}

#[cfg(any(windows, test))]
fn effective_metric(interface: u32, route: u32) -> io::Result<u32> {
    checked_metric(interface.checked_add(route))
}

#[cfg(any(windows, test))]
fn checked_metric(metric: Option<u32>) -> io::Result<u32> {
    metric
        .filter(|metric| *metric < u32::MAX)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid IPv4 route metric"))
}

#[cfg(windows)]
mod windows {
    use super::*;
    use std::mem::zeroed;
    use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_NOT_FOUND, NO_ERROR};
    use windows_sys::Win32::NetworkManagement::IpHelper::{
        GetIfEntry2, GetIpForwardEntry2, GetIpInterfaceEntry, GetUnicastIpAddressEntry,
        SetIpForwardEntry2, IF_TYPE_ETHERNET_CSMACD, IF_TYPE_IEEE80211, MIB_IF_ROW2,
        MIB_IPFORWARD_ROW2, MIB_IPINTERFACE_ROW, MIB_UNICASTIPADDRESS_ROW,
    };
    use windows_sys::Win32::NetworkManagement::Ndis::{IfOperStatusUp, NET_LUID_LH};
    use windows_sys::Win32::Networking::WinSock::{AF_INET, SOCKADDR_INET};

    pub struct NativeRouteTable;

    fn present(status: u32) -> io::Result<bool> {
        match status {
            NO_ERROR => Ok(true),
            ERROR_NOT_FOUND | ERROR_FILE_NOT_FOUND => Ok(false),
            _ => Err(io::Error::from_raw_os_error(status as i32)),
        }
    }

    fn address(ip: Ipv4Addr) -> SOCKADDR_INET {
        // SAFETY: the socket address consists of integer fields; only its initialized
        // IPv4 member is passed to IP Helper.
        let mut address: SOCKADDR_INET = unsafe { zeroed() };
        address.Ipv4.sin_family = AF_INET;
        address.Ipv4.sin_addr.S_un.S_addr = u32::from_ne_bytes(ip.octets());
        address
    }

    fn route_row(key: RouteKey) -> io::Result<Option<MIB_IPFORWARD_ROW2>> {
        // SAFETY: these native structures are plain data. Set the complete key
        // before querying; the API fills the remaining fields on success.
        let mut row: MIB_IPFORWARD_ROW2 = unsafe { zeroed() };
        row.InterfaceLuid = NET_LUID_LH { Value: key.luid };
        row.DestinationPrefix.Prefix = address(Ipv4Addr::UNSPECIFIED);
        row.NextHop = address(key.gateway);
        if !present(unsafe { GetIpForwardEntry2(&mut row) })? {
            return Ok(None);
        }
        Ok((row.ValidLifetime != 0 && row.Loopback == 0).then_some(row))
    }

    fn describe(row: &MIB_IPFORWARD_ROW2, key: RouteKey) -> DefaultRoute {
        DefaultRoute {
            key,
            protocol: row.Protocol,
            origin: row.Origin,
            metric: row.Metric,
        }
    }

    impl RouteTable for NativeRouteTable {
        fn find(&self, target: RouteTarget) -> io::Result<Option<RoutedInterface>> {
            if target.connection.interface_index == 0 || target.gateway.is_unspecified() {
                return Ok(None);
            }
            // SAFETY: all structures are zero-initialized plain data. Each read
            // receives its documented key and fills the rest on success.
            unsafe {
                let mut interface: MIB_IPINTERFACE_ROW = zeroed();
                interface.Family = AF_INET;
                interface.InterfaceIndex = target.connection.interface_index;
                if !present(GetIpInterfaceEntry(&mut interface))? || interface.Connected == 0 {
                    return Ok(None);
                }

                let mut link: MIB_IF_ROW2 = zeroed();
                link.InterfaceLuid = interface.InterfaceLuid;
                if !present(GetIfEntry2(&mut link))?
                    || link.OperStatus != IfOperStatusUp
                    || !matches!(link.Type, IF_TYPE_ETHERNET_CSMACD | IF_TYPE_IEEE80211)
                    || link.InterfaceAndOperStatusFlags._bitfield & 1 == 0
                {
                    return Ok(None);
                }

                let mut source: MIB_UNICASTIPADDRESS_ROW = zeroed();
                source.InterfaceLuid = interface.InterfaceLuid;
                source.Address = address(target.connection.ipv4);
                if !present(GetUnicastIpAddressEntry(&mut source))? {
                    return Ok(None);
                }

                let key = RouteKey {
                    luid: interface.InterfaceLuid.Value,
                    gateway: target.gateway,
                };
                Ok(route_row(key)?.map(|row| RoutedInterface {
                    route: describe(&row, key),
                    metric: interface.Metric,
                }))
            }
        }

        fn set_metric(&mut self, expected: DefaultRoute, metric: u32) -> io::Result<bool> {
            let Some(mut row) = route_row(expected.key)? else {
                return Ok(false);
            };
            if describe(&row, expected.key) != expected {
                return Ok(false);
            }
            row.Metric = metric;
            // SAFETY: this row was read using the exact default route key. Only
            // the metric field has changed and the pointer remains valid.
            present(unsafe { SetIpForwardEntry2(&row) })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(index: u8) -> RouteTarget {
        RouteTarget {
            connection: NetworkConnection {
                ipv4: Ipv4Addr::new(10, index, 0, 10),
                interface_index: index.into(),
            },
            gateway: Ipv4Addr::new(10, index, 0, 1),
        }
    }

    impl RoutedInterface {
        fn test_route(target: RouteTarget, interface_metric: u32, route_metric: u32) -> Self {
            Self {
                route: DefaultRoute {
                    key: RouteKey {
                        luid: target.connection.interface_index.into(),
                        gateway: target.gateway,
                    },
                    protocol: 3,
                    origin: 2,
                    metric: route_metric,
                },
                metric: interface_metric,
            }
        }
    }

    struct MemoryTable {
        routes: Vec<RoutedInterface>,
        writes: Vec<(u64, u32)>,
    }

    impl RouteTable for MemoryTable {
        fn find(&self, target: RouteTarget) -> io::Result<Option<RoutedInterface>> {
            Ok(self
                .routes
                .iter()
                .find(|entry| {
                    entry.route.key.luid == u64::from(target.connection.interface_index)
                        && entry.route.key.gateway == target.gateway
                })
                .copied())
        }

        fn set_metric(&mut self, expected: DefaultRoute, metric: u32) -> io::Result<bool> {
            let Some(entry) = self.routes.iter_mut().find(|entry| entry.route == expected) else {
                return Ok(false);
            };
            entry.route.metric = metric;
            self.writes.push((expected.key.luid, metric));
            Ok(true)
        }
    }

    fn table(wifi: RouteTarget, ethernet: RouteTarget) -> MemoryTable {
        MemoryTable {
            routes: vec![
                RoutedInterface::test_route(wifi, 35, 1),
                RoutedInterface::test_route(ethernet, 25, 0),
                RoutedInterface::test_route(target(18), 0, 0),
            ],
            writes: Vec::new(),
        }
    }

    #[test]
    fn prefers_isp_by_raising_only_ethernet_and_restores_on_disable() {
        let (wifi, ethernet) = (target(12), target(20));
        let mut table = table(wifi, ethernet);
        let mut preference = RoutePreference::default();
        let pair = Some((wifi, ethernet));
        assert!(preference.update_routes(pair, &mut table).unwrap());
        assert!(preference.update_routes(pair, &mut table).unwrap());
        assert_eq!(table.writes, [(20, 12)]);
        assert!(!preference.update_routes(None, &mut table).unwrap());
        assert_eq!(table.writes, [(20, 12), (20, 0)]);
    }

    #[test]
    fn falls_back_to_ethernet_and_restores_metric_when_isp_recovers() {
        let (wifi, ethernet) = (target(12), target(20));
        let mut table = table(wifi, ethernet);
        table.routes[1].metric = 36;
        let mut preference = RoutePreference::default();
        for pair in [(wifi, ethernet), (ethernet, wifi), (wifi, ethernet)] {
            assert!(preference.update_routes(Some(pair), &mut table).unwrap());
        }
        assert_eq!(table.writes, [(20, 1), (20, 0), (12, 2), (12, 1), (20, 1)]);
    }

    #[test]
    fn restores_backup_on_loss_of_preferred_route_and_never_touches_tunnel() {
        let (wifi, ethernet) = (target(12), target(20));
        let mut table = table(wifi, ethernet);
        let mut preference = RoutePreference::default();
        let pair = Some((wifi, ethernet));
        assert!(preference.update_routes(pair, &mut table).unwrap());
        table.routes.remove(0);
        assert!(!preference.update_routes(pair, &mut table).unwrap());
        assert_eq!(table.writes, [(20, 12), (20, 0)]);
    }

    #[test]
    fn leaves_an_external_metric_change_in_place() {
        let (wifi, ethernet) = (target(12), target(20));
        let mut table = table(wifi, ethernet);
        let mut preference = RoutePreference::default();
        let pair = Some((wifi, ethernet));
        assert!(preference.update_routes(pair, &mut table).unwrap());
        table.routes[1].route.metric = 3;
        assert!(!preference.update_routes(pair, &mut table).unwrap());
        assert!(!preference.update_routes(pair, &mut table).unwrap());
        assert!(!preference.update_routes(None, &mut table).unwrap());
        assert_eq!(table.routes[1].route.metric, 3);
        assert_eq!(table.writes, [(20, 12)]);
    }
}
