use serde_json::Value;
use socket2::{Domain, Protocol, Socket, Type};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const PORTAL: Ipv4Addr = Ipv4Addr::new(172, 30, 21, 100);
const PORTAL_HOST: &str = "172.30.21.100";
const PROBE_HOST: &str = "www.msftconnecttest.com";
const PROBE_PATH: &str = "/connecttest.txt";
const MAX_RESPONSE: u64 = 256 * 1024;

#[cfg(all(test, windows))]
mod live_read_only_test {
    use super::*;
    #[test]
    #[ignore = "reads actual campus portal state but performs no login or redial"]
    fn current_campus_portal_status_is_readable() {
        let adapters = crate::network::list_adapters().unwrap();
        let (identity, connection) = adapters
            .into_iter()
            .find_map(|a| {
                crate::network::matched_connection(&a.identity)
                    .ok()
                    .flatten()
                    .map(|c| (a.identity, c))
            })
            .expect("no suitable campus adapter");
        let client = PortalClient::new(
            connection.ipv4,
            connection.interface_index,
            move || {
                crate::network::matched_connection(&identity).ok().flatten() == Some(connection)
            },
            Arc::new(Mutex::new(())),
        );
        assert!(
            client.status().is_ok(),
            "campus portal status not recognized"
        );
    }

