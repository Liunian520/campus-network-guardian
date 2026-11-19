use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::io::{self, Read, Write};

const GITHUB_API: &str = "api.github.com";
const GITHUB_LATEST_PATH: &str = "/repos/Liunian520/campus-network-guardian/releases/latest";
const GITHUB_RELEASES: &str =
    "https://github.com/Liunian520/campus-network-guardian/releases/download";
const MIRROR: &str = "gh-proxy.com";
const MIRROR_LATEST_PATH: &str =
    "/https://api.github.com/repos/Liunian520/campus-network-guardian/releases/latest";
const MAX_METADATA_BYTES: u64 = 1024 * 1024;
const MAX_INSTALLER_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug)]
pub(super) struct Release {
    pub(super) version: String,
    size: u64,
    digest: [u8; 32],
}

#[derive(Deserialize)]
struct ReleaseMetadata {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    state: String,
    browser_download_url: String,
    size: u64,
    digest: Option<String>,
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn version_number(value: &str) -> Option<[u32; 3]> {
    let mut components = value.split('.');
    let mut version = [0; 3];
    for number in &mut version {
        let component = components.next()?;
        if !component.bytes().all(|byte| byte.is_ascii_digit())
            || (component.starts_with('0') && component.len() > 1)
        {
            return None;
        }
        *number = component.parse().ok()?;
    }
    components.next().is_none().then_some(version)
}

fn installer_name(version: &str) -> String {
    format!("campus-network-guardian-{version}-windows-x64-setup.exe")
}

fn installer_url(version: &str, name: &str) -> String {
    format!("{GITHUB_RELEASES}/v{version}/{name}")
}

fn parse_sha256(value: &str) -> Option<[u8; 32]> {
    let hex = value.strip_prefix("sha256:")?;
    if hex.len() != 64 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let mut digest = [0; 32];
    for (i, byte) in digest.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(digest)
}

fn parse_release(current: &str, body: &[u8]) -> io::Result<Option<Release>> {
    let current = version_number(current).ok_or_else(|| invalid("invalid current version"))?;
    let release: ReleaseMetadata =
        serde_json::from_slice(body).map_err(|_| invalid("invalid GitHub release metadata"))?;
    if release.draft || release.prerelease {
        return Ok(None);
    }
    let version = release
        .tag_name
        .strip_prefix('v')
        .ok_or_else(|| invalid("invalid GitHub release tag"))?;
    let number =
        version_number(version).ok_or_else(|| invalid("invalid GitHub release version"))?;
    if number <= current {
        return Ok(None);
    }

    let name = installer_name(version);
    let mut installers = release.assets.iter().filter(|asset| asset.name == name);
    let installer = installers
        .next()
        .ok_or_else(|| invalid("release does not include the expected installer"))?;
    if installers.next().is_some()
        || installer.state != "uploaded"
        || installer.browser_download_url != installer_url(version, &name)
        || !(1..=MAX_INSTALLER_BYTES).contains(&installer.size)
    {
        return Err(invalid("invalid installer metadata"));
    }
    let digest = installer
        .digest
        .as_deref()
        .and_then(parse_sha256)
        .ok_or_else(|| invalid("installer lacks a valid GitHub SHA-256 digest"))?;
    Ok(Some(Release {
        version: version.to_string(),
        size: installer.size,
        digest,
    }))
}

fn read_metadata(host: &str, path: &str) -> io::Result<Vec<u8>> {
    let mut body = Vec::new();
    https::get(host, path, "application/vnd.github+json")?
        .take(MAX_METADATA_BYTES + 1)
        .read_to_end(&mut body)?;
    if body.len() as u64 > MAX_METADATA_BYTES {
        return Err(invalid("GitHub release metadata is too large"));
    }
    Ok(body)
}

pub(super) fn latest(current: &str) -> io::Result<Option<Release>> {
    // The mirror is a discovery hint. Only the independent HTTPS response
    // from api.github.com can authorize a newer release and its exact bytes.
    let mirror =
        read_metadata(MIRROR, MIRROR_LATEST_PATH).and_then(|body| parse_release(current, &body));
    if matches!(mirror, Ok(None)) {
        return Ok(None);
    }
    let official = read_metadata(GITHUB_API, GITHUB_LATEST_PATH)?;
    parse_release(current, &official)
}

fn verify_installer(
    release: &Release,
    reader: impl Read,
    output: &mut impl Write,
) -> io::Result<()> {
    let mut reader = reader.take(release.size + 1);
    let mut buffer = [0; 32 * 1024];
    let mut checksum = Sha256::new();
    let mut total = 0_u64;
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        total += count as u64;
        if total > release.size {
            return Err(invalid("installer exceeds the size published by GitHub"));
        }
        checksum.update(&buffer[..count]);
        output.write_all(&buffer[..count])?;
    }
    if total != release.size || checksum.finalize()[..] != release.digest {
        return Err(invalid("installer bytes do not match the GitHub release"));
    }
    Ok(())
}

