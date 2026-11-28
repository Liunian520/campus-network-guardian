use std::collections::VecDeque;
use std::fmt::Write;
use std::time::{Duration, Instant};

const TIMELINE_LIMIT: usize = 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Role {
    Primary,
    Secondary,
}

impl Role {
    pub(crate) const ALL: [Self; 2] = [Self::Primary, Self::Secondary];

    pub(crate) fn index(self) -> usize {
        match self {
            Self::Primary => 0,
            Self::Secondary => 1,
        }
    }

    fn other(self) -> Self {
        match self {
            Self::Primary => Self::Secondary,
            Self::Secondary => Self::Primary,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Primary => "主网络",
            Self::Secondary => "第二个网络",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Kind {
    Wifi,
    Ethernet,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum Link {
    #[default]
    Unknown,
    Matched,
    Unavailable,
    AdapterChanged,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum Auth {
    #[default]
    Unknown,
    Offline,
    Online,
    Unreachable,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum Carrier {
    #[default]
    Unknown,
    NotRequired,
    Dialing,
    Connected,
    Failed,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum External {
    #[default]
    NotChecked,
    Checking,
    Reachable,
    PortalRedirect,
    Inconclusive,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Update {
    Link(Link),
    Auth(Auth),
    Carrier(Carrier),
    External(External),
}

impl Update {
    fn field(self) -> &'static str {
        match self {
            Self::Link(_) => "网络接入",
            Self::Auth(_) => "校园网认证",
            Self::Carrier(_) => "运营商拨号",
            Self::External(_) => "IPv4 外网",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Link(Link::Unknown) | Self::Auth(Auth::Unknown) => "待检查",
            Self::Link(Link::Matched) => "已匹配",
            Self::Link(Link::Unavailable) => "未连接或不可用",
            Self::Link(Link::AdapterChanged) => "网卡已变化",
            Self::Auth(Auth::Offline) => "待认证",
            Self::Auth(Auth::Online) => "已认证",
            Self::Auth(Auth::Unreachable) | Self::External(External::Inconclusive) => {
                "暂时无法确认"
            }
            Self::Carrier(Carrier::Unknown) => "待确认",
            Self::Carrier(Carrier::NotRequired) => "无需拨号",
            Self::Carrier(Carrier::Dialing) => "正在拨号",
            Self::Carrier(Carrier::Connected) => "已拨号",
            Self::Carrier(Carrier::Failed) => "拨号失败",
            Self::External(External::NotChecked) => "未检测",
            Self::External(External::Checking) => "正在检测",
            Self::External(External::Reachable) => "可达",
            Self::External(External::PortalRedirect) => "返回校园网认证页",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Action {
    LoginStarted,
    LoginSucceeded,
    LoginFailed,
    LoginUnconfirmed,
    RedialStarted,
    RedialAccepted,
    RedialFailed,
    RedialUnconfirmed,
}

impl Action {
    fn label(self) -> &'static str {
        match self {
            Self::LoginStarted => "开始校园网认证",
            Self::LoginSucceeded => "校园网认证成功",
            Self::LoginFailed => "校园网认证失败",
            Self::LoginUnconfirmed => "校园网认证结果待确认",
            Self::RedialStarted => "开始运营商拨号",
            Self::RedialAccepted => "运营商拨号请求已受理",
            Self::RedialFailed => "运营商拨号失败",
            Self::RedialUnconfirmed => "运营商拨号结果待确认",
        }
    }
}

struct Network {
    kind: Kind,
    link: Link,
    auth: Auth,
    carrier: Carrier,
    external: External,
    last_external_observation: External,
    next_check: Option<Instant>,
}

impl Network {
    fn new(kind: Kind) -> Self {
        Self {
            kind,
            link: Link::default(),
            auth: Auth::default(),
            carrier: Carrier::default(),
            external: External::default(),
            last_external_observation: External::default(),
            next_check: None,
        }
    }

    fn update(&mut self, update: Update) -> bool {
        match update {
            Update::Link(value) => replace_if_changed(&mut self.link, value),
            Update::Auth(value) => replace_if_changed(&mut self.auth, value),
            Update::Carrier(value) => replace_if_changed(&mut self.carrier, value),
            Update::External(value) => {
                // Polling can redraw the current status without adding repeat results to history.
                self.external = value;
                value != External::Checking
                    && replace_if_changed(&mut self.last_external_observation, value)
            }
        }
    }

    fn states(&self) -> [Update; 4] {
        [
            Update::Link(self.link),
            Update::Auth(self.auth),
            Update::Carrier(self.carrier),
            Update::External(self.external),
        ]
    }

    fn next_check_label(&self, now: Instant) -> Option<String> {
        let deadline = self.next_check?;
        let remaining = deadline.saturating_duration_since(now);
        if remaining.is_zero() {
            return Some("即将开始".into());
        }
        let seconds = remaining
            .as_secs()
            .saturating_add(u64::from(remaining.subsec_nanos() != 0));
        Some(format!("约 {seconds} 秒后"))
    }
}

fn replace_if_changed<T: Copy + Eq>(slot: &mut T, value: T) -> bool {
    std::mem::replace(slot, value) != value
}

enum EventKind {
    Transition(Update),
    Action(Action),
}

struct Event {
    role: Role,
    time: Instant,
    kind: EventKind,
}

#[derive(Default)]
pub(crate) struct Diagnostics {
    networks: [Option<Network>; 2],
    timeline: VecDeque<Event>,
}

impl Diagnostics {
    pub(crate) fn configure(&mut self, networks: &[(Role, Kind)]) {
        self.networks = [None, None];
        self.timeline.clear();
        for &(role, kind) in networks {
            self.networks[role.index()] = Some(Network::new(kind));
        }
    }

    pub(crate) fn update(&mut self, role: Role, update: Update) {
        if self.networks[role.index()]
            .as_mut()
            .is_some_and(|network| network.update(update))
        {
            self.push_event(role, EventKind::Transition(update));
        }
    }

    pub(crate) fn record(&mut self, role: Role, action: Action) {
        if self.networks[role.index()].is_some() {
            self.push_event(role, EventKind::Action(action));
        }
    }

    fn push_event(&mut self, role: Role, kind: EventKind) {
        if self.timeline.len() == TIMELINE_LIMIT {
            self.timeline.pop_front();
        }
        self.timeline.push_back(Event {
            role,
            time: Instant::now(),
            kind,
        });
    }

    pub(crate) fn set_next_check(&mut self, role: Role, wait: Option<Duration>) {
        if let Some(network) = &mut self.networks[role.index()] {
            network.next_check = wait.and_then(|wait| Instant::now().checked_add(wait));
        }
    }

    fn network_label(&self, role: Role, kind: Kind) -> String {
        let label = match kind {
            Kind::Wifi => "Wi-Fi",
            Kind::Ethernet => "以太网",
        };
        if self.networks[role.other().index()]
            .as_ref()
            .is_some_and(|network| network.kind == kind)
        {
            return format!("{}（{label}）", role.label());
        }
        label.into()
    }

    pub(crate) fn summary(&self) -> String {
        let now = Instant::now();
        let mut summary = String::new();
        for role in Role::ALL {
            let Some(network) = &self.networks[role.index()] else {
                continue;
            };
            if !summary.is_empty() {
                summary.push_str("\r\n\r\n");
            }
            let label = self.network_label(role, network.kind);
            let [link, auth, carrier, external] = network.states();
            let _ = write!(
                summary,
                "{label}：{} {} · {} {}\r\n{} {} · {} {}",
                link.field(),
                link.label(),
                auth.field(),
                auth.label(),
                carrier.field(),
                carrier.label(),
                external.field(),
                external.label(),
            );
            if let Some(next_check) = network.next_check_label(now) {
                let _ = write!(summary, " · 下次检查 {next_check}");
            }
        }
        if summary.is_empty() {
            summary.push_str("尚未配置守护网络");
        }
        summary
    }

    pub(crate) fn report(&self) -> String {
        let now = Instant::now();
        let mut report = String::from("校园网守护诊断报告\r\n");
        if self.networks.iter().all(Option::is_none) {
            report.push_str("\r\n尚未配置守护网络\r\n");
        }
        for role in Role::ALL {
            let Some(network) = &self.networks[role.index()] else {
                continue;
            };
            let label = self.network_label(role, network.kind);
            let _ = write!(report, "\r\n{label}\r\n");
            for state in network.states() {
                let _ = write!(report, "  {}：{}\r\n", state.field(), state.label());
            }
            let next_check = network.next_check_label(now);
            let _ = write!(
                report,
                "  下次检查：{}\r\n",
                next_check.as_deref().unwrap_or("待安排")
            );
        }

        report.push_str("\r\n最近恢复记录\r\n");
        if self.timeline.is_empty() {
            report.push_str("  暂无记录\r\n");
        }
        for event in self.timeline.iter().rev() {
            let Some(network) = &self.networks[event.role.index()] else {
                continue;
            };
            let label = self.network_label(event.role, network.kind);
            let age = now.saturating_duration_since(event.time).as_secs();
            match age {
                0 => report.push_str("  刚刚 · "),
                seconds => {
                    let _ = write!(report, "  {seconds} 秒前 · ");
                }
            }
            match event.kind {
                EventKind::Transition(state) => {
                    let _ = write!(report, "{label} · {}：{}\r\n", state.field(), state.label());
                }
                EventKind::Action(action) => {
                    let _ = write!(report, "{label} · {}\r\n", action.label());
                }
            }
        }
        report
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_shows_independent_layers_for_each_configured_network() {
        let mut diagnostics = Diagnostics::default();
        diagnostics.configure(&[
            (Role::Primary, Kind::Wifi),
            (Role::Secondary, Kind::Ethernet),
        ]);
        diagnostics.update(Role::Primary, Update::Link(Link::Matched));
        diagnostics.update(Role::Primary, Update::Auth(Auth::Online));
        diagnostics.update(Role::Primary, Update::Carrier(Carrier::NotRequired));
        diagnostics.update(Role::Primary, Update::External(External::Inconclusive));
        diagnostics.update(Role::Secondary, Update::Link(Link::AdapterChanged));
        diagnostics.set_next_check(Role::Primary, Some(Duration::ZERO));

        assert_eq!(
            diagnostics.summary(),
            "Wi-Fi：网络接入 已匹配 · 校园网认证 已认证\r\n运营商拨号 无需拨号 · IPv4 外网 暂时无法确认 · 下次检查 即将开始\r\n\r\n以太网：网络接入 网卡已变化 · 校园网认证 待检查\r\n运营商拨号 待确认 · IPv4 外网 未检测"
        );
        let report = diagnostics.report();
        assert!(report.contains("Wi-Fi\r\n  网络接入：已匹配\r\n  校园网认证：已认证\r\n  运营商拨号：无需拨号\r\n  IPv4 外网：暂时无法确认\r\n  下次检查：即将开始"));
        assert!(report.contains("以太网\r\n  网络接入：网卡已变化\r\n  校园网认证：待检查\r\n  运营商拨号：待确认\r\n  IPv4 外网：未检测\r\n  下次检查：待安排"));
    }

    #[test]
    fn timeline_records_changes_instead_of_repeated_checks() {
        let mut diagnostics = Diagnostics::default();
        diagnostics.configure(&[(Role::Primary, Kind::Wifi)]);
        diagnostics.update(Role::Primary, Update::Link(Link::Matched));
        diagnostics.update(Role::Primary, Update::Link(Link::Matched));
        diagnostics.record(Role::Primary, Action::LoginStarted);

        let report = diagnostics.report();
        assert_eq!(report.matches("Wi-Fi · 网络接入：已匹配").count(), 1);
        let action = report.find("Wi-Fi · 开始校园网认证").unwrap();
        let transition = report.find("Wi-Fi · 网络接入：已匹配").unwrap();
        assert!(action < transition);
    }

    #[test]
    fn timeline_keeps_only_the_most_recent_thirty_two_entries() {
        let mut diagnostics = Diagnostics::default();
        diagnostics.configure(&[(Role::Primary, Kind::Wifi)]);
        diagnostics.record(Role::Primary, Action::LoginStarted);
        for _ in 0..32 {
            diagnostics.record(Role::Primary, Action::RedialStarted);
        }

        let report = diagnostics.report();
        assert_eq!(report.matches("Wi-Fi · 开始运营商拨号").count(), 32);
        assert!(!report.contains("开始校园网认证"));
    }

    #[test]
    fn configure_clears_old_observations_even_when_the_network_kind_is_unchanged() {
        let mut diagnostics = Diagnostics::default();
        diagnostics.configure(&[(Role::Primary, Kind::Wifi)]);
        diagnostics.update(Role::Primary, Update::External(External::Reachable));
        diagnostics.set_next_check(Role::Primary, Some(Duration::ZERO));
        diagnostics.configure(&[(Role::Primary, Kind::Wifi)]);
        diagnostics.record(Role::Secondary, Action::LoginFailed);

        let report = diagnostics.report();
        assert!(report.contains("IPv4 外网：未检测\r\n  下次检查：待安排"));
        assert!(report.contains("最近恢复记录\r\n  暂无记录"));
        assert!(!report.contains("校园网认证失败"));
    }

    #[test]
    fn report_distinguishes_networks_with_the_same_kind() {
        let mut diagnostics = Diagnostics::default();
        diagnostics.configure(&[(Role::Primary, Kind::Wifi), (Role::Secondary, Kind::Wifi)]);
        diagnostics.update(Role::Primary, Update::External(External::Reachable));
        diagnostics.update(Role::Secondary, Update::External(External::PortalRedirect));

        let report = diagnostics.report();
        assert!(report.contains("主网络（Wi-Fi） · IPv4 外网：可达"));
        assert!(report.contains("第二个网络（Wi-Fi） · IPv4 外网：返回校园网认证页"));
    }

    #[test]
    fn repeating_a_probe_keeps_its_previous_observation_in_the_timeline() {
        let mut diagnostics = Diagnostics::default();
        diagnostics.configure(&[(Role::Primary, Kind::Wifi)]);
        diagnostics.update(Role::Primary, Update::External(External::Reachable));
        diagnostics.update(Role::Primary, Update::External(External::Checking));
        assert!(diagnostics.summary().contains("IPv4 外网 正在检测"));
        diagnostics.update(Role::Primary, Update::External(External::Reachable));

        let report = diagnostics.report();
        assert_eq!(report.matches("Wi-Fi · IPv4 外网：可达").count(), 1);
        assert!(!report.contains("Wi-Fi · IPv4 外网：正在检测"));
    }
}