    #[test]
    fn non_loopback_test_client_cannot_enable_public_discovery() {
        let client = PortalClient::new(
            Ipv4Addr::new(10, 0, 0, 2),
            12,
            || false,
            Arc::new(Mutex::new(())),
        );
        assert_eq!(client.probe_addresses(), Err(PortalError::ContextUnknown));
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DialState {
    Disabled,
    Pending,
    Connected,
    Failed,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PortalStatus {
    Offline,
    Online(DialState),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PortalError {
    Network,
    NetworkChanged,
    Unexpected,
    ContextUnknown,
    ManualAction,
    Captcha,
    CheckFailed(AuthFailure),
    LoginFailed(AuthFailure),
    AlreadyOnline,
}

/// Only fixed categories leave the response parser; never retain server text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthFailure {
    Password,
    Account,
    Parameters,
    Unknown,
}

/// Credential requests connect only to the fixed campus address. A separate,
/// credential-free connectivity probe discovers the selected network's NAS.
/// Every socket binds both the chosen interface and local address; redirects
/// are returned for explicit validation, never followed by the transport.
pub struct PortalClient {
    local: Ipv4Addr,
    remote: SocketAddrV4,
    interface_index: Option<u32>,
    guard: Box<dyn Fn() -> bool + Send + Sync>,
    gate: Arc<Mutex<()>>,
}
impl PortalClient {
    pub fn new(
        local: Ipv4Addr,
        interface_index: u32,
        guard: impl Fn() -> bool + Send + Sync + 'static,
        gate: Arc<Mutex<()>>,
    ) -> Self {
        Self {
            local,
            remote: SocketAddrV4::new(PORTAL, 80),
            interface_index: Some(interface_index),
            guard: Box::new(guard),
            gate,
        }
    }
    fn request(
        &self,
        method: &str,
        path: &str,
        headers: &str,
        body: &[u8],
    ) -> Result<Response, PortalError> {
        self.request_inner(method, path, headers, body, false)
    }
    fn request_inner(
        &self,
        method: &str,
        path: &str,
        headers: &str,
        body: &[u8],
        allow_redirect: bool,
    ) -> Result<Response, PortalError> {
        self.request_to(
            self.remote,
            PORTAL_HOST,
            method,
            path,
            headers,
            body,
            allow_redirect,
        )
    }
    #[allow(clippy::too_many_arguments)]
    fn request_to(
        &self,
        remote: SocketAddrV4,
        host: &str,
        method: &str,
        path: &str,
        headers: &str,
        body: &[u8],
        allow_redirect: bool,
    ) -> Result<Response, PortalError> {
        let portal_request = remote == self.remote && host == PORTAL_HOST;
        if !portal_request && (method != "GET" || !headers.is_empty() || !body.is_empty()) {
            return Err(PortalError::ContextUnknown);
        }
        if !matches!(method, "GET" | "POST")
            || !path.starts_with('/')
            || !path.bytes().all(|byte| (0x21..=0x7e).contains(&byte))
            || !host
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
        {
            return Err(PortalError::ContextUnknown);
        }
        let socket = Socket::new(Domain::IPV4, Type::STREAM, Some(Protocol::TCP))
            .map_err(|_| PortalError::Network)?;
        if let Some(index) = self.interface_index {
            bind_interface(&socket, index)?;
        }
        #[cfg(not(test))]
        if self.interface_index.is_none() {
            return Err(PortalError::NetworkChanged);
        }
        socket
            .bind(&SocketAddr::V4(SocketAddrV4::new(self.local, 0)).into())
            .map_err(|_| PortalError::Network)?;
        socket
            .connect_timeout(&SocketAddr::V4(remote).into(), Duration::from_secs(5))
            .map_err(|_| PortalError::Network)?;
        let mut stream: TcpStream = socket.into();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .map_err(|_| PortalError::Network)?;
        stream
            .set_write_timeout(Some(Duration::from_secs(5)))
            .map_err(|_| PortalError::Network)?;
        let mut request = SecretBytes(Vec::with_capacity(
            method.len() + path.len() + headers.len() + body.len() + 256,
        ));
        write!(request.0, "{method} {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\nAccept: application/json, text/html\r\n{headers}Content-Length: {}\r\n\r\n", body.len()).map_err(|_| PortalError::Network)?;
        request.0.extend_from_slice(body);
        // Serialize final validation and the complete write with settings and
        // credential changes. Never hold the lock while awaiting a response.
        if method == "POST" {
            let _gate = self
                .gate
                .try_lock()
                .map_err(|_| PortalError::NetworkChanged)?;
            if !(self.guard)() {
                return Err(PortalError::NetworkChanged);
            }
            stream
                .write_all(&request.0)
                .map_err(|_| PortalError::Network)?;
        } else {
            stream
                .write_all(&request.0)
                .map_err(|_| PortalError::Network)?;
        }
        let mut bytes = Vec::new();
        stream
            .take(MAX_RESPONSE + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| PortalError::Network)?;
        if bytes.len() as u64 > MAX_RESPONSE {
            return Err(PortalError::Unexpected);
        }
        // The peer may have applied a POST before dropping its response. Treat an
        // empty response as transient so the controller can recheck status first.
        if method == "POST" && bytes.is_empty() {
            return Err(PortalError::Network);
        }
        parse_http_inner(&bytes, allow_redirect)
    }
    /// Discover context from an intercepted connectivity probe. The portal's
    /// default route can identify an unrelated campus NAS and is never used.
    fn discover_nas_id(&self) -> Result<String, PortalError> {
        let discover = || {
            let mut first = None;
            for remote in self.probe_addresses()? {
                match self.request_to(remote, PROBE_HOST, "GET", PROBE_PATH, "", b"", true) {
                    Ok(response) => {
                        first = Some(response);
                        break;
                    }
                    Err(PortalError::Network) => continue,
                    Err(error) => return Err(error),
                }
            }
            let first = first.ok_or(PortalError::ContextUnknown)?;
            let location = first
                .location
                .as_deref()
                .ok_or(PortalError::ContextUnknown)?;
            let (path, id) = captive_route(location, self.local)?;
            let second = self.request_inner("GET", path, "", b"", true)?;
            verify_login_location(
                second
                    .location
                    .as_deref()
                    .ok_or(PortalError::ContextUnknown)?,
                self.local,
                id,
            )?;
            Ok(id.to_owned())
        };
        discover().map_err(|error| match error {
            PortalError::NetworkChanged => error,
            _ => PortalError::ContextUnknown,
        })
    }
    #[cfg(test)]
    fn probe_addresses(&self) -> Result<Vec<SocketAddrV4>, PortalError> {
        // Test clients explicitly inject a loopback HTTP server as `remote`.
        // Even ignored tests cannot accidentally resolve or contact the public probe.
        self.remote
            .ip()
            .is_loopback()
            .then(|| vec![self.remote])
            .ok_or(PortalError::ContextUnknown)
    }
    #[cfg(not(test))]
    fn probe_addresses(&self) -> Result<Vec<SocketAddrV4>, PortalError> {
        resolve_probe_addresses()
    }
    /// Verify expected page before any credential-bearing request. HTTP does not
    /// authenticate its peer; the user must explicitly accept that risk.
    fn verify(&self) -> Result<(), PortalError> {
        let page = self.request("GET", "/tpl/isp/login_account.html", "", b"")?;
        if page.body.contains("/account/status")
            && page.body.contains("/account/redial")
            && page.body.contains("/account/check")
        {
            Ok(())
        } else {
            Err(PortalError::Unexpected)
        }
    }
    fn session(&self) -> Result<String, PortalError> {
        let response = self.request("GET", "/api/csrf-token", "", b"")?;
        let value: Value =
            serde_json::from_str(&response.body).map_err(|_| PortalError::Unexpected)?;
        let token = value
            .get("csrf_token")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty() && safe_header(s))
            .ok_or(PortalError::Unexpected)?;
        let cookie = response
            .cookie
            .as_deref()
            .filter(|s| safe_header(s))
            .ok_or(PortalError::Unexpected)?;
        Ok(format!("X-CSRF-Token: {token}\r\nCookie: {cookie}\r\n"))
    }
    pub fn status(&self) -> Result<PortalStatus, PortalError> {
        self.verify()?;
        let headers = self.session()?;
        let response = self.request("GET", "/api/account/status", &headers, b"")?;
        parse_status(&response.body)
    }
    pub fn login(&self, username: &str, password: &str) -> Result<PortalStatus, PortalError> {
        if username.is_empty()
            || password.is_empty()
            || username.len() > 1024
            || password.len() > 4096
        {
            return Err(PortalError::ManualAction);
        }
        self.verify()?;
        let nas_id = self.discover_nas_id().map_err(|e| {
            if e == PortalError::Unexpected {
                PortalError::ContextUnknown
            } else {
                e
            }
        })?;
        let mut headers = self.session()?;
        headers.push_str("Content-Type: application/x-www-form-urlencoded\r\n");
        // Reserve the encoded maximum up front so reallocations do not leave copies
        // of credential bytes behind in freed allocations.
        let mut body = credential_form_buffer(username.len(), password.len());
        body.0.extend_from_slice(b"username=");
        form_encode_into(username, &mut body.0);
        body.0.extend_from_slice(b"&password=");
        form_encode_into(password, &mut body.0);
        write!(body.0, "&userIpv4={}&nasId={}", self.local, nas_id)
            .map_err(|_| PortalError::Unexpected)?;
        let check = self.request("POST", "/api/account/check", &headers, &body.0)?;
        match parse_check(&check.body) {
            Ok(()) => {}
            Err(PortalError::AlreadyOnline) => return self.status(),
            Err(error) => return Err(error),
        }
        let login = self.request("POST", "/api/account/login", &headers, &body.0)?;
        match parse_login(&login.body) {
            Ok(()) | Err(PortalError::AlreadyOnline) => {}
            Err(error) => return Err(error),
        }
        drop(body);
        self.status()
    }
    pub fn redial(&self) -> Result<(), PortalError> {
        self.verify()?;
        let headers = self.session()?;
        let response = self.request("POST", "/api/account/redial", &headers, b"")?;
        let value: Value =
            serde_json::from_str(&response.body).map_err(|_| PortalError::Unexpected)?;
        if value.get("code").and_then(Value::as_i64) == Some(0) {
            Ok(())
        } else {
            Err(PortalError::Unexpected)
        }
    }
}

#[cfg(not(test))]
fn resolve_probe_addresses() -> Result<Vec<SocketAddrV4>, PortalError> {
    use std::{net::ToSocketAddrs, sync::mpsc, thread};
    let (sender, receiver) = mpsc::sync_channel(1);
    thread::Builder::new()
        .name("campus-probe-dns".into())
        .spawn(move || {
            let addresses = (PROBE_HOST, 80).to_socket_addrs().map(|addresses| {
                let mut selected = Vec::new();
                for address in addresses {
                    if let SocketAddr::V4(address) = address {
                        let ip = address.ip();
                        if !ip.is_unspecified()
                            && !ip.is_loopback()
                            && !ip.is_multicast()
                            && !ip.is_broadcast()
                            && !selected.contains(&address)
                        {
                            selected.push(address);
                            if selected.len() == 3 {
                                break;
                            }
                        }
                    }
                }
                selected
            });
            let _ = sender.send(addresses);
        })
        .map_err(|_| PortalError::ContextUnknown)?;
    let addresses = receiver
        .recv_timeout(Duration::from_secs(5))
        .map_err(|_| PortalError::ContextUnknown)?
        .map_err(|_| PortalError::ContextUnknown)?;
    if addresses.is_empty() {
        Err(PortalError::ContextUnknown)
    } else {
        Ok(addresses)
    }
}

fn fixed_portal_path(location: &str, require_absolute: bool) -> Result<&str, PortalError> {
    if let Some(path) = location.strip_prefix("http://172.30.21.100/") {
        return Ok(&location[location.len() - path.len() - 1..]);
    }
    if !require_absolute && location.starts_with('/') && !location.starts_with("//") {
        return Ok(location);
    }
    Err(PortalError::ContextUnknown)
}

fn query_component(raw: &str) -> Result<String, PortalError> {
    let mut decoded = Vec::with_capacity(raw.len());
    let mut bytes = raw.bytes();
    while let Some(byte) = bytes.next() {
        let byte = match byte {
            b'%' => {
                let digit = |byte: u8| (byte as char).to_digit(16).map(|value| value as u8);
                let high = digit(bytes.next().ok_or(PortalError::ContextUnknown)?)
                    .ok_or(PortalError::ContextUnknown)?;
                let low = digit(bytes.next().ok_or(PortalError::ContextUnknown)?)
                    .ok_or(PortalError::ContextUnknown)?;
                high * 16 + low
            }
            b'+' => b' ',
            other => other,
        };
        if byte < 0x20 || byte == 0x7f || matches!(byte, b'\\' | b'#') {
            return Err(PortalError::ContextUnknown);
        }
        decoded.push(byte);
    }
    let decoded = String::from_utf8(decoded).map_err(|_| PortalError::ContextUnknown)?;
    if decoded.trim().is_empty() {
        return Err(PortalError::ContextUnknown);
    }
    Ok(decoded)
}

fn context_query(query: &str) -> Result<HashMap<String, String>, PortalError> {
    if query.is_empty()
        || query.len() > 2048
        || !query.bytes().all(|byte| (0x21..=0x7e).contains(&byte))
        || query.contains(['#', '\\', '?'])
    {
        return Err(PortalError::ContextUnknown);
    }
    let mut parameters = HashMap::new();
    let mut unique = std::collections::HashSet::new();
    for pair in query.split('&') {
        if parameters.len() >= 32 {
            return Err(PortalError::ContextUnknown);
        }
        let (key, value) = pair.split_once('=').ok_or(PortalError::ContextUnknown)?;
        let key = query_component(key)?;
        if !key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
            || !unique.insert(key.to_ascii_lowercase())
        {
            return Err(PortalError::ContextUnknown);
        }
        // The real campus gateway can include an empty optional WLAN AC name.
        // Required source IP/NAS fields still have to parse and match below.
        let value = if key == "wlanacname" && value.is_empty() {
            String::new()
        } else {
            query_component(value)?
        };
        parameters.insert(key, value);
    }
    Ok(parameters)
}

fn captive_route(location: &str, local: Ipv4Addr) -> Result<(&str, &str), PortalError> {
    let path = fixed_portal_path(location, true)?;
    let (route, query) = path.split_once('?').ok_or(PortalError::ContextUnknown)?;
    let id = route
        .strip_prefix("/api/r/")
        .ok_or(PortalError::ContextUnknown)?;
    if id.is_empty() || id.len() > 20 || !id.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(PortalError::ContextUnknown);
    }
    let parameters = context_query(query)?;
    if parameters
        .get("userip")
        .and_then(|value| value.parse::<Ipv4Addr>().ok())
        != Some(local)
    {
        return Err(PortalError::ContextUnknown);
    }
    Ok((path, id))
}

fn verify_login_location(location: &str, local: Ipv4Addr, id: &str) -> Result<(), PortalError> {
    let path = fixed_portal_path(location, false)?;
    let (template, query) = path.split_once('?').ok_or(PortalError::ContextUnknown)?;
    if !matches!(template, "/tpl/isp/login.html" | "/tpl/whut/login.html") {
        return Err(PortalError::ContextUnknown);
    }
    let parameters = context_query(query)?;
    if parameters
        .get("ip")
        .and_then(|value| value.parse::<Ipv4Addr>().ok())
        != Some(local)
        || parameters.get("nasId").map(String::as_str) != Some(id)
        || parameters
            .get("userip")
            .is_some_and(|value| value.parse::<Ipv4Addr>().ok() != Some(local))
    {
        return Err(PortalError::ContextUnknown);
    }
    Ok(())
}

#[cfg(windows)]
fn bind_interface(socket: &Socket, index: u32) -> Result<(), PortalError> {
    use std::os::windows::io::AsRawSocket;
    use windows_sys::Win32::Networking::WinSock::{
        setsockopt, IPPROTO_IP, IP_UNICAST_IF, SOCKET_ERROR,
    };
    if index == 0 {
        return Err(PortalError::NetworkChanged);
    }
    // Windows requires the IPv4 interface index in network byte order.
    let network_order = index.to_be();
    let result = unsafe {
        setsockopt(
            socket.as_raw_socket() as usize,
            IPPROTO_IP,
            IP_UNICAST_IF,
            (&network_order as *const u32).cast(),
            std::mem::size_of::<u32>() as i32,
        )
    };
    if result == SOCKET_ERROR {
        Err(PortalError::NetworkChanged)
    } else {
        Ok(())
    }
}
#[cfg(not(windows))]
fn bind_interface(_socket: &Socket, _index: u32) -> Result<(), PortalError> {
    Err(PortalError::NetworkChanged)
}

fn safe_header(s: &str) -> bool {
    s.bytes().all(|b| (0x20..=0x7e).contains(&b))
}
struct Response {
    body: String,
    cookie: Option<String>,
    location: Option<String>,
}
#[cfg(test)]
fn parse_http(bytes: &[u8]) -> Result<Response, PortalError> {
    parse_http_inner(bytes, false)
}
fn parse_http_inner(bytes: &[u8], allow_redirect: bool) -> Result<Response, PortalError> {
    let split = bytes
        .windows(4)
        .position(|x| x == b"\r\n\r\n")
        .ok_or(PortalError::Unexpected)?;
    let head = std::str::from_utf8(&bytes[..split]).map_err(|_| PortalError::Unexpected)?;
    let mut lines = head.split("\r\n");
    let first = lines.next().ok_or(PortalError::Unexpected)?;
    let ok = first.starts_with("HTTP/1.1 200 ") || first.starts_with("HTTP/1.0 200 ");
    let redirect = first.starts_with("HTTP/1.1 302 ") || first.starts_with("HTTP/1.0 302 ");
    if !ok && !(allow_redirect && redirect) {
        return Err(PortalError::Unexpected);
    }
    let mut cookie = None;
    let mut location = None;
    let mut chunked = false;
    let mut length = None;
    for line in lines {
        let (key, value) = line.split_once(':').ok_or(PortalError::Unexpected)?;
        let value = value.trim();
        if key.eq_ignore_ascii_case("location") {
            if location.is_some() || !safe_header(value) {
                return Err(PortalError::Unexpected);
            }
            location = Some(value.to_owned());
        }
        if key.eq_ignore_ascii_case("set-cookie") && value.starts_with("yudear=") {
            cookie = Some(value.split(';').next().unwrap_or("").to_owned());
        }
        if key.eq_ignore_ascii_case("transfer-encoding") {
            if !value.eq_ignore_ascii_case("chunked") {
                return Err(PortalError::Unexpected);
            }
            chunked = true;
        }
        if key.eq_ignore_ascii_case("content-length") {
            length = Some(
                value
                    .parse::<usize>()
                    .map_err(|_| PortalError::Unexpected)?,
            );
        }
    }
    let raw = &bytes[split + 4..];
    let data = if chunked {
        decode_chunks(raw)?
    } else {
        if let Some(n) = length {
            if raw.len() != n {
                return Err(PortalError::Unexpected);
            }
        }
        raw.to_vec()
    };
    let body = String::from_utf8(data).map_err(|_| PortalError::Unexpected)?;
    if allow_redirect && (!redirect || location.is_none()) {
        return Err(PortalError::Unexpected);
    }
    Ok(Response {
        body,
        cookie,
        location,
    })
}
fn decode_chunks(mut data: &[u8]) -> Result<Vec<u8>, PortalError> {
    let mut out = Vec::new();
    loop {
        let p = data
            .windows(2)
            .position(|x| x == b"\r\n")
            .ok_or(PortalError::Unexpected)?;
        let size = std::str::from_utf8(&data[..p])
            .map_err(|_| PortalError::Unexpected)?
            .split(';')
            .next()
            .ok_or(PortalError::Unexpected)?;
        let n = usize::from_str_radix(size, 16).map_err(|_| PortalError::Unexpected)?;
        data = &data[p + 2..];
        if n == 0 {
            return Ok(out);
        }
        if n > data.len().saturating_sub(2) || &data[n..n + 2] != b"\r\n" {
            return Err(PortalError::Unexpected);
        }
        out.extend_from_slice(&data[..n]);
        data = &data[n + 2..];
    }
}
fn json(s: &str) -> Result<Value, PortalError> {
    serde_json::from_str(s).map_err(|_| PortalError::Unexpected)
}
fn parse_status(s: &str) -> Result<PortalStatus, PortalError> {
    let v = json(s)?;
    match v.get("code").and_then(Value::as_i64) {
        // The page treats every nonzero code as offline; that is not enough to
        // distinguish an authentication expiry from an internal portal error.
        Some(1) => match v.get("msg").and_then(Value::as_str).map(str::trim) {
            Some("未登录" | "用户未登录" | "未认证" | "用户未认证" | "不在线" | "用户不在线") => {
                Ok(PortalStatus::Offline)
            }
            _ => Err(PortalError::Unexpected),
        },
        Some(0) => {
            if !v.get("online").is_some_and(Value::is_object) {
                return Err(PortalError::Unexpected);
            }
            let state = match v.get("enableDial").and_then(Value::as_bool) {
                Some(false) => DialState::Disabled,
                Some(true) => {
                    let code = v
                        .get("dialCode")
                        .and_then(Value::as_str)
                        .ok_or(PortalError::Unexpected)?;
                    let msg = v
                        .get("dialMsg")
                        .and_then(Value::as_str)
                        .ok_or(PortalError::Unexpected)?;
                    if code == "ok:dialup" {
                        DialState::Connected
                    } else if code.is_empty() && msg.is_empty() {
                        DialState::Pending
                    } else {
                        // Match the portal: any nonempty result other than its
                        // exact success code displays the redial button.
                        DialState::Failed
                    }
                }
                _ => return Err(PortalError::Unexpected),
            };
            Ok(PortalStatus::Online(state))
        }
        _ => Err(PortalError::Unexpected),
    }
}
fn parse_check(s: &str) -> Result<(), PortalError> {
    let v = json(s)?;
    match v.get("code").and_then(Value::as_i64) {
        Some(0) => match v.get("isChangePwd") {
            Some(Value::Number(n)) if n.as_i64() == Some(0) => Ok(()),
            Some(Value::String(s)) if s == "0" => Ok(()),
            Some(Value::Number(n)) if n.as_i64() == Some(1) => Err(PortalError::ManualAction),
            Some(Value::String(s)) if s == "1" => Err(PortalError::ManualAction),
            _ => Err(PortalError::Unexpected),
        },
        Some(1) => match failure_message(&v) {
            FailureMessage::AlreadyOnline => Err(PortalError::AlreadyOnline),
            FailureMessage::Other(reason) => Err(PortalError::CheckFailed(reason)),
        },
        Some(2) => Err(PortalError::ManualAction),
        _ => Err(PortalError::Unexpected),
    }
}
fn parse_login(s: &str) -> Result<(), PortalError> {
    let v = json(s)?;
    match v.get("code").and_then(Value::as_i64) {
        Some(0) => Ok(()),
        Some(1) => match failure_message(&v) {
            FailureMessage::AlreadyOnline => Err(PortalError::AlreadyOnline),
            FailureMessage::Other(reason) => Err(PortalError::LoginFailed(reason)),
        },
        Some(2) => Err(PortalError::Captcha),
        _ => Err(PortalError::Unexpected),
    }
}

enum FailureMessage {
    AlreadyOnline,
    Other(AuthFailure),
}

fn failure_message(value: &Value) -> FailureMessage {
    let message = value
        .get("authMsg")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .or_else(|| value.get("msg").and_then(Value::as_str))
        .unwrap_or("")
        .trim();
    // Only exact known phrases are authoritative enough to trigger a state recheck.
    if matches!(
        message,
        "用户已在线" | "用户已经在线" | "账号已在线" | "账号已经在线" | "已在线" | "已经在线"
    ) {
        return FailureMessage::AlreadyOnline;
    }
    let contains = |phrases: &[&str]| phrases.iter().any(|phrase| message.contains(phrase));
    let reason = if contains(&["密码错误", "密码不正确", "密码有误", "密码不对"]) {
        AuthFailure::Password
    } else if contains(&[
        "账号不存在",
        "账户不存在",
        "用户不存在",
        "账号已锁定",
        "账户已锁定",
        "用户已锁定",
        "账号停用",
        "账户停用",
    ]) {
        AuthFailure::Account
    } else if contains(&["参数错误", "参数不正确", "参数有误", "缺少参数", "无效参数"])
    {
        AuthFailure::Parameters
    } else {
        AuthFailure::Unknown
    };
    FailureMessage::Other(reason)
}
// Best-effort scrubbing keeps credential-bearing temporary request buffers short-lived.
struct SecretBytes(Vec<u8>);
fn credential_form_buffer(username_len: usize, password_len: usize) -> SecretBytes {
    // Covers all literal keys, a 15-byte IPv4 address, and a 20-digit NAS ID.
    SecretBytes(Vec::with_capacity(3 * (username_len + password_len) + 96))
}
impl Drop for SecretBytes {
    fn drop(&mut self) {
        for byte in &mut self.0 {
            unsafe {
                std::ptr::write_volatile(byte, 0);
            }
        }
        std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
    }
}
fn form_encode_into(s: &str, out: &mut Vec<u8>) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b),
            b' ' => out.push(b'+'),
            _ => {
                out.push(b'%');
                out.push(HEX[(b >> 4) as usize]);
                out.push(HEX[(b & 15) as usize]);
            }
        }
    }
}
#[cfg(test)]
fn form_encode(s: &str) -> String {
    let mut out = Vec::new();
    form_encode_into(s, &mut out);
    String::from_utf8(out).unwrap()
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_distinguishes_dial_states_and_refuses_unknown() {
        assert_eq!(
            parse_status(
                r#"{"code":0,"online":{},"enableDial":true,"dialCode":"ok:dialup","dialMsg":"ok"}"#
            )
            .unwrap(),
            PortalStatus::Online(DialState::Connected)
        );
        assert_eq!(
            parse_status(r#"{"code":0,"online":{},"enableDial":true,"dialCode":"","dialMsg":""}"#)
                .unwrap(),
            PortalStatus::Online(DialState::Pending)
        );
        assert_eq!(parse_status(r#"{"code":0,"online":{},"enableDial":true,"dialCode":"err:dialup","dialMsg":"failed"}"#).unwrap(), PortalStatus::Online(DialState::Failed));
        assert!(parse_status(r#"{"code":0}"#).is_err());
        assert!(parse_status(r#"{"code":99}"#).is_err());
    }

    #[test]
    fn ambiguous_states_never_authorize_login_or_redial() {
        assert!(parse_status(r#"{"code":1,"msg":"backend failure"}"#).is_err());
        assert!(parse_status(r#"{"code":1}"#).is_err());
        assert_eq!(
            parse_status(r#"{"code":1,"msg":"用户未登录"}"#).unwrap(),
            PortalStatus::Offline
        );
        assert!(parse_status(
            r#"{"code":0,"online":{},"enableDial":true,"dialCode":null,"dialMsg":"failed"}"#
        )
        .is_err());
        assert!(
            parse_status(r#"{"code":0,"online":{},"enableDial":true,"dialCode":"failure"}"#)
                .is_err()
        );
    }

    #[test]
    fn portal_offers_redial_for_any_nonempty_unsuccessful_dial_result() {
        for response in [
            r#"{"code":0,"online":{},"enableDial":true,"dialCode":"","dialMsg":"拨号失败"}"#,
            r#"{"code":0,"online":{},"enableDial":true,"dialCode":"operator-error","dialMsg":""}"#,
            r#"{"code":0,"online":{},"enableDial":true,"dialCode":"other","dialMsg":"详情"}"#,
        ] {
            assert_eq!(
                parse_status(response),
                Ok(PortalStatus::Online(DialState::Failed))
            );
        }
    }

    #[test]
    fn credential_and_manual_intervention_fail_closed() {
        assert!(matches!(
            parse_check(r#"{"code":0,"isChangePwd":1}"#),
            Err(PortalError::ManualAction)
        ));
        assert!(matches!(
            parse_check(r#"{"code":1,"msg":"bad password"}"#),
            Err(PortalError::CheckFailed(AuthFailure::Unknown))
        ));
        assert!(matches!(
            parse_login(r#"{"code":2}"#),
            Err(PortalError::Captcha)
        ));
        assert!(parse_check(r#"{"code":7}"#).is_err());
    }

    #[test]
    fn real_portal_accepts_string_password_change_flags() {
        assert!(parse_check(r#"{"code":0,"isChangePwd":"0"}"#).is_ok());
        assert_eq!(
            parse_check(r#"{"code":0,"isChangePwd":"1"}"#),
            Err(PortalError::ManualAction)
        );
    }

    #[test]
    fn login_failures_are_classified_without_carrying_server_text() {
        for (response, expected) in [
            (
                r#"{"code":1,"authMsg":"用户名或密码错误，示例私人内容"}"#,
                "LoginFailed(Password)",
            ),
            (r#"{"code":1,"msg":"账号不存在"}"#, "LoginFailed(Account)"),
            (r#"{"code":1,"msg":"参数错误"}"#, "LoginFailed(Parameters)"),
            (
                r#"{"code":1,"msg":"secret-example"}"#,
                "LoginFailed(Unknown)",
            ),
            (
                r#"{"code":1,"authMsg":"密码错误","msg":"参数错误"}"#,
                "LoginFailed(Password)",
            ),
        ] {
            assert_eq!(
                format!("{:?}", parse_login(response).unwrap_err()),
                expected
            );
        }
        assert_eq!(
            format!(
                "{:?}",
                parse_check(r#"{"code":1,"msg":"参数错误"}"#).unwrap_err()
            ),
            "CheckFailed(Parameters)"
        );
    }

    #[test]
    fn already_online_signal_is_distinct_from_failed_credentials() {
        assert_eq!(
            format!(
                "{:?}",
                parse_login(r#"{"code":1,"msg":"用户已在线"}"#).unwrap_err()
            ),
            "AlreadyOnline"
        );
        assert_eq!(
            format!(
                "{:?}",
                parse_check(r#"{"code":1,"msg":"用户已在线"}"#).unwrap_err()
            ),
            "AlreadyOnline"
        );
        assert_eq!(
            format!(
                "{:?}",
                parse_login(r#"{"code":1,"msg":"用户已在线但账号已锁定"}"#).unwrap_err()
            ),
            "LoginFailed(Account)"
        );
    }

    #[test]
    fn encodes_credentials_as_form_data() {
        assert_eq!(form_encode("a+b &中"), "a%2Bb+%26%E4%B8%AD");
    }

    #[test]
    fn credential_form_preallocates_for_maximum_escaping_and_context() {
        let username = "&".repeat(1024);
        let password = "&".repeat(4096);
        let mut body = credential_form_buffer(username.len(), password.len());
        let initial_capacity = body.0.capacity();
        body.0.extend_from_slice(b"username=");
        form_encode_into(&username, &mut body.0);
        body.0.extend_from_slice(b"&password=");
        form_encode_into(&password, &mut body.0);
        write!(
            body.0,
            "&userIpv4={}&nasId={}",
            "255.255.255.255",
            "9".repeat(20)
        )
        .unwrap();
        assert!(
            body.0.len() <= initial_capacity,
            "credential buffer must not grow"
        );
    }
}

#[cfg(test)]
mod transport_tests {
    use super::*;
    use std::net::TcpListener;
    use std::thread;

    fn serve(responses: Vec<&'static str>) -> (PortalClient, thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let client = PortalClient {
            local: Ipv4Addr::LOCALHOST,
            remote: SocketAddrV4::new(Ipv4Addr::LOCALHOST, port),
            interface_index: None,
            guard: Box::new(|| true),
            gate: Arc::new(Mutex::new(())),
        };
        let task = thread::spawn(move || {
            let mut requests = Vec::new();
            for response in responses {
                let (mut stream, peer) = listener.accept().unwrap();
                assert_eq!(peer.ip(), Ipv4Addr::LOCALHOST);
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut buf = Vec::new();
                let mut block = [0; 1024];
                loop {
                    let n = stream.read(&mut block).unwrap();
                    assert_ne!(n, 0);
                    buf.extend_from_slice(&block[..n]);
                    if let Some(p) = buf.windows(4).position(|x| x == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&buf[..p]);
                        let len = head
                            .lines()
                            .find_map(|l| l.strip_prefix("Content-Length: "))
                            .unwrap()
                            .parse::<usize>()
                            .unwrap();
                        if buf.len() >= p + 4 + len {
                            break;
                        }
                    }
                }
                requests.push(String::from_utf8(buf).unwrap());
                stream.write_all(response.as_bytes()).unwrap();
            }
            requests
        });
        (client, task)
    }
    fn ok(body: &str) -> String {
        format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        )
    }
    #[test]
    fn non_portal_sensitive_requests_are_rejected_before_connecting() {
        let portal = TcpListener::bind("127.0.0.1:0").unwrap();
        let probe = TcpListener::bind("127.0.0.1:0").unwrap();
        portal.set_nonblocking(true).unwrap();
        probe.set_nonblocking(true).unwrap();
        let portal_remote =
            SocketAddrV4::new(Ipv4Addr::LOCALHOST, portal.local_addr().unwrap().port());
        let probe_remote =
            SocketAddrV4::new(Ipv4Addr::LOCALHOST, probe.local_addr().unwrap().port());
        assert_ne!(portal_remote, probe_remote);
        let client = PortalClient {
            local: Ipv4Addr::LOCALHOST,
            remote: portal_remote,
            interface_index: None,
            guard: Box::new(|| true),
            gate: Arc::new(Mutex::new(())),
        };
        for (remote, host, method, headers, body) in [
            (probe_remote, PORTAL_HOST, "POST", "", &b""[..]),
            (
                probe_remote,
                PROBE_HOST,
                "GET",
                "Cookie: yudear=fake-cookie\r\n",
                &b""[..],
            ),
            (probe_remote, PROBE_HOST, "GET", "", &b"fake-password"[..]),
            (portal_remote, PROBE_HOST, "POST", "", &b""[..]),
        ] {
            assert_eq!(
                client
                    .request_to(remote, host, method, PROBE_PATH, headers, body, true)
                    .err(),
                Some(PortalError::ContextUnknown)
            );
        }
        for listener in [&portal, &probe] {
            assert_eq!(
                listener.accept().unwrap_err().kind(),
                std::io::ErrorKind::WouldBlock
            );
        }
    }
    #[test]
    fn rejects_redirect_without_following_it() {
        assert!(parse_http(
            b"HTTP/1.1 302 Found\r\nLocation: http://elsewhere/\r\nContent-Length: 0\r\n\r\n"
        )
        .is_err());
    }
    #[test]
    fn nonempty_malformed_response_is_not_a_transient_network_error() {
        assert_eq!(parse_http(b"garbage").err(), Some(PortalError::Unexpected));
    }
    #[test]
    fn parses_chunked_body_and_rejects_truncation() {
        assert_eq!(
            parse_http(
                b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\ntest\r\n0\r\n\r\n"
            )
            .unwrap()
            .body,
            "test"
        );
        assert!(parse_http(b"HTTP/1.1 200 OK\r\nContent-Length: 8\r\n\r\ntest").is_err());
    }
    #[test]
    fn status_uses_bound_interface_and_csrf_cookie() {
        let page: &'static str =
            Box::leak(ok("/account/status /account/redial /account/check").into_boxed_str());
        let session: &'static str = Box::leak(format!("HTTP/1.1 200 OK\r\nSet-Cookie: yudear=abc; Path=/\r\nContent-Length: 22\r\n\r\n{{\"csrf_token\":\"12345\"}}").into_boxed_str());
        let status: &'static str = Box::leak(
            ok(r#"{"code":0,"online":{},"enableDial":true,"dialCode":"ok:dialup","dialMsg":"ok"}"#)
                .into_boxed_str(),
        );
        let (client, task) = serve(vec![page, session, status]);
        assert_eq!(
            client.status().unwrap(),
            PortalStatus::Online(DialState::Connected)
        );
        let requests = task.join().unwrap();
        assert!(requests[2].contains("X-CSRF-Token: 12345\r\n"));
        assert!(requests[2].contains("Cookie: yudear=abc\r\n"));
    }

    #[test]
    fn login_uses_captive_wireless_nas_instead_of_the_unrelated_default_route() {
        let page = Box::leak(ok("/account/status /account/redial /account/check").into_boxed_str());
        let first = "HTTP/1.1 302 Found\r\nLocation: http://172.30.21.100/api/r/52?acip=172.30.1.223&acname=WHUT-Bras-ME60-A&userip=127.0.0.1&wlanacname=\r\nContent-Length: 0\r\n\r\n";
        let second = "HTTP/1.1 302 Found\r\nLocation: http://172.30.21.100/tpl/isp/login.html?ip=127.0.0.1&nasId=52&acip=172.30.1.223&wlanacname=\r\nContent-Length: 0\r\n\r\n";
        let session = "HTTP/1.1 200 OK\r\nSet-Cookie: yudear=abc; Path=/\r\nContent-Length: 22\r\n\r\n{\"csrf_token\":\"12345\"}";
        let check = Box::leak(ok(r#"{"code":0,"isChangePwd":0}"#).into_boxed_str());
        let login = Box::leak(ok(r#"{"code":0}"#).into_boxed_str());
        let status = Box::leak(ok(r#"{"code":0,"online":{},"enableDial":false}"#).into_boxed_str());
        let (client, task) = serve(vec![
            page, first, second, session, check, login, page, session, status,
        ]);
        assert_eq!(
            client.login("fake-user", "fake-password"),
            Ok(PortalStatus::Online(DialState::Disabled))
        );
        let requests = task.join().unwrap();
        assert!(requests[1].starts_with("GET /connecttest.txt "));
        assert!(requests[1].contains("Host: www.msftconnecttest.com\r\n"));
        assert!(!requests[1].contains("Cookie:"));
        assert!(!requests[1].contains("fake-user"));
        assert!(requests[2].starts_with(
            "GET /api/r/52?acip=172.30.1.223&acname=WHUT-Bras-ME60-A&userip=127.0.0.1&wlanacname= "
        ));
        assert!(!requests
            .iter()
            .any(|request| request.contains("/api/r/default")));
        for req in [&requests[4], &requests[5]] {
            assert!(req
                .contains("username=fake-user&password=fake-password&userIpv4=127.0.0.1&nasId=52"));
        }
    }

    #[test]
    fn a_successful_connectivity_probe_does_not_authorize_login() {
        let page = Box::leak(ok("/account/status /account/redial /account/check").into_boxed_str());
        let probe = Box::leak(ok("Microsoft Connect Test").into_boxed_str());
        let (client, task) = serve(vec![page, probe]);
        assert_eq!(
            client.login("fake-user", "fake-password"),
            Err(PortalError::ContextUnknown)
        );
        assert!(task
            .join()
            .unwrap()
            .iter()
            .all(|request| request.starts_with("GET ")));
    }

    #[test]
    fn unsafe_authentication_context_never_sends_credentials() {
        let page: &'static str =
            Box::leak(ok("/account/status /account/redial /account/check").into_boxed_str());
        let first = "HTTP/1.1 302 Found\r\nLocation: http://172.30.21.100/api/r/14?userip=127.0.0.1\r\nContent-Length: 0\r\n\r\n";
        for location in [
            "http://evil.example/api/r/14?userip=127.0.0.1",
            "http://172.30.21.100.evil.example/api/r/14?userip=127.0.0.1",
            "http://172.30.21.100@evil.example/api/r/14?userip=127.0.0.1",
            "http://172.30.21.100:81/api/r/14?userip=127.0.0.1",
            "https://172.30.21.100/api/r/14?userip=127.0.0.1",
            "//evil.example/api/r/14?userip=127.0.0.1",
            "/api/r/../14",
            "/api/r/isp",
            "/api/r/14?extra=1",
            "/api/r/123456789012345678901",
            "http://172.30.21.100/api/r/default?userip=127.0.0.1",
            "http://172.30.21.100/api/r/14?userip=127.0.0.2",
            "http://172.30.21.100/api/r/14?userip=127.0.0.1&userip=127.0.0.1",
            "http://172.30.21.100/api/r/14?userip=127.0.0.1&%75serip=127.0.0.1",
            "http://172.30.21.100/api/r/14?userip=127.0.0.1&acname=",
            "http://172.30.21.100/api/r/14?userip=&acname=WHUT",
            "http://172.30.21.100/api/r/14?userip=127.0.0.1&acname=%0d%0aCookie%3Abad",
            "http://172.30.21.100/api/r/14?userip=127.0.0.1#fragment",
        ] {
            let redirect: &'static str = Box::leak(
                format!("HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\n\r\n")
                    .into_boxed_str(),
            );
            let (client, task) = serve(vec![page, redirect]);
            assert_eq!(
                client.login("fake-user", "fake-password"),
                Err(PortalError::ContextUnknown)
            );
            assert!(task.join().unwrap().iter().all(|r| r.starts_with("GET ")));
        }
        for location in [
            "/tpl/whut/login.html?ip=127.0.0.2&nasId=14",
            "/tpl/whut/login.html?ip=127.0.0.1&nasId=15",
            "/tpl/whut/login.html?ip=127.0.0.1&nasId=14&nasId=14",
            "/tpl/whut/login.html?ip=127.0.0.1&ip=127.0.0.1&nasId=14",
            "/tpl/whut/login.html?ip=127.0.0.1",
            "http://evil.example/tpl/whut/login.html?ip=127.0.0.1&nasId=14",
            "http://172.30.21.100.evil.example/tpl/whut/login.html?ip=127.0.0.1&nasId=14",
            "/tpl/whut/login.html?ip=127.0.0.1&nasId=14#fragment",
            "/tpl/isp/login.html?ip=127.0.0.1&nasId=14&acname=",
            "/tpl/isp/login.html?ip=127.0.0.1&nasId=14&%6easId=14",
            "/tpl/other/login.html?ip=127.0.0.1&nasId=14",
        ] {
            let redirect: &'static str = Box::leak(
                format!("HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\n\r\n")
                    .into_boxed_str(),
            );
            let (client, task) = serve(vec![page, first, redirect]);
            assert_eq!(
                client.login("fake-user", "fake-password"),
                Err(PortalError::ContextUnknown)
            );
            assert!(task.join().unwrap().iter().all(|r| r.starts_with("GET ")));
        }
    }
}

#[cfg(test)]
mod login_flow_tests {
    use super::*;
    use std::net::TcpListener;
    use std::thread;
    #[test]
    fn checks_password_change_before_login_and_rechecks_status() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let client = PortalClient {
            local: Ipv4Addr::LOCALHOST,
            remote: SocketAddrV4::new(Ipv4Addr::LOCALHOST, listener.local_addr().unwrap().port()),
            interface_index: None,
            guard: Box::new(|| true),
            gate: Arc::new(Mutex::new(())),
        };
        let worker = thread::spawn(move || {
            let bodies = [
                "/account/status /account/redial /account/check",
                "REDIRECT_PROBE",
                "REDIRECT_NAS",
                "{\"csrf_token\":\"test\"}",
                "{\"code\":0,\"isChangePwd\":0}",
                "{\"code\":0}",
                "/account/status /account/redial /account/check",
                "{\"csrf_token\":\"test\"}",
                "{\"code\":0,\"online\":{},\"enableDial\":false}",
            ];
            let mut requests = Vec::new();
            for body in bodies {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut buf = Vec::new();
                let mut block = [0; 1024];
                loop {
                    let n = stream.read(&mut block).unwrap();
                    assert_ne!(n, 0);
                    buf.extend_from_slice(&block[..n]);
                    if let Some(p) = buf.windows(4).position(|x| x == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&buf[..p]);
                        let len = head
                            .lines()
                            .find_map(|l| l.strip_prefix("Content-Length: "))
                            .unwrap()
                            .parse::<usize>()
                            .unwrap();
                        if buf.len() >= p + 4 + len {
                            break;
                        }
                    }
                }
                requests.push(String::from_utf8(buf).unwrap());
                let response = match body {
                    "REDIRECT_PROBE" => "HTTP/1.1 302 Found\r\nLocation: http://172.30.21.100/api/r/14?userip=127.0.0.1\r\nContent-Length: 0\r\n\r\n".to_owned(),
                    "REDIRECT_NAS" => "HTTP/1.1 302 Found\r\nLocation: /tpl/whut/login.html?ip=127.0.0.1&nasId=14\r\nContent-Length: 0\r\n\r\n".to_owned(),
                    _ => format!("HTTP/1.1 200 OK\r\nSet-Cookie: yudear=test; Path=/\r\nContent-Length: {}\r\n\r\n{body}", body.len()),
                };
                stream.write_all(response.as_bytes()).unwrap();
            }
            requests
        });
        assert_eq!(
            client.login("user+name", "p&ss").unwrap(),
            PortalStatus::Online(DialState::Disabled)
        );
        let requests = worker.join().unwrap();
        assert!(requests[4].starts_with("POST /api/account/check "));
        assert!(requests[5].starts_with("POST /api/account/login "));
        for i in [4, 5] {
            assert!(requests[i]
                .contains("username=user%2Bname&password=p%26ss&userIpv4=127.0.0.1&nasId=14"));
        }
        assert!(requests[8].starts_with("GET /api/account/status "));
    }
}

#[cfg(test)]
mod identity_tests {
    use super::*;
    use std::net::TcpListener;
    use std::thread;
    #[test]
    fn wrong_page_prevents_any_credential_request() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let client = PortalClient {
            local: Ipv4Addr::LOCALHOST,
            remote: SocketAddrV4::new(Ipv4Addr::LOCALHOST, listener.local_addr().unwrap().port()),
            interface_index: None,
            guard: Box::new(|| true),
            gate: Arc::new(Mutex::new(())),
        };
        let worker = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0; 1024];
            let n = stream.read(&mut request).unwrap();
            assert!(!request[..n].windows(8).any(|v| v == b"password"));
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nwrong")
                .unwrap();
        });
        assert_eq!(
            client.login("user", "password"),
            Err(PortalError::Unexpected)
        );
        worker.join().unwrap();
    }
}
#[cfg(test)]
mod guard_tests {
    use super::*;
    use std::net::TcpListener;
    use std::thread;
    #[test]
    fn changing_guard_is_rechecked_for_each_post() {
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let count = Arc::clone(&calls);
        let client = PortalClient {
            local: Ipv4Addr::LOCALHOST,
            remote: SocketAddrV4::new(Ipv4Addr::LOCALHOST, listener.local_addr().unwrap().port()),
            interface_index: None,
            guard: Box::new(move || count.fetch_add(1, Ordering::SeqCst) == 0),
            gate: Arc::new(Mutex::new(())),
        };
        let worker = thread::spawn(move || {
            let (mut check, _) = listener.accept().unwrap();
            check
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut buf = [0u8; 1024];
            assert!(check.read(&mut buf).unwrap() > 0);
            check
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}")
                .unwrap();
            drop(check);
            let (mut login, _) = listener.accept().unwrap();
            login
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            assert_eq!(login.read(&mut buf).unwrap(), 0);
        });
        assert!(client
            .request("POST", "/api/account/check", "", b"password=secret")
            .is_ok());
        assert_eq!(
            client
                .request("POST", "/api/account/login", "", b"password=secret")
                .err(),
            Some(PortalError::NetworkChanged)
        );
        worker.join().unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }
    #[test]
    fn zero_interface_index_fails_closed() {
        let socket = Socket::new(Domain::IPV4, Type::STREAM, Some(Protocol::TCP)).unwrap();
        assert!(matches!(
            bind_interface(&socket, 0),
            Err(PortalError::NetworkChanged)
        ));
    }
    #[test]
    fn rejected_guard_sends_no_credentials_or_redial_request() {
        for path in [
            "/api/account/check",
            "/api/account/login",
            "/api/account/redial",
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            let client = PortalClient {
                local: Ipv4Addr::LOCALHOST,
                remote: SocketAddrV4::new(Ipv4Addr::LOCALHOST, port),
                interface_index: None,
                guard: Box::new(|| false),
                gate: Arc::new(Mutex::new(())),
            };
            let worker = thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut buf = [0u8; 512];
                stream.read(&mut buf).unwrap()
            });
            assert_eq!(
                client.request("POST", path, "", b"password=secret").err(),
                Some(PortalError::NetworkChanged)
            );
            assert_eq!(worker.join().unwrap(), 0);
        }
    }
}
#[cfg(test)]
mod gate_tests {
    use super::*;
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};
    use std::thread;
    #[test]
    fn busy_gate_never_emits_post_bytes() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let gate = Arc::new(Mutex::new(()));
        let held = gate.lock().unwrap();
        let client = PortalClient {
            local: Ipv4Addr::LOCALHOST,
            remote: SocketAddrV4::new(Ipv4Addr::LOCALHOST, listener.local_addr().unwrap().port()),
            interface_index: None,
            guard: Box::new(|| true),
            gate: Arc::clone(&gate),
        };
        let worker = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut buf = [0u8; 512];
            stream.read(&mut buf).unwrap()
        });
        assert_eq!(
            client
                .request("POST", "/api/account/login", "", b"password=secret")
                .err(),
            Some(PortalError::NetworkChanged)
        );
        assert_eq!(worker.join().unwrap(), 0);
        drop(held);
    }
    #[test]
    fn poisoned_gate_never_emits_post_bytes() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let gate = Arc::new(Mutex::new(()));
        let poisoned = Arc::clone(&gate);
        let _ = thread::spawn(move || {
            let _held = poisoned.lock().unwrap();
            panic!("poison test mutex");
        })
        .join();
        let client = PortalClient {
            local: Ipv4Addr::LOCALHOST,
            remote: SocketAddrV4::new(Ipv4Addr::LOCALHOST, listener.local_addr().unwrap().port()),
            interface_index: None,
            guard: Box::new(|| true),
            gate,
        };
        let worker = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut buf = [0u8; 512];
            stream.read(&mut buf).unwrap()
        });
        assert_eq!(
            client.request("POST", "/api/account/redial", "", b"").err(),
            Some(PortalError::NetworkChanged)
        );
        assert_eq!(worker.join().unwrap(), 0);
    }
    #[test]
    fn guard_and_write_are_locked_but_response_wait_is_not() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let gate = Arc::new(Mutex::new(()));
        let check_gate = Arc::clone(&gate);
        let client = PortalClient {
            local: Ipv4Addr::LOCALHOST,
            remote: SocketAddrV4::new(Ipv4Addr::LOCALHOST, listener.local_addr().unwrap().port()),
            interface_index: None,
            guard: Box::new(move || check_gate.try_lock().is_err()),
            gate: Arc::clone(&gate),
        };
        let worker = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut buf = [0u8; 512];
            assert!(stream.read(&mut buf).unwrap() > 0);
            // The sender must release gate before waiting for this response.
            let _held = gate.lock().unwrap();
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}")
                .unwrap();
        });
        assert!(client
            .request("POST", "/api/account/login", "", b"password=secret")
            .is_ok());
        worker.join().unwrap();
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;

    pub const TEST_USERNAME: &str = "example-user";
    pub const TEST_PASSWORD: &str = "example-password";

    #[derive(Clone, Copy, Debug)]
    pub enum LoginOutcome {
        Success,
        SuccessResponseLost,
        WrongPassword,
        Captcha,
        ChangePassword,
        AlreadyOnline,
    }
    #[derive(Clone, Copy, Debug)]
    pub enum RedialOutcome {
        SuccessAfterPending(usize),
        SuccessResponseLost,
        Error,
        Timeout,
    }
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::thread::{self, JoinHandle};

    struct State {
        status: PortalStatus,
        after_login: PortalStatus,
        login: LoginOutcome,
        redial: RedialOutcome,
        pending: Option<usize>,
        status_lag: usize,
        deferred: Option<(usize, PortalStatus)>,
        status_failures: usize,
        requests: Vec<String>,
    }
    pub struct MockPortal {
        address: SocketAddrV4,
        state: Arc<Mutex<State>>,
        stopped: Arc<AtomicBool>,
        worker: Option<JoinHandle<()>>,
    }
    impl MockPortal {
        pub fn new(initial: PortalStatus, login: LoginOutcome, redial: RedialOutcome) -> Self {
            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
            let address = match listener.local_addr().unwrap() {
                SocketAddr::V4(addr) => addr,
                _ => unreachable!(),
            };
            listener.set_nonblocking(true).unwrap();
            let state = Arc::new(Mutex::new(State {
                status: initial,
                after_login: PortalStatus::Online(DialState::Failed),
                login,
                redial,
                pending: None,
                status_lag: 0,
                deferred: None,
                status_failures: 0,
                requests: Vec::new(),
            }));
            let stopped = Arc::new(AtomicBool::new(false));
            let server_state = Arc::clone(&state);
            let server_stop = Arc::clone(&stopped);
            let worker = thread::spawn(move || {
                while !server_stop.load(Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((stream, peer)) => {
                            assert_eq!(peer.ip(), Ipv4Addr::LOCALHOST);
                            handle(stream, &server_state);
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(5))
                        }
                        Err(e) => panic!("loopback mock accept failed: {e}"),
                    }
                }
            });
            Self {
                address,
                state,
                stopped,
                worker: Some(worker),
            }
        }
        pub fn client(&self) -> PortalClient {
            self.client_with_guard(|| true)
        }
        pub fn client_with_guard(
            &self,
            guard: impl Fn() -> bool + Send + Sync + 'static,
        ) -> PortalClient {
            PortalClient {
                local: Ipv4Addr::LOCALHOST,
                remote: self.address,
                interface_index: None,
                guard: Box::new(guard),
                gate: Arc::new(Mutex::new(())),
            }
        }
        /// Only method and path are retained; the fake credential request body is never recorded.
        pub fn set_status_lag(&self, n: usize) {
            self.state.lock().unwrap().status_lag = n;
        }
        pub fn fail_next_status_requests(&self, n: usize) {
            self.state.lock().unwrap().status_failures = n;
        }
        pub fn set_post_login_status(&self, status: PortalStatus) {
            assert!(matches!(status, PortalStatus::Online(_)));
            self.state.lock().unwrap().after_login = status;
        }
        pub fn requests(&self) -> Vec<String> {
            self.state.lock().unwrap().requests.clone()
        }
    }
    impl Drop for MockPortal {
        fn drop(&mut self) {
            self.stopped.store(true, Ordering::SeqCst);
            self.worker.take().unwrap().join().unwrap();
        }
    }
    fn handle(mut stream: TcpStream, state: &Mutex<State>) {
        // On Windows an accepted socket can inherit the listener's nonblocking
        // mode. Switch it back before reading the request to avoid a race.
        stream.set_nonblocking(false).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut bytes = Vec::new();
        let mut buf = [0; 1024];
        let body_start = loop {
            let n = match stream.read(&mut buf) {
                Ok(0) | Err(_) => return,
                Ok(n) => n,
            };
            bytes.extend_from_slice(&buf[..n]);
            if bytes.len() > 16384 {
                return;
            }
            if let Some(pos) = bytes.windows(4).position(|v| v == b"\r\n\r\n") {
                let head = String::from_utf8_lossy(&bytes[..pos]);
                let len = head
                    .lines()
                    .find_map(|line| line.strip_prefix("Content-Length: "))
                    .and_then(|s| s.parse::<usize>().ok())
                    .unwrap_or(0);
                if bytes.len() >= pos + 4 + len {
                    break pos + 4;
                }
            }
        };
        let head = match std::str::from_utf8(&bytes[..body_start]) {
            Ok(v) => v,
            Err(_) => return,
        };
        let request_line = head.lines().next().unwrap_or("");
        let mut parts = request_line.split_whitespace();
        let method = parts.next().unwrap_or("");
        let path = parts.next().unwrap_or("");
        let mut state = state.lock().unwrap();
        state.requests.push(format!("{method} {path}"));
        let redirect = match (method, path) {
            ("GET", "/api/r/default") => Some("/api/r/14"),
            ("GET", "/api/r/14") => Some("/tpl/whut/login.html?ip=127.0.0.1&nasId=14"),
            ("GET", "/connecttest.txt") => Some("http://172.30.21.100/api/r/52?acip=172.30.1.223&acname=WHUT-Bras-ME60-A&userip=127.0.0.1"),
            ("GET", "/api/r/52?acip=172.30.1.223&acname=WHUT-Bras-ME60-A&userip=127.0.0.1") => Some("/tpl/isp/login.html?ip=127.0.0.1&nasId=52"),
            _ => None,
        };
        if let Some(location) = redirect {
            drop(state);
            let _ = write!(
                stream,
                "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\n\r\n"
            );
            return;
        }
        let authorized = head.contains("X-CSRF-Token: mock-token\r\n")
            && head.contains("Cookie: yudear=mock-cookie\r\n");
        let valid_credentials = &bytes[body_start..]
            == b"username=example-user&password=example-password&userIpv4=127.0.0.1&nasId=52";
        let mut timeout = false;
        let mut drop_response = false;
        let mut reset_response = false;
        let body = match (method, path) {
            ("GET", "/tpl/isp/login_account.html") => {
                "/account/status /account/redial /account/check".to_owned()
            }
            ("GET", "/api/csrf-token") => r#"{"csrf_token":"mock-token"}"#.to_owned(),
            ("GET", "/api/account/status") if authorized && state.status_failures > 0 => {
                state.status_failures -= 1;
                reset_response = true;
                String::new()
            }
            ("GET", "/api/account/status") if authorized => {
                if let Some((remaining, target)) = state.deferred {
                    if remaining == 0 {
                        state.status = target;
                        state.deferred = None;
                    } else {
                        state.deferred = Some((remaining - 1, target));
                    }
                }
                if let Some(remaining) = state.pending {
                    if remaining == 0 {
                        state.status = PortalStatus::Online(DialState::Connected);
                        state.pending = None;
                    } else {
                        state.status = PortalStatus::Online(DialState::Pending);
                        state.pending = Some(remaining - 1);
                    }
                }
                match state.status {
                    PortalStatus::Offline => r#"{"code":1,"msg":"用户未登录"}"#,
                    PortalStatus::Online(DialState::Disabled) => r#"{"code":0,"online":{},"enableDial":false}"#,
                    PortalStatus::Online(DialState::Pending) => r#"{"code":0,"online":{},"enableDial":true,"dialCode":"","dialMsg":""}"#,
                    PortalStatus::Online(DialState::Connected) => r#"{"code":0,"online":{},"enableDial":true,"dialCode":"ok:dialup","dialMsg":"ok"}"#,
                    PortalStatus::Online(DialState::Failed) => r#"{"code":0,"online":{},"enableDial":true,"dialCode":"err:dialup","dialMsg":"failed"}"#,
                }.to_owned()
            }
            ("POST", "/api/account/check") if authorized && valid_credentials => {
                match state.login {
                    LoginOutcome::ChangePassword => r#"{"code":0,"isChangePwd":"1"}"#,
                    _ => r#"{"code":0,"isChangePwd":"0"}"#,
                }
                .to_owned()
            }
            ("POST", "/api/account/login") if authorized && valid_credentials => {
                match state.login {
                    LoginOutcome::SuccessResponseLost => {
                        state.deferred = Some((state.status_lag, state.after_login));
                        drop_response = true;
                        ""
                    }
                    LoginOutcome::Success => {
                        state.status = state.after_login;
                        r#"{"code":0}"#
                    }
                    LoginOutcome::WrongPassword => r#"{"code":1}"#,
                    LoginOutcome::Captcha => r#"{"code":2}"#,
                    LoginOutcome::ChangePassword => r#"{"code":99}"#,
                    LoginOutcome::AlreadyOnline => {
                        state.status = state.after_login;
                        r#"{"code":1,"msg":"用户已在线"}"#
                    }
                }
                .to_owned()
            }
            ("POST", "/api/account/redial") if authorized => match state.redial {
                RedialOutcome::SuccessAfterPending(n) => {
                    state.pending = Some(n);
                    r#"{"code":0}"#
                }
                RedialOutcome::SuccessResponseLost => {
                    state.deferred =
                        Some((state.status_lag, PortalStatus::Online(DialState::Connected)));
                    drop_response = true;
                    ""
                }
                RedialOutcome::Error => r#"{"code":99}"#,
                RedialOutcome::Timeout => {
                    timeout = true;
                    ""
                }
            }
            .to_owned(),
            _ => r#"{"code":99}"#.to_owned(),
        };
        drop(state);
        if reset_response {
            socket2::SockRef::from(&stream)
                .set_linger(Some(Duration::ZERO))
                .unwrap();
            return;
        }
        if drop_response {
            return;
        }
        if timeout {
            thread::sleep(Duration::from_millis(5300));
            return;
        }
        let response = format!("HTTP/1.1 200 OK\r\nSet-Cookie: yudear=mock-cookie; Path=/\r\nContent-Length: {}\r\n\r\n{body}", body.len());
        let _ = stream.write_all(response.as_bytes());
    }

    #[test]
    fn login_failures_do_not_continue_to_status_or_password_change_to_login() {
        for (outcome, expected, expect_login) in [
            (
                LoginOutcome::WrongPassword,
                PortalError::LoginFailed(AuthFailure::Unknown),
                true,
            ),
            (LoginOutcome::Captcha, PortalError::Captcha, true),
            (
                LoginOutcome::ChangePassword,
                PortalError::ManualAction,
                false,
            ),
        ] {
            let portal = MockPortal::new(PortalStatus::Offline, outcome, RedialOutcome::Error);
            assert_eq!(
                portal.client().login(TEST_USERNAME, TEST_PASSWORD),
                Err(expected)
            );
            let requests = portal.requests();
            assert_eq!(
                requests.iter().any(|p| p == "POST /api/account/login"),
                expect_login
            );
            assert!(!requests.iter().any(|p| p == "GET /api/account/status"));
            assert!(!requests
                .iter()
                .any(|p| p.contains(TEST_PASSWORD) || p.contains(TEST_USERNAME)));
        }
    }

    #[test]
    fn lost_login_response_can_leave_old_status_temporarily() {
        let portal = MockPortal::new(
            PortalStatus::Offline,
            LoginOutcome::SuccessResponseLost,
            RedialOutcome::Error,
        );
        portal.set_status_lag(2);
        portal.set_post_login_status(PortalStatus::Online(DialState::Failed));
        let client = portal.client();
        assert_eq!(
            client.login(TEST_USERNAME, TEST_PASSWORD),
            Err(PortalError::Network)
        );
        assert_eq!(client.status(), Ok(PortalStatus::Offline));
        assert_eq!(client.status(), Ok(PortalStatus::Offline));
        assert_eq!(client.status(), Ok(PortalStatus::Online(DialState::Failed)));
    }

    #[test]
    fn lost_redial_response_can_leave_old_status_temporarily() {
        let portal = MockPortal::new(
            PortalStatus::Online(DialState::Failed),
            LoginOutcome::Success,
            RedialOutcome::SuccessResponseLost,
        );
        portal.set_status_lag(2);
        let client = portal.client();
        assert_eq!(client.redial(), Err(PortalError::Network));
        assert_eq!(client.status(), Ok(PortalStatus::Online(DialState::Failed)));
        assert_eq!(client.status(), Ok(PortalStatus::Online(DialState::Failed)));
        assert_eq!(
            client.status(),
            Ok(PortalStatus::Online(DialState::Connected))
        );
    }

    #[test]
    fn status_network_failure_does_not_advance_pending_transition() {
        let portal = MockPortal::new(
            PortalStatus::Online(DialState::Failed),
            LoginOutcome::Success,
            RedialOutcome::SuccessResponseLost,
        );
        portal.set_status_lag(1);
        portal.fail_next_status_requests(1);
        let client = portal.client();
        assert_eq!(client.redial(), Err(PortalError::Network));
        assert_eq!(client.status(), Err(PortalError::Network));
        assert_eq!(client.status(), Ok(PortalStatus::Online(DialState::Failed)));
        assert_eq!(
            client.status(),
            Ok(PortalStatus::Online(DialState::Connected))
        );
    }

    #[test]
    fn successful_login_can_report_connected_operator() {
        let portal = MockPortal::new(
            PortalStatus::Offline,
            LoginOutcome::Success,
            RedialOutcome::Error,
        );
        portal.set_post_login_status(PortalStatus::Online(DialState::Connected));
        assert_eq!(
            portal.client().login(TEST_USERNAME, TEST_PASSWORD),
            Ok(PortalStatus::Online(DialState::Connected))
        );
    }

    #[test]
    fn already_online_response_is_verified_using_real_status_request() {
        let portal = MockPortal::new(
            PortalStatus::Offline,
            LoginOutcome::AlreadyOnline,
            RedialOutcome::Error,
        );
        portal.set_post_login_status(PortalStatus::Online(DialState::Connected));
        assert_eq!(
            portal.client().login(TEST_USERNAME, TEST_PASSWORD),
            Ok(PortalStatus::Online(DialState::Connected))
        );
        assert!(portal
            .requests()
            .iter()
            .any(|p| p == "GET /api/account/status"));
    }

    #[test]
    fn lost_login_response_can_leave_server_logged_in() {
        let portal = MockPortal::new(
            PortalStatus::Offline,
            LoginOutcome::SuccessResponseLost,
            RedialOutcome::Error,
        );
        portal.set_post_login_status(PortalStatus::Online(DialState::Connected));
        let client = portal.client();
        assert_eq!(
            client.login(TEST_USERNAME, TEST_PASSWORD),
            Err(PortalError::Network)
        );
        assert_eq!(
            client.status(),
            Ok(PortalStatus::Online(DialState::Connected))
        );
        assert_eq!(
            portal
                .requests()
                .iter()
                .filter(|p| p.as_str() == "POST /api/account/login")
                .count(),
            1
        );
    }

    #[test]
    fn redial_error_does_not_change_operator_state() {
        let portal = MockPortal::new(
            PortalStatus::Online(DialState::Failed),
            LoginOutcome::Success,
            RedialOutcome::Error,
        );
        let client = portal.client();
        assert_eq!(client.redial(), Err(PortalError::Unexpected));
        assert_eq!(client.status(), Ok(PortalStatus::Online(DialState::Failed)));
    }

    #[test]
    fn redial_timeout_is_reported_as_network_failure() {
        let portal = MockPortal::new(
            PortalStatus::Online(DialState::Failed),
            LoginOutcome::Success,
            RedialOutcome::Timeout,
        );
        assert_eq!(portal.client().redial(), Err(PortalError::Network));
        assert_eq!(
            portal
                .requests()
                .iter()
                .filter(|p| p.as_str() == "POST /api/account/redial")
                .count(),
            1
        );
    }

    #[test]
    fn offline_login_and_failed_redial_complete_against_loopback_server() {
        let portal = MockPortal::new(
            PortalStatus::Offline,
            LoginOutcome::Success,
            RedialOutcome::SuccessAfterPending(1),
        );
        let client = portal.client();
        assert_eq!(client.status(), Ok(PortalStatus::Offline));
        assert_eq!(
            client.login(TEST_USERNAME, TEST_PASSWORD),
            Ok(PortalStatus::Online(DialState::Failed))
        );
        assert_eq!(client.redial(), Ok(()));
        assert_eq!(
            client.status(),
            Ok(PortalStatus::Online(DialState::Pending))
        );
        assert_eq!(
            client.status(),
            Ok(PortalStatus::Online(DialState::Connected))
        );
        let requests = portal.requests();
        assert_eq!(
            requests
                .iter()
                .filter(|p| p.as_str() == "POST /api/account/login")
                .count(),
            1
        );
        assert_eq!(
            requests
                .iter()
                .filter(|p| p.as_str() == "POST /api/account/redial")
                .count(),
            1
        );
    }
}