pub(super) fn download(release: &Release, output: &mut impl Write) -> io::Result<()> {
    let name = installer_name(&release.version);
    let path = format!("/{}", installer_url(&release.version, &name));
    let reader = https::get(MIRROR, &path, "application/octet-stream")?;
    verify_installer(release, reader, output)
}

pub(super) fn verify(release: &Release, input: &mut impl Read) -> io::Result<()> {
    verify_installer(release, input, &mut io::sink())
}

mod https {
    use std::ffi::c_void;
    use std::io::{self, Read};
    use std::mem::size_of;
    use std::ptr::{null, null_mut};
    use std::time::{Duration, Instant};
    use windows_sys::Win32::Networking::WinHttp::*;

    struct Handle(*mut c_void);

    impl Handle {
        fn new(raw: *mut c_void) -> io::Result<Self> {
            if raw.is_null() {
                Err(io::Error::last_os_error())
            } else {
                Ok(Self(raw))
            }
        }

        fn option(&self, option: u32, value: u32) -> io::Result<()> {
            // SAFETY: the handle is valid and each supported option accepts a DWORD.
            check(unsafe {
                WinHttpSetOption(
                    self.0,
                    option,
                    (&value as *const u32).cast(),
                    size_of::<u32>() as u32,
                )
            })
        }
    }

    impl Drop for Handle {
        fn drop(&mut self) {
            // SAFETY: the handle is valid and is owned exclusively by this value.
            unsafe { WinHttpCloseHandle(self.0) };
        }
    }

