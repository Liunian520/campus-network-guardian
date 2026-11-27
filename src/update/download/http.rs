use std::{io, time::Duration};

pub(super) use crate::winhttp::Response;

use super::invalid;

pub(super) fn get(
    url: &super::Url,
    range: Option<(u64, u64)>,
    timeout: Duration,
) -> io::Result<Response> {
    crate::winhttp::get(&url.host, &url.path, &request_headers(url, range)?, timeout)
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
