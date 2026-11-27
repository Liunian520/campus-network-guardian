use std::{
    ffi::c_void,
    io::{self, Read},
    mem::size_of,
    ptr::{null, null_mut},
    time::{Duration, Instant},
};
use windows_sys::Win32::Networking::WinHttp::*;

const MAX_HEADER_BYTES: usize = 16 * 1024;

pub(super) struct Response {
    pub status: u16,
    pub location: Option<String>,
    pub content_range: Option<String>,
    pub content_length: Option<u64>,
    pub content_encoding: Option<String>,
    pub reader: Box<dyn Read + Send>,
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn request_headers(url: &super::Url, range: Option<(u64, u64)>) -> io::Result<String> {
    let mut headers = String::from(
        "Accept: application/octet-stream, application/json\r\nAccept-Encoding: identity\r\n",
    );
    let endpoint = url.path.split('?').next().unwrap_or("");
    let metadata = (url.host == "app.liunianai.cn" && endpoint == "/api/v1/app/check-update")
        || (url.host == "api.github.com"
            && url.path == "/repos/Liunian520/campus-network-guardian/releases/latest");
    if metadata {
        headers.push_str("Cache-Control: no-cache\r\n");
    }
    if let Some((start, end)) = range {
        if start > end {
            return Err(invalid("invalid update byte range"));
        }
        headers.push_str(&format!("Range: bytes={start}-{end}\r\n"));
    }
    Ok(headers)
}

fn content_length(value: Option<&str>) -> io::Result<Option<u64>> {
    value
        .map(|value| {
            if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err(invalid("invalid update content length"));
            }
            value
                .parse()
                .map_err(|_| invalid("invalid update content length"))
        })
        .transpose()
}

struct Handle(*mut c_void);

// Each handle belongs to one response. Its caller starts only one operation at
// a time and waits for completion before starting another. Moving the response
// transfers its complete handle tree; callers never share handles.
unsafe impl Send for Handle {}

impl Handle {
    fn new(raw: *mut c_void) -> io::Result<Self> {
        if raw.is_null() {
            Err(io::Error::last_os_error())
        } else {
            Ok(Self(raw))
        }
    }

    fn option(&self, option: u32, value: u32) -> io::Result<()> {
        // SAFETY: the handle is live and these options accept a DWORD value.
        check(unsafe {
            WinHttpSetOption(
                self.0,
                option,
                (&value as *const u32).cast(),
                size_of::<u32>() as u32,
            )
        })
    }

    fn header(&self, query: u32) -> io::Result<Option<String>> {
        let mut buffer = [0_u16; MAX_HEADER_BYTES / 2];
        let mut size = MAX_HEADER_BYTES as u32;
        let mut index = 0;
        // SAFETY: the fixed UTF-16 buffer is at least `size` bytes long.
        let ok = unsafe {
            WinHttpQueryHeaders(
                self.0,
                query,
                null(),
                buffer.as_mut_ptr().cast(),
                &mut size,
                &mut index,
            )
        };
        if ok == 0 {
            let error = io::Error::last_os_error();
            return if error.raw_os_error() == Some(ERROR_WINHTTP_HEADER_NOT_FOUND as i32) {
                Ok(None)
            } else {
                Err(invalid("invalid or oversized update response header"))
            };
        }
        if size as usize > MAX_HEADER_BYTES || !size.is_multiple_of(2) {
            return Err(invalid("invalid update response header"));
        }
        let mut units = &buffer[..size as usize / 2];
        if units.last() == Some(&0) {
            units = &units[..units.len() - 1];
        }
        let value = String::from_utf16(units)
            .map_err(|_| invalid("invalid update response header encoding"))?;
        if value
            .chars()
            .any(|value| matches!(value, '\0' | '\r' | '\n'))
        {
            return Err(invalid("invalid update response header"));
        }
        // Ambiguous duplicate security-relevant headers are not accepted.
        // A null buffer asks whether another instance exists without copying it.
        let mut next_size = 0;
        let next = unsafe {
            WinHttpQueryHeaders(
                self.0,
                query,
                null(),
                null_mut(),
                &mut next_size,
                &mut index,
            )
        };
        if next != 0
            || io::Error::last_os_error().raw_os_error()
                != Some(ERROR_WINHTTP_HEADER_NOT_FOUND as i32)
        {
            return Err(invalid("duplicate update response header"));
        }
        Ok(Some(value))
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: this value exclusively owns the handle.
        unsafe { WinHttpCloseHandle(self.0) };
    }
}