    pub(super) struct Response {
        // Close handles in child-before-parent order.
        request: Handle,
        _connection: Handle,
        _session: Handle,
        deadline: Instant,
    }

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(Some(0)).collect()
    }

    fn check(ok: i32) -> io::Result<()> {
        if ok == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    pub(super) fn get(host: &str, path: &str, accept: &str) -> io::Result<Response> {
        let deadline = Instant::now() + Duration::from_secs(120);
        let agent = wide("CampusNetworkGuardian/updates");
        let host = wide(host);
        let path = wide(path);
        let method = wide("GET");
        let headers = wide(&format!("Accept: {accept}\r\nCache-Control: no-cache\r\n"));

        // SAFETY: the UTF-16 strings and buffers remain valid throughout the
        // synchronous calls. The owned handles are dropped on all error paths.
        unsafe {
            let session = Handle::new(WinHttpOpen(
                agent.as_ptr(),
                WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
                null(),
                null(),
                0,
            ))?;
            check(WinHttpSetTimeouts(session.0, 5_000, 5_000, 8_000, 8_000))?;
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
            check(WinHttpSendRequest(
                request.0,
                headers.as_ptr(),
                (headers.len() - 1) as u32,
                null(),
                0,
                0,
                0,
            ))?;
            check(WinHttpReceiveResponse(request.0, null_mut()))?;
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
            if status != 200 {
                return Err(io::Error::other(format!(
                    "update server returned HTTP {status}"
                )));
            }
            Ok(Response {
                request,
                _connection: connection,
                _session: session,
                deadline,
            })
        }
    }

    impl Read for Response {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            if Instant::now() >= self.deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "update download timed out",
                ));
            }
            let length = buffer.len().min(64 * 1024) as u32;
            if length == 0 {
                return Ok(0);
            }
            let mut received = 0_u32;
            // SAFETY: the buffer is valid and checked to be at least length bytes long.
            check(unsafe {
                WinHttpReadData(
                    self.request.0,
                    buffer.as_mut_ptr().cast(),
                    length,
                    &mut received,
                )
            })?;
            Ok(received as usize)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    const TEST_DIGEST: &str =
        "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    fn metadata() -> Value {
        let name = "campus-network-guardian-0.10.0-windows-x64-setup.exe";
        json!({
            "tag_name": "v0.10.0",
            "draft": false,
            "prerelease": false,
            "assets": [{
                "name": name,
                "size": 3,
                "state": "uploaded",
                "browser_download_url": format!(
                    "https://github.com/Liunian520/campus-network-guardian/releases/download/v0.10.0/{name}"
                ),
                "digest": TEST_DIGEST,
            }],
        })
    }

    fn parse(current: &str, content: &Value) -> io::Result<Option<Release>> {
        parse_release(current, &serde_json::to_vec(content).unwrap())
    }

    #[test]
    fn selects_newer_stable_version_numerically() {
        let release = parse("0.9.9", &metadata()).unwrap().unwrap();
        assert_eq!(release.version, "0.10.0");
        assert_eq!(release.size, 3);
        assert!(parse("0.10.0", &metadata()).unwrap().is_none());
        assert!(parse("0.11.0", &metadata()).unwrap().is_none());
    }

    #[test]
    fn ignores_drafts_and_prereleases() {
        for flag in ["draft", "prerelease"] {
            let mut release = metadata();
            release[flag] = json!(true);
            assert!(parse("0.5.1", &release).unwrap().is_none());
        }
    }

    #[test]
    fn rejects_tags_outside_the_stable_numeric_format() {
        for tag in ["v0.10.0-rc.1", "v0.010.0", "v0.10.0/a", "0.10.0", "v.10.0"] {
            let mut release = metadata();
            release["tag_name"] = json!(tag);
            assert!(parse("0.5.1", &release).is_err(), "{tag}");
        }
    }

    #[test]
    fn rejects_asset_without_a_unique_exact_name_size_url_and_digest() {
        for (field, value) in [
            (
                "name",
                json!("Campus-network-guardian-0.10.0-windows-x64-setup.exe"),
            ),
            ("state", json!("starter")),
            ("size", json!(0)),
            ("size", json!(1_u64 << 32)),
            ("digest", Value::Null),
            ("digest", json!("sha256:deadbeef")),
            (
                "browser_download_url",
                json!("https://example.com/setup.exe"),
            ),
        ] {
            let mut release = metadata();
            release["assets"][0][field] = value;
            assert!(parse("0.5.1", &release).is_err(), "{field}");
        }
        let mut release = metadata();
        let duplicate = release["assets"][0].clone();
        release["assets"].as_array_mut().unwrap().push(duplicate);
        assert!(parse("0.5.1", &release).is_err());
    }

    #[test]
    fn verifies_exact_size_and_hash_before_accepting_download() {
        let release = parse("0.5.1", &metadata()).unwrap().unwrap();
        let mut output = Vec::new();
        verify_installer(&release, b"abc".as_slice(), &mut output).unwrap();
        assert_eq!(output, b"abc");
        for bad_bytes in [b"abd".as_slice(), b"ab", b"abcd"] {
            assert!(verify_installer(&release, bad_bytes, &mut Vec::new()).is_err());
        }
    }
}
