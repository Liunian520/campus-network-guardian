use crate::portal::{AuthFailure, DialState, PortalError, PortalStatus};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    None,
    Login,
    Redial,
    NetworkChanged,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Hold {
    None,
    Manual,
    Login,
    Configuration,
}

pub struct Controller {
    next: Instant,
    retry_until: Option<Instant>,
    failures: u8,
    hold: Hold,
    dial_poll: Option<u8>,
    uncertain_action: Option<(Action, Instant)>,
    message: &'static str,
    hold_message: Option<&'static str>,
}
impl Controller {
    pub fn new(now: Instant) -> Self {
        Self {
            next: now,
            retry_until: None,
            failures: 0,
            hold: Hold::None,
            dial_poll: None,
            uncertain_action: None,
            message: "正在启动",
            hold_message: None,
        }
    }
    pub fn due(&self, now: Instant) -> bool {
        now >= self.next
    }
    pub fn next(&self) -> Instant {
        self.next
    }
    /// Whether saved settings permit any automatic operating-system changes.
    pub fn configuration_available(&self) -> bool {
        self.hold != Hold::Configuration
    }
    pub fn message(&self) -> &'static str {
        if self.hold == Hold::None {
            self.message
        } else {
            self.hold_message.unwrap_or(self.message)
        }
    }
    pub fn network_changed(&mut self, now: Instant) {
        self.next = now + Duration::from_secs(5);
        self.message = "网络已变化，正在等待网络稳定";
    }
    pub fn configured(&mut self, now: Instant) {
        self.hold = Hold::None;
        self.hold_message = None;
        self.failures = 0;
        self.retry_until = None;
        self.dial_poll = None;
        self.next = now;
    }
    pub fn credentials_deleted(&mut self) {
        self.hold = Hold::Configuration;
        self.dial_poll = None;
        self.message = "密码不可用，自动操作已暂停，请打开设置";
        self.hold_message = Some(self.message);
    }
    pub fn check_now(&mut self, now: Instant) {
        self.next = now;
    }
    fn allowed(&self, now: Instant) -> bool {
        self.retry_until.is_none_or(|until| now >= until)
    }
    fn defer(&mut self, now: Instant, message: &'static str) {
        if !self.allowed(now) {
            self.message = "重试冷却中，暂不执行认证或重拨";
            return;
        }
        self.failures = self.failures.saturating_add(1);
        let secs = match self.failures {
            1 => 15,
            2 => 30,
            3 => 60,
            4 => 120,
            _ => 300,
        };
        self.next = now + Duration::from_secs(secs);
        self.retry_until = Some(self.next);
        self.message = if secs == 300 {
            "连续失败，将在 5 分钟后自动重试"
        } else {
            message
        };
    }
    pub fn on_action_result(
        &mut self,
        now: Instant,
        action: Action,
        result: Result<Option<PortalStatus>, PortalError>,
    ) {
        let previous_hold = self.hold;
        match result {
            Ok(Some(status @ PortalStatus::Online(DialState::Connected | DialState::Disabled)))
                if action == Action::Login =>
            {
                let _ = self.on_status(now, Ok(status));
            }
            Ok(Some(PortalStatus::Online(DialState::Failed))) if action == Action::Login => {
                self.dial_poll = None;
                self.next = now + Duration::from_secs(4);
                self.message = "校园认证成功，等待检查运营商拨号";
            }
            Ok(Some(PortalStatus::Offline)) if action == Action::Login => {
                self.defer(now, "认证后仍未在线，将延迟重试")
            }
            Ok(Some(PortalStatus::Online(DialState::Pending))) if action == Action::Login => {
                self.dial_poll = Some(15);
                self.next = now + Duration::from_secs(4);
                self.message = "正在等待认证或运营商拨号结果";
            }
            Ok(None) if action == Action::Redial => {
                self.dial_poll = Some(15);
                self.next = now + Duration::from_secs(4);
                self.message = "正在等待认证或运营商拨号结果";
            }
            Ok(_) => {
                self.hold = Hold::Configuration;
                self.message = "认证响应无法识别，自动操作已暂停，请检查设置";
            }
            Err(PortalError::Network) => {
                if matches!(action, Action::Login | Action::Redial) {
                    // A response timeout does not mean the server rejected the POST.
                    // Observe the old state for a full minute before retrying it.
                    self.uncertain_action = Some((action, now + Duration::from_secs(60)));
                }
                self.defer(now, "请求结果未知，先观察门户状态");
            }
            Err(PortalError::DiscoveryUnavailable) => {
                self.defer(now, "校园认证入口暂不可达，将延迟重试");
            }
            Err(PortalError::NetworkChanged) => {
                if matches!(action, Action::Login | Action::Redial) {
                    self.uncertain_action = Some((action, now + Duration::from_secs(60)));
                }
                self.network_changed(now);
            }
            Err(PortalError::ManualAction) => {
                self.hold = Hold::Login;
                self.message = "账号或密码需要人工处理，请检查并重新保存设置";
            }
            Err(PortalError::ContextUnknown) => {
                self.hold = Hold::Login;
                self.message = "校园认证参数无法识别，自动登录已暂停，请检查网络";
            }
            Err(PortalError::Captcha) => {
                self.hold = Hold::Manual;
                self.message = "需要人工处理验证码，请打开认证页面";
            }
            Err(PortalError::CheckFailed(reason) | PortalError::LoginFailed(reason)) => {
                self.hold = Hold::Login;
                self.message = match (result, reason) {
                    (Err(PortalError::CheckFailed(_)), AuthFailure::Password) => {
                        "登录预检提示密码有误，请检查并重新保存设置"
                    }
                    (Err(PortalError::CheckFailed(_)), AuthFailure::Account) => {
                        "登录预检提示账号异常，请打开认证页面检查"
                    }
                    (Err(PortalError::CheckFailed(_)), AuthFailure::Parameters) => {
                        "登录预检提示认证参数异常，请打开认证页面检查"
                    }
                    (Err(PortalError::CheckFailed(_)), AuthFailure::Unknown) => {
                        "登录预检未通过，请打开认证页面检查"
                    }
                    (_, AuthFailure::Password) => "门户提示密码有误，请检查并重新保存设置",
                    (_, AuthFailure::Account) => "门户提示账号异常，请打开认证页面检查",
                    (_, AuthFailure::Parameters) => "门户提示认证参数异常，请打开认证页面检查",
                    (_, AuthFailure::Unknown) => "门户未接受登录，请打开认证页面检查",
                };
            }
            Err(PortalError::AlreadyOnline) => {
                // Defensive fallback if this signal ever reaches the controller.
                self.uncertain_action = Some((Action::Login, now + Duration::from_secs(60)));
                self.defer(now, "门户提示已在线，正在复核状态");
            }
            Err(PortalError::Unexpected) if action == Action::Login => {
                self.hold = Hold::Login;
                self.message = "登录响应无法识别，自动登录已暂停，请检查并重新保存设置";
            }
            Err(PortalError::Unexpected) => {
                if action == Action::Redial {
                    self.uncertain_action = Some((action, now + Duration::from_secs(60)));
                }
                self.defer(now, "门户响应异常，将自动重试");
            }
        }
        if self.hold != previous_hold {
            self.hold_message = (self.hold != Hold::None).then_some(self.message);
        }
    }
    pub fn on_status(&mut self, now: Instant, result: Result<PortalStatus, PortalError>) -> Action {
        self.next = now + Duration::from_secs(60);
        if let (Some((action, deadline)), Ok(state)) = (self.uncertain_action, result) {
            let still_old = matches!(
                (action, state),
                (Action::Login, PortalStatus::Offline)
                    | (
                        Action::Redial,
                        PortalStatus::Online(DialState::Failed) | PortalStatus::Offline
                    )
            );
            if still_old && now < deadline {
                self.next = (now + Duration::from_secs(4)).min(deadline);
                self.message = "请求结果未知，正在等待门户更新状态";
                return Action::None;
            }
            if now >= deadline || !still_old {
                self.uncertain_action = None;
            }
        }
        match result {
            Ok(PortalStatus::Online(state))
                if matches!(state, DialState::Connected | DialState::Disabled) =>
            {
                if self.hold == Hold::Manual {
                    self.hold = Hold::None;
                }
                if self.allowed(now) {
                    self.failures = 0;
                    self.retry_until = None;
                }
                self.dial_poll = None;
                self.message = if self.hold == Hold::None {
                    "校园网在线"
                } else {
                    "校园网在线；自动操作已暂停，请检查设置"
                };
                Action::None
            }
            Ok(PortalStatus::Online(DialState::Pending)) => {
                if self.hold == Hold::Manual {
                    self.hold = Hold::None;
                }
                self.message = "运营商正在拨号";
                let remaining = self.dial_poll.get_or_insert(15);
                *remaining = remaining.saturating_sub(1);
                if *remaining > 0 {
                    self.next = now + Duration::from_secs(4);
                } else {
                    self.dial_poll = None;
                    self.defer(now, "运营商拨号超时，稍后检查");
                }
                Action::None
            }
            Ok(state @ (PortalStatus::Offline | PortalStatus::Online(DialState::Failed))) => {
                if matches!(state, PortalStatus::Online(_)) && self.hold == Hold::Manual {
                    self.hold = Hold::None;
                }
                if self.dial_poll.take().is_some() {
                    self.defer(now, "拨号未成功，将延迟重试");
                    return Action::None;
                }
                if self.hold == Hold::Configuration
                    || (self.hold != Hold::None && state == PortalStatus::Offline)
                {
                    return Action::None;
                }
                if !self.allowed(now) {
                    self.message = "重试冷却中，暂不执行认证或重拨";
                    return Action::None;
                }
                if state == PortalStatus::Offline {
                    self.message = "校园认证失效，正在登录";
                    Action::Login
                } else {
                    self.message = "运营商拨号失败，正在重拨";
                    Action::Redial
                }
            }
            Ok(PortalStatus::Online(_)) => unreachable!(),
            Err(PortalError::NetworkChanged) => {
                self.network_changed(now);
                Action::NetworkChanged
            }
            Err(error) => {
                self.defer(
                    now,
                    if error == PortalError::Network {
                        "认证门户不可达，将延迟检查"
                    } else {
                        "门户响应异常，将自动重试"
                    },
                );
                Action::None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn login_failure_reason_survives_status_and_network_changes() {
        let now = Instant::now();
        let mut c = Controller::new(now);
        c.on_action_result(
            now,
            Action::Login,
            Err(PortalError::LoginFailed(AuthFailure::Password)),
        );
        let reason = "门户提示密码有误，请检查并重新保存设置";
        assert_eq!(c.message(), reason);
        assert_eq!(c.on_status(now, Err(PortalError::Network)), Action::None);
        assert_eq!(c.message(), reason);
        c.network_changed(now);
        assert_eq!(c.message(), reason);
        assert_eq!(
            c.on_status(
                now + Duration::from_secs(20),
                Ok(PortalStatus::Online(DialState::Connected))
            ),
            Action::None
        );
        assert_eq!(c.message(), reason);
        assert_eq!(
            c.on_status(now + Duration::from_secs(21), Ok(PortalStatus::Offline)),
            Action::None
        );
        assert_eq!(c.message(), reason);
    }

    #[test]
    fn login_failure_pauses_login_but_allows_redial_when_campus_is_online() {
        let now = Instant::now();
        let mut c = Controller::new(now);
        c.on_action_result(
            now,
            Action::Login,
            Err(PortalError::LoginFailed(AuthFailure::Password)),
        );
        assert_eq!(c.on_status(now, Ok(PortalStatus::Offline)), Action::None);
        assert_eq!(
            c.on_status(now, Ok(PortalStatus::Online(DialState::Failed))),
            Action::Redial
        );
        c.on_action_result(now, Action::Redial, Ok(None));
        assert_eq!(c.message(), "门户提示密码有误，请检查并重新保存设置");
        assert_eq!(
            c.on_status(
                now + Duration::from_secs(4),
                Ok(PortalStatus::Online(DialState::Connected))
            ),
            Action::None
        );
        assert_eq!(
            c.on_status(now + Duration::from_secs(8), Ok(PortalStatus::Offline)),
            Action::None
        );
    }

    #[test]
    fn missing_credentials_keep_all_automatic_actions_paused() {
        let now = Instant::now();
        let mut c = Controller::new(now);
        c.credentials_deleted();
        assert_eq!(
            c.on_status(now, Ok(PortalStatus::Online(DialState::Failed))),
            Action::None
        );
        assert_eq!(c.on_status(now, Ok(PortalStatus::Offline)), Action::None);
    }
    #[test]
    fn sanitized_failure_message_persists_while_automatic_login_is_paused() {
        let now = Instant::now();
        let mut c = Controller::new(now);
        c.on_action_result(
            now,
            Action::Login,
            Err(PortalError::LoginFailed(AuthFailure::Parameters)),
        );
        assert_eq!(c.message(), "门户提示认证参数异常，请打开认证页面检查");
        assert_eq!(
            c.on_status(now + Duration::from_secs(60), Ok(PortalStatus::Offline)),
            Action::None
        );
        assert_eq!(c.message(), "门户提示认证参数异常，请打开认证页面检查");
    }
    #[test]
    fn offline_status_requests_login_automatically() {
        let now = Instant::now();
        let mut c = Controller::new(now);
        assert_eq!(c.on_status(now, Ok(PortalStatus::Offline)), Action::Login);
    }

    #[test]
    fn operator_failure_redials_without_login() {
        let now = Instant::now();
        let mut c = Controller::new(now);
        assert_eq!(
            c.on_status(now, Ok(PortalStatus::Online(DialState::Failed))),
            Action::Redial
        );
    }
    #[test]
    fn retries_are_delayed_and_check_now_does_not_bypass_cooldown() {
        let now = Instant::now();
        let mut c = Controller::new(now);
        assert_eq!(c.on_status(now, Ok(PortalStatus::Offline)), Action::Login);
        c.on_action_result(now, Action::Login, Err(PortalError::Network));
        c.check_now(now);
        assert_eq!(c.on_status(now, Ok(PortalStatus::Offline)), Action::None);
        assert_eq!(
            c.on_status(
                now + std::time::Duration::from_secs(60),
                Ok(PortalStatus::Offline)
            ),
            Action::Login
        );
    }
    #[test]
    fn captcha_blocks_login_until_online_or_new_configuration() {
        let now = Instant::now();
        let mut c = Controller::new(now);
        c.on_action_result(now, Action::Login, Err(PortalError::Captcha));
        assert_eq!(c.on_status(now, Ok(PortalStatus::Offline)), Action::None);
        assert_eq!(
            c.on_status(now, Ok(PortalStatus::Online(DialState::Connected))),
            Action::None
        );
        assert_eq!(c.on_status(now, Ok(PortalStatus::Offline)), Action::Login);
    }
    #[test]
    fn deleted_credentials_block_until_reconfigured() {
        let now = Instant::now();
        let mut c = Controller::new(now);
        c.credentials_deleted();
        assert_eq!(
            c.on_status(now, Ok(PortalStatus::Online(DialState::Connected))),
            Action::None
        );
        assert_eq!(c.on_status(now, Ok(PortalStatus::Offline)), Action::None);
        c.configured(now);
        assert_eq!(c.on_status(now, Ok(PortalStatus::Offline)), Action::Login);
    }
    #[test]
    fn successful_action_polls_then_redials_if_dial_fails() {
        let now = Instant::now();
        let mut c = Controller::new(now);
        c.on_action_result(
            now,
            Action::Login,
            Ok(Some(PortalStatus::Online(DialState::Pending))),
        );
        assert!(!c.due(now));
        let later = now + std::time::Duration::from_secs(4);
        assert!(c.due(later));
        assert_eq!(
            c.on_status(later, Ok(PortalStatus::Online(DialState::Failed))),
            Action::None
        );
        assert_eq!(
            c.on_status(
                later + std::time::Duration::from_secs(15),
                Ok(PortalStatus::Online(DialState::Failed))
            ),
            Action::Redial
        );
    }
    #[test]
    fn network_change_waits_to_stabilize() {
        let now = Instant::now();
        let mut c = Controller::new(now);
        c.network_changed(now);
        assert!(!c.due(now));
        assert!(c.due(now + std::time::Duration::from_secs(5)));
    }
    #[test]
    fn network_change_does_not_forget_acknowledged_redial() {
        let start = Instant::now();
        let mut c = Controller::new(start);
        c.on_action_result(start, Action::Redial, Ok(None));
        c.network_changed(start + std::time::Duration::from_secs(1));
        let now = start + std::time::Duration::from_secs(6);
        assert_eq!(
            c.on_status(now, Ok(PortalStatus::Online(DialState::Failed))),
            Action::None
        );
    }
    #[test]
    fn login_result_reports_online_immediately() {
        let now = Instant::now();
        let mut c = Controller::new(now);
        c.on_action_result(
            now,
            Action::Login,
            Ok(Some(PortalStatus::Online(DialState::Connected))),
        );
        assert_eq!(c.message(), "校园网在线");
    }
    #[test]
    fn login_result_operator_failure_allows_redial_after_recheck() {
        let now = Instant::now();
        let mut c = Controller::new(now);
        c.on_action_result(
            now,
            Action::Login,
            Ok(Some(PortalStatus::Online(DialState::Failed))),
        );
        let later = now + std::time::Duration::from_secs(4);
        assert_eq!(
            c.on_status(later, Ok(PortalStatus::Online(DialState::Failed))),
            Action::Redial
        );
    }
    #[test]
    fn pending_redial_times_out_after_15_checks_and_respects_cooldown() {
        let start = Instant::now();
        let mut c = Controller::new(start);
        c.on_action_result(start, Action::Redial, Ok(None));
        let mut now = start;
        for _ in 0..15 {
            now += std::time::Duration::from_secs(4);
            assert_eq!(
                c.on_status(now, Ok(PortalStatus::Online(DialState::Pending))),
                Action::None
            );
        }
        assert_eq!(c.next(), now + std::time::Duration::from_secs(15));
        c.check_now(now);
        assert_eq!(
            c.on_status(now, Ok(PortalStatus::Online(DialState::Failed))),
            Action::None
        );
    }
    #[test]
    fn lost_redial_response_does_not_repeat_while_server_may_still_update() {
        let start = Instant::now();
        let mut c = Controller::new(start);
        c.on_action_result(start, Action::Redial, Err(PortalError::Network));
        for seconds in [15, 19, 23, 27, 31, 35, 39, 43, 47, 51, 55, 59] {
            let now = start + std::time::Duration::from_secs(seconds);
            assert_eq!(
                c.on_status(now, Ok(PortalStatus::Online(DialState::Failed))),
                Action::None
            );
        }
        let now = start + std::time::Duration::from_secs(60);
        assert_eq!(
            c.on_status(now, Ok(PortalStatus::Online(DialState::Failed))),
            Action::Redial
        );
    }
    #[test]
    fn network_change_after_login_post_observes_old_offline_state_before_retrying() {
        let start = Instant::now();
        let mut c = Controller::new(start);
        c.on_action_result(start, Action::Login, Err(PortalError::NetworkChanged));
        for seconds in [5, 9, 13, 17, 21, 25, 29, 33, 37, 41, 45, 49, 53, 57, 59] {
            let now = start + std::time::Duration::from_secs(seconds);
            assert_eq!(c.on_status(now, Ok(PortalStatus::Offline)), Action::None);
        }
        assert_eq!(
            c.on_status(
                start + std::time::Duration::from_secs(60),
                Ok(PortalStatus::Offline)
            ),
            Action::Login
        );
    }
    #[test]
    fn malformed_redial_response_observes_old_failed_state_before_retrying() {
        let start = Instant::now();
        let mut c = Controller::new(start);
        c.on_action_result(start, Action::Redial, Err(PortalError::Unexpected));
        for seconds in [15, 19, 23, 27, 31, 35, 39, 43, 47, 51, 55, 59] {
            let now = start + std::time::Duration::from_secs(seconds);
            assert_eq!(
                c.on_status(now, Ok(PortalStatus::Online(DialState::Failed))),
                Action::None
            );
        }
        let now = start + std::time::Duration::from_secs(60);
        assert_eq!(
            c.on_status(now, Ok(PortalStatus::Online(DialState::Failed))),
            Action::Redial
        );
    }
    #[test]
    fn transient_status_error_during_redial_does_not_discard_polling() {
        let start = Instant::now();
        let mut c = Controller::new(start);
        c.on_action_result(start, Action::Redial, Ok(None));
        assert_eq!(
            c.on_status(
                start + std::time::Duration::from_secs(4),
                Err(PortalError::Network)
            ),
            Action::None
        );
        assert_eq!(
            c.on_status(
                start + std::time::Duration::from_secs(19),
                Ok(PortalStatus::Online(DialState::Failed))
            ),
            Action::None
        );
        assert_eq!(
            c.on_status(
                start + std::time::Duration::from_secs(49),
                Ok(PortalStatus::Online(DialState::Failed))
            ),
            Action::Redial
        );
    }
    #[test]
    fn lost_login_response_waits_for_server_to_finish_before_retrying() {
        let start = Instant::now();
        let mut c = Controller::new(start);
        c.on_action_result(start, Action::Login, Err(PortalError::Network));
        for seconds in [15, 19, 23, 27, 31, 35, 39, 43, 47, 51, 55, 59] {
            assert_eq!(
                c.on_status(
                    start + std::time::Duration::from_secs(seconds),
                    Ok(PortalStatus::Offline)
                ),
                Action::None
            );
        }
        assert_eq!(
            c.on_status(
                start + std::time::Duration::from_secs(60),
                Ok(PortalStatus::Offline)
            ),
            Action::Login
        );
    }
}

#[cfg(test)]
mod end_to_end_tests {
    use super::*;
    use crate::portal::{
        test_support::{LoginOutcome, MockPortal, RedialOutcome, TEST_PASSWORD, TEST_USERNAME},
        PortalClient,
    };

    fn tick(controller: &mut Controller, client: &PortalClient, now: Instant) {
        assert!(controller.due(now));
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
    fn count(mock: &MockPortal, request: &str) -> usize {
        mock.requests()
            .iter()
            .filter(|line| line.as_str() == request)
            .count()
    }
    #[test]
    fn full_login_and_redial_flow_uses_local_server_and_fake_credentials() {
        let mock = MockPortal::new(
            PortalStatus::Offline,
            LoginOutcome::Success,
            RedialOutcome::SuccessAfterPending(2),
        );
        let client = mock.client();
        let mut now = Instant::now();
        let mut controller = Controller::new(now);
        tick(&mut controller, &client, now);
        assert_eq!(count(&mock, "POST /api/account/login"), 1);
        now = controller.next();
        tick(&mut controller, &client, now);
        assert_eq!(count(&mock, "POST /api/account/redial"), 1);
        for _ in 0..3 {
            now = controller.next();
            tick(&mut controller, &client, now);
        }
        assert_eq!(controller.message(), "校园网在线");
        assert_eq!(count(&mock, "POST /api/account/login"), 1);
        assert_eq!(count(&mock, "POST /api/account/redial"), 1);
    }
    #[test]
    fn lost_login_response_rechecks_state_before_retrying() {
        let mock = MockPortal::new(
            PortalStatus::Offline,
            LoginOutcome::SuccessResponseLost,
            RedialOutcome::SuccessAfterPending(0),
        );
        mock.set_post_login_status(PortalStatus::Online(DialState::Connected));
        let client = mock.client();
        let now = Instant::now();
        let mut controller = Controller::new(now);
        tick(&mut controller, &client, now);
        assert_eq!(count(&mock, "POST /api/account/login"), 1);
        let later = controller.next();
        tick(&mut controller, &client, later);
        assert_eq!(controller.message(), "校园网在线");
        assert_eq!(count(&mock, "POST /api/account/login"), 1);
    }
    #[test]
    fn lost_login_response_then_operator_failure_recovers_without_second_login() {
        let mock = MockPortal::new(
            PortalStatus::Offline,
            LoginOutcome::SuccessResponseLost,
            RedialOutcome::SuccessAfterPending(0),
        );
        let client = mock.client();
        let now = Instant::now();
        let mut controller = Controller::new(now);
        tick(&mut controller, &client, now);
        let later = controller.next();
        tick(&mut controller, &client, later);
        let done = controller.next();
        tick(&mut controller, &client, done);
        assert_eq!(controller.message(), "校园网在线");
        assert_eq!(count(&mock, "POST /api/account/login"), 1);
        assert_eq!(count(&mock, "POST /api/account/redial"), 1);
    }
    #[test]
    fn delayed_login_state_does_not_cause_a_duplicate_post() {
        let mock = MockPortal::new(
            PortalStatus::Offline,
            LoginOutcome::SuccessResponseLost,
            RedialOutcome::SuccessAfterPending(0),
        );
        mock.set_status_lag(3);
        mock.set_post_login_status(PortalStatus::Online(DialState::Connected));
        let client = mock.client();
        let mut now = Instant::now();
        let mut controller = Controller::new(now);
        tick(&mut controller, &client, now);
        for _ in 0..4 {
            now = controller.next();
            tick(&mut controller, &client, now);
        }
        assert_eq!(controller.message(), "校园网在线");
        assert_eq!(count(&mock, "POST /api/account/login"), 1);
    }
    #[test]
    fn delayed_redial_state_does_not_cause_a_duplicate_post() {
        let mock = MockPortal::new(
            PortalStatus::Online(DialState::Failed),
            LoginOutcome::Success,
            RedialOutcome::SuccessResponseLost,
        );
        mock.set_status_lag(3);
        let client = mock.client();
        let mut now = Instant::now();
        let mut controller = Controller::new(now);
        tick(&mut controller, &client, now);
        for _ in 0..4 {
            now = controller.next();
            tick(&mut controller, &client, now);
        }
        assert_eq!(controller.message(), "校园网在线");
        assert_eq!(count(&mock, "POST /api/account/redial"), 1);
    }
    #[test]
    fn transient_status_failure_during_dial_still_recovers_without_duplicate_post() {
        let mock = MockPortal::new(
            PortalStatus::Online(DialState::Failed),
            LoginOutcome::Success,
            RedialOutcome::SuccessAfterPending(2),
        );
        let client = mock.client();
        let mut now = Instant::now();
        let mut controller = Controller::new(now);
        tick(&mut controller, &client, now);
        mock.fail_next_status_requests(1);
        for _ in 0..4 {
            now = controller.next();
            tick(&mut controller, &client, now);
        }
        assert_eq!(controller.message(), "校园网在线");
        assert_eq!(count(&mock, "POST /api/account/redial"), 1);
    }
    #[test]
    fn captcha_does_not_cause_repeated_login() {
        let mock = MockPortal::new(
            PortalStatus::Offline,
            LoginOutcome::Captcha,
            RedialOutcome::SuccessAfterPending(0),
        );
        let client = mock.client();
        let now = Instant::now();
        let mut controller = Controller::new(now);
        tick(&mut controller, &client, now);
        let later = controller.next();
        tick(&mut controller, &client, later);
        assert_eq!(count(&mock, "POST /api/account/login"), 1);
        assert_eq!(count(&mock, "POST /api/account/redial"), 0);
    }
}