struct Body {
    // Fields drop in declaration order, closing children before their parents.
    request: Handle,
    _connection: Handle,
    _session: Handle,
    deadline: Instant,
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}

fn check(ok: i32) -> io::Result<()> {
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn remaining_millis(deadline: Instant, cap: u32) -> io::Result<u32> {
    let remaining = deadline
        .checked_duration_since(Instant::now())
        .filter(|duration| !duration.is_zero())
        .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "update request timed out"))?;
    Ok(remaining.as_millis().clamp(1, u128::from(cap)) as u32)
}

pub(super) fn get(
    url: &super::Url,
    range: Option<(u64, u64)>,
    timeout: Duration,
) -> io::Result<Response> {
    let headers = wide(&request_headers(url, range)?);
    let deadline = Instant::now()
        .checked_add(timeout)
        .ok_or_else(|| invalid("invalid update timeout"))?;
    let initial_timeout = remaining_millis(deadline, 5_000)? as i32;
    let agent = wide("CampusNetworkGuardian/updates");
    let host = wide(&url.host);
    let path = wide(&url.path);
    let method = wide("GET");

    // SAFETY: all pointers refer to live UTF-16 strings or checked output
    // buffers, and owned handles are released on every return path.
    unsafe {
        let session = Handle::new(WinHttpOpen(
            agent.as_ptr(),
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            null(),
            null(),
            0,
        ))?;
        check(WinHttpSetTimeouts(
            session.0,
            initial_timeout,
            initial_timeout,
            initial_timeout,
            initial_timeout,
        ))?;
        let connection = Handle::new(WinHttpConnect(
            session.0,
            host.as_ptr(),
            INTERNET_DEFAULT_HTTPS_PORT,
            0,
        ))?;
        let request = Handle::new(WinHttpOpenRequest(
            connection.0,
            method.as_ptr(),
            path.as_ptr(),
            null(),
            null(),
            null(),
            WINHTTP_FLAG_SECURE,
        ))?;
        request.option(
            WINHTTP_OPTION_REDIRECT_POLICY,
            WINHTTP_OPTION_REDIRECT_POLICY_NEVER,
        )?;
        request.option(
            WINHTTP_OPTION_DISABLE_FEATURE,
            WINHTTP_DISABLE_AUTHENTICATION | WINHTTP_DISABLE_COOKIES,
        )?;
        request.option(WINHTTP_OPTION_MAX_RESPONSE_HEADER_SIZE, 64 * 1024)?;
        request.option(
            WINHTTP_OPTION_SEND_TIMEOUT,
            remaining_millis(deadline, 8_000)?,
        )?;
        check(WinHttpSendRequest(
            request.0,
            headers.as_ptr(),
            (headers.len() - 1) as u32,
            null(),
            0,
            0,
            0,
        ))?;
        request.option(
            WINHTTP_OPTION_RECEIVE_RESPONSE_TIMEOUT,
            remaining_millis(deadline, 8_000)?,
        )?;
        request.option(
            WINHTTP_OPTION_RECEIVE_TIMEOUT,
            remaining_millis(deadline, 8_000)?,
        )?;
        check(WinHttpReceiveResponse(request.0, null_mut()))?;
        remaining_millis(deadline, 8_000)?;
        let mut status = 0_u32;
        let mut size = size_of::<u32>() as u32;
        check(WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            null(),
            (&mut status as *mut u32).cast(),
            &mut size,
            null_mut(),
        ))?;
        let status = u16::try_from(status).map_err(|_| invalid("invalid update HTTP status"))?;
        let location = request.header(WINHTTP_QUERY_LOCATION)?;
        let content_range = request.header(WINHTTP_QUERY_CONTENT_RANGE)?;
        let content_encoding = request.header(WINHTTP_QUERY_CONTENT_ENCODING)?;
        let length_header = request.header(WINHTTP_QUERY_CONTENT_LENGTH)?;
        let content_length = content_length(length_header.as_deref())?;
        Ok(Response {
            status,
            location,
            content_range,
            content_length,
            content_encoding,
            reader: Box::new(Body {
                request,
                _connection: connection,
                _session: session,
                deadline,
            }),
        })
    }
}

impl Read for Body {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        self.request.option(
            WINHTTP_OPTION_RECEIVE_TIMEOUT,
            remaining_millis(self.deadline, 8_000)?,
        )?;
        let length = buffer.len().min(64 * 1024) as u32;
        let mut received = 0;
        // SAFETY: the output buffer holds at least `length` bytes. The request
        // stays synchronous and exclusively owned until this call returns.
        check(unsafe {
            WinHttpReadData(
                self.request.0,
                buffer.as_mut_ptr().cast(),
                length,
                &mut received,
            )
        })?;
        remaining_millis(self.deadline, 8_000)?;
        if received > length {
            return Err(invalid("invalid update response length"));
        }
        Ok(received as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binary_url() -> super::super::Url {
        super::super::Url {
            host: "r2.lnyun.site".into(),
            path: "/installer.zip?signature=example".into(),
        }
    }

    #[test]
    fn anonymous_range_request_uses_identity_encoding() {
        let headers = request_headers(&binary_url(), Some((17, 41))).unwrap();
        assert!(headers.contains("Accept-Encoding: identity\r\n"));
        assert!(headers.contains("Range: bytes=17-41\r\n"));
        assert!(!headers.contains("Authorization:"));
        assert!(!headers.contains("Cookie:"));
    }

    #[test]
    fn whole_request_omits_range_header() {
        let headers = request_headers(&binary_url(), None).unwrap();
        assert!(!headers.contains("Range:"));
        assert!(headers.ends_with("\r\n"));
    }

    #[test]
    fn rejects_reversed_range() {
        assert!(request_headers(&binary_url(), Some((41, 17))).is_err());
    }

    #[test]
    fn parses_content_length_without_accepting_ambiguous_values() {
        assert_eq!(content_length(None).unwrap(), None);
        assert_eq!(content_length(Some("2097152")).unwrap(), Some(2097152));
        for bad in ["", " 3", "3 ", "+3", "3,3", "-1", "18446744073709551616"] {
            assert!(content_length(Some(bad)).is_err(), "{bad}");
        }
    }

    #[test]
    fn trusted_metadata_requests_bypass_stale_cache() {
        for (host, path) in [
            (
                "app.liunianai.cn",
                "/api/v1/app/check-update?package=example&version_code=6007",
            ),
            (
                "api.github.com",
                "/repos/Liunian520/campus-network-guardian/releases/latest",
            ),
        ] {
            let url = super::super::Url {
                host: host.into(),
                path: path.into(),
            };
            assert!(request_headers(&url, None)
                .unwrap()
                .contains("Cache-Control: no-cache\r\n"));
        }
    }

    #[test]
    fn binary_and_range_requests_preserve_cdn_caching() {
        for (host, path) in [
            ("r2.lnyun.site", "/installer.zip?signature=example"),
            ("cfile.lnyun.site", "/installer.zip"),
            ("app.liunianai.cn", "/api/v1/app/download/275"),
            ("github.com", "/Liunian520/campus-network-guardian/releases/download/v0.10.0/setup.exe"),
            ("gh-proxy.com", "/https://github.com/Liunian520/campus-network-guardian/releases/download/v0.10.0/setup.exe"),
            ("r2.lnyun.site", "/api/v1/app/check-update?version_code=6007"),
            ("app.liunianai.cn", "/api/v1/app/check-update/file.zip"),
        ] {
            let url = super::super::Url { host: host.into(), path: path.into() };
            for range in [None, Some((0, 1023))] {
                let headers = request_headers(&url, range).unwrap();
                assert!(!headers.contains("Cache-Control:"), "{host}{path}: {range:?}");
            }
        }
    }
}
