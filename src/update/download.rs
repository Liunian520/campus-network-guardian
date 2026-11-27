use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{self, Cursor, Read, Seek, SeekFrom, Write},
    time::{Duration, Instant},
};

const GITHUB_API: &str = "api.github.com";
const GITHUB_LATEST_PATH: &str = "/repos/Liunian520/campus-network-guardian/releases/latest";
const GITHUB_RELEASES: &str =
    "https://github.com/Liunian520/campus-network-guardian/releases/download";
const MIRROR: &str = "gh-proxy.com";
const SERVICE: &str = "app.liunianai.cn";
const SERVICE_PACKAGE: &str = "cn.liunianai.campusnetworkguardian.installerzip";
const MAX_METADATA_BYTES: u64 = 1024 * 1024;
const MAX_INSTALLER_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Clone, Debug)]
pub(super) struct Release {
    pub(super) version: String,
    #[cfg(test)]
    size: u64,
    #[cfg(test)]
    digest: [u8; 32],
    candidates: Vec<Candidate>,
    official: Option<VerifiedInstaller>,
    details: ReleaseDetails,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct ReleaseDetails {
    pub(super) version: String,
    pub(super) source: String,
    pub(super) changelog: String,
}

#[derive(Debug)]
pub(super) struct CheckDetails {
    pub(super) release: Option<Release>,
    pub(super) details: Option<ReleaseDetails>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct VerifiedInstaller {
    size: u64,
    digest: [u8; 32],
    version: String,
    source: String,
}

impl VerifiedInstaller {
    pub(super) fn version(&self) -> &str {
        &self.version
    }
    pub(super) fn source(&self) -> &str {
        &self.source
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Payload {
    Installer,
    Zip,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Family {
    Service,
    Github,
    Mirror,
    Metadata,
}

#[derive(Clone, Debug)]
struct Candidate {
    url: Url,
    resolved: Option<Url>,
    payload: Payload,
    family: Family,
    size: u64,
    digest: [u8; 32],
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Url {
    host: String,
    path: String,
}

mod http;

trait Transport: Sync {
    fn get(
        &self,
        url: &Url,
        range: Option<(u64, u64)>,
        timeout: Duration,
    ) -> io::Result<http::Response>;
}

struct Network;
impl Transport for Network {
    fn get(
        &self,
        url: &Url,
        range: Option<(u64, u64)>,
        timeout: Duration,
    ) -> io::Result<http::Response> {
        http::get(url, range, timeout)
    }
}

const MAX_REDIRECTS: usize = 5;
const PROBE_TIMEOUT: Duration = Duration::from_secs(16);
const METADATA_TIMEOUT: Duration = Duration::from_secs(15);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(120);
const RANGE_THRESHOLD: u64 = 1024 * 1024;
const PROBE_BYTES: u64 = 64 * 1024;

fn parse_url(value: &str) -> io::Result<Url> {
    if value.len() > 16 * 1024
        || !value.bytes().all(|byte| (0x21..=0x7e).contains(&byte))
        || value.contains(['\\', '#'])
    {
        return Err(invalid("invalid update redirect URL"));
    }
    let https = value
        .strip_prefix("https://")
        .ok_or_else(|| invalid("update redirects require HTTPS"))?;
    let (host, path) = https.split_once('/').map_or((https, "/"), |(host, path)| {
        (host, &value[value.len() - path.len() - 1..])
    });
    if host.is_empty()
        || !host.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'-')
        })
    {
        return Err(invalid("invalid update redirect authority"));
    }
    Ok(Url {
        host: host.into(),
        path: path.into(),
    })
}

fn allowed_redirect(family: Family, url: &Url) -> bool {
    match family {
        Family::Service => url.host == SERVICE || service_cdn(url),
        Family::Github | Family::Mirror => {
            matches!(
                url.host.as_str(),
                "github.com"
                    | "release-assets.githubusercontent.com"
                    | "objects.githubusercontent.com"
            ) || (family == Family::Mirror && url.host == MIRROR)
        }
        Family::Metadata => false,
    }
}

fn service_cdn(url: &Url) -> bool {
    matches!(url.host.as_str(), "r2.lnyun.site" | "cfile.lnyun.site")
}

fn remaining(deadline: Instant) -> io::Result<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|duration| !duration.is_zero())
        .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "update operation timed out"))
}

fn fetch(
    transport: &impl Transport,
    initial: &Url,
    family: Family,
    range: Option<(u64, u64)>,
    deadline: Instant,
) -> io::Result<(http::Response, Url)> {
    let mut url = initial.clone();
    for redirects in 0..=MAX_REDIRECTS {
        let response = transport.get(&url, range, remaining(deadline)?)?;
        remaining(deadline)?;
        if !matches!(response.status, 301 | 302 | 303 | 307 | 308) {
            return Ok((response, url));
        }
        if redirects == MAX_REDIRECTS {
            return Err(invalid("too many update redirects"));
        }
        let location = response
            .location
            .as_deref()
            .ok_or_else(|| invalid("update redirect lacks location"))?;
        let target = if location.starts_with('/') && !location.starts_with("//") {
            parse_url(&format!("https://{}{location}", url.host))?
        } else {
            parse_url(location)?
        };
        if !allowed_redirect(family, &target) {
            return Err(invalid("untrusted update redirect host"));
        }
        url = target;
    }
    Err(invalid("too many update redirects"))
}

fn identity(response: &http::Response) -> io::Result<()> {
    if response
        .content_encoding
        .as_deref()
        .is_some_and(|encoding| !encoding.eq_ignore_ascii_case("identity"))
    {
        return Err(invalid("update response used unsupported content encoding"));
    }
    Ok(())
}

fn validate_range(response: &http::Response, start: u64, end: u64, total: u64) -> io::Result<()> {
    identity(response)?;
    if response.status != 206
        || response.content_range.as_deref() != Some(&format!("bytes {start}-{end}/{total}"))
        || response
            .content_length
            .is_some_and(|length| length != end - start + 1)
    {
        return Err(invalid("update server did not honor the exact byte range"));
    }
    Ok(())
}

fn read_exact_body(reader: impl Read, expected: u64, deadline: Instant) -> io::Result<Vec<u8>> {
    if expected > MAX_INSTALLER_BYTES {
        return Err(invalid("update response exceeds size limit"));
    }
    let mut input = reader.take(expected + 1);
    let mut data = Vec::with_capacity(expected as usize);
    let mut buffer = [0; 32 * 1024];
    loop {
        remaining(deadline)?;
        let count = input.read(&mut buffer)?;
        remaining(deadline)?;
        if count == 0 {
            break;
        }
        if data.len() as u64 + count as u64 > expected {
            return Err(invalid("update response exceeds its expected size"));
        }
        data.extend_from_slice(&buffer[..count]);
    }
    if data.len() as u64 != expected {
        return Err(invalid("truncated update response"));
    }
    Ok(data)
}

fn read_metadata_with(transport: &impl Transport, host: &str, path: &str) -> io::Result<Vec<u8>> {
    let deadline = Instant::now() + METADATA_TIMEOUT;
    let (response, _) = fetch(
        transport,
        &Url {
            host: host.into(),
            path: path.into(),
        },
        Family::Metadata,
        None,
        deadline,
    )?;
    identity(&response)?;
    if response.status != 200
        || response
            .content_length
            .is_some_and(|length| length > MAX_METADATA_BYTES)
    {
        return Err(invalid("invalid update metadata response"));
    }
    let mut input = response.reader.take(MAX_METADATA_BYTES + 1);
    let mut data = Vec::new();
    let mut buffer = [0; 16 * 1024];
    loop {
        remaining(deadline)?;
        let count = input.read(&mut buffer)?;
        remaining(deadline)?;
        if count == 0 {
            break;
        }
        data.extend_from_slice(&buffer[..count]);
        if data.len() as u64 > MAX_METADATA_BYTES {
            return Err(invalid("update metadata exceeds size limit"));
        }
    }
    if response
        .content_length
        .is_some_and(|length| length != data.len() as u64)
    {
        return Err(invalid("truncated update metadata"));
    }
    Ok(data)
}

fn check_details_with(current: &str, transport: &impl Transport) -> io::Result<CheckDetails> {
    let code = version_code(current).ok_or_else(|| invalid("invalid current version"))?;
    let service_path = format!("/api/v1/app/check-update?package={SERVICE_PACKAGE}&platform=windows&version_code={code}&channel=stable");
    std::thread::scope(|scope| {
        let service = scope.spawn(|| {
            read_metadata_with(transport, SERVICE, &service_path)
                .and_then(|body| parse_service(current, &body))
        });
        let github = read_metadata_with(transport, GITHUB_API, GITHUB_LATEST_PATH)
            .and_then(|body| parse_github(current, &body));
        let service = service
            .join()
            .unwrap_or_else(|_| Err(invalid("first-party update check failed")));
        select_observations(service, github)
    })
}

fn ranked_candidates(release: &Release, transport: &impl Transport) -> Vec<(Candidate, bool)> {
    let results = std::thread::scope(|scope| {
        let workers = release
            .candidates
            .iter()
            .map(|candidate| {
                scope.spawn(move || {
                    let started = Instant::now();
                    let deadline = started + PROBE_TIMEOUT;
                    let end = (candidate.size.min(PROBE_BYTES)) - 1;
                    (|| {
                        let (mut response, url) = fetch(
                            transport,
                            &candidate.url,
                            candidate.family,
                            Some((0, end)),
                            deadline,
                        )?;
                        identity(&response)?;
                        let range_supported = response.status == 206;
                        if range_supported {
                            validate_range(&response, 0, end, candidate.size)?;
                        } else if response.status != 200
                            || response
                                .content_length
                                .is_some_and(|size| size != candidate.size)
                        {
                            return Err(invalid("update node probe failed"));
                        }
                        let mut bytes = vec![0; (end + 1) as usize];
                        response.reader.read_exact(&mut bytes)?;
                        remaining(deadline)?;
                        let mut resolved = candidate.clone();
                        resolved.resolved = Some(url);
                        Ok((resolved, range_supported, started.elapsed()))
                    })()
                })
            })
            .collect::<Vec<_>>();
        workers
            .into_iter()
            .map(|worker| {
                worker
                    .join()
                    .unwrap_or_else(|_| Err(invalid("update node probe failed")))
            })
            .collect::<Vec<_>>()
    });
    let mut available = results
        .iter()
        .enumerate()
        .filter_map(|(index, result)| {
            result
                .as_ref()
                .ok()
                .map(|(candidate, ranges, elapsed)| (index, candidate.clone(), *ranges, *elapsed))
        })
        .collect::<Vec<_>>();
    // Compare measured completion estimates rather than only TLS/first-byte
    // latency: the service ZIP and official EXE can have different sizes.
    available.sort_by_key(|(index, candidate, _, elapsed)| {
        (
            elapsed.as_nanos() * u128::from(candidate.size)
                / u128::from(candidate.size.min(PROBE_BYTES)),
            *index,
        )
    });
    let mut ranked = available
        .into_iter()
        .map(|(_, candidate, ranges, _)| (candidate, ranges))
        .collect::<Vec<_>>();
    // A server may reject a probe while accepting a normal GET; keep it as a
    // final single-stream fallback without letting it delay available nodes.
    ranked.extend(
        results
            .into_iter()
            .enumerate()
            .filter_map(|(index, result)| {
                result
                    .err()
                    .map(|_| (release.candidates[index].clone(), false))
            }),
    );
    ranked
}

fn parallel_bytes(
    candidate: &Candidate,
    transport: &impl Transport,
    deadline: Instant,
) -> io::Result<(Vec<u8>, usize, bool)> {
    let threads = ((candidate.size / (1024 * 1024)) as usize).clamp(2, 4);
    let chunk = candidate.size.div_ceil(threads as u64);
    let parts = std::thread::scope(|scope| {
        let workers = (0..threads)
            .map(|index| {
                let start = index as u64 * chunk;
                let end = ((index as u64 + 1) * chunk).min(candidate.size) - 1;
                scope.spawn(move || {
                    let (response, url) = fetch(
                        transport,
                        candidate.resolved.as_ref().unwrap_or(&candidate.url),
                        candidate.family,
                        Some((start, end)),
                        deadline,
                    )?;
                    validate_range(&response, start, end, candidate.size)?;
                    let bytes = read_exact_body(response.reader, end - start + 1, deadline)?;
                    Ok((bytes, service_cdn(&url)))
                })
            })
            .collect::<Vec<_>>();
        workers
            .into_iter()
            .map(|worker| {
                worker
                    .join()
                    .unwrap_or_else(|_| Err(invalid("update range worker failed")))
            })
            .collect::<io::Result<Vec<_>>>()
    })?;
    let mut bytes = Vec::with_capacity(candidate.size as usize);
    let mut via_cdn = true;
    for (part, part_via_cdn) in parts {
        bytes.extend_from_slice(&part);
        via_cdn &= part_via_cdn;
    }
    Ok((bytes, threads, via_cdn))
}

fn reset(output: &mut File) -> io::Result<()> {
    output.set_len(0)?;
    output.seek(SeekFrom::Start(0))?;
    Ok(())
}

fn hash_copy(
    expected: &VerifiedInstaller,
    reader: impl Read,
    output: &mut impl Write,
    deadline: Instant,
) -> io::Result<()> {
    let mut input = reader.take(expected.size + 1);
    let mut checksum = Sha256::new();
    let mut total = 0u64;
    let mut buffer = [0; 32 * 1024];
    loop {
        remaining(deadline)?;
        let count = input.read(&mut buffer)?;
        remaining(deadline)?;
        if count == 0 {
            break;
        }
        total += count as u64;
        if total > expected.size {
            return Err(invalid("installer exceeds its published size"));
        }
        checksum.update(&buffer[..count]);
        output.write_all(&buffer[..count])?;
    }
    if total != expected.size || checksum.finalize()[..] != expected.digest {
        return Err(invalid("installer size or SHA-256 mismatch"));
    }
    Ok(())
}

fn install_payload(
    release: &Release,
    candidate: &Candidate,
    reader: impl Read,
    output: &mut File,
    deadline: Instant,
) -> io::Result<VerifiedInstaller> {
    let outer = VerifiedInstaller {
        size: candidate.size,
        digest: candidate.digest,
        version: release.version.clone(),
        source: String::new(),
    };
    if candidate.payload == Payload::Installer {
        hash_copy(&outer, reader, output, deadline)?;
        return Ok(outer);
    }
    let bytes = read_exact_body(reader, candidate.size, deadline)?;
    if Sha256::digest(&bytes)[..] != candidate.digest {
        return Err(invalid("installer ZIP SHA-256 mismatch"));
    }
    super::archive::extract_installer(&release.version, Cursor::new(bytes), output)
        .map_err(|_| invalid("invalid installer ZIP"))?;
    remaining(deadline)?;
    output.flush()?;
    output.rewind()?;
    let mut checksum = Sha256::new();
    let mut size = 0u64;
    let mut buffer = [0; 32 * 1024];
    loop {
        let count = output.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        size += count as u64;
        if size > MAX_INSTALLER_BYTES {
            return Err(invalid("expanded installer exceeds size limit"));
        }
        checksum.update(&buffer[..count]);
    }
    let verified = VerifiedInstaller {
        size,
        digest: checksum.finalize().into(),
        version: release.version.clone(),
        source: String::new(),
    };
    if size == 0
        || release
            .official
            .as_ref()
            .is_some_and(|official| official.size != size || official.digest != verified.digest)
    {
        return Err(invalid("installer ZIP differs from official installer"));
    }
    Ok(verified)
}

fn download_with(
    release: &Release,
    output: &mut File,
    transport: &impl Transport,
) -> io::Result<VerifiedInstaller> {
    download_with_timeout(release, output, transport, DOWNLOAD_TIMEOUT)
}

fn download_with_timeout(
    release: &Release,
    output: &mut File,
    transport: &impl Transport,
    attempt_timeout: Duration,
) -> io::Result<VerifiedInstaller> {
    let ranked = ranked_candidates(release, transport);
    for (candidate, ranges) in ranked {
        reset(output)?;
        let mut successful = None;
        if ranges && candidate.size >= RANGE_THRESHOLD {
            let deadline = Instant::now() + attempt_timeout;
            if let Ok((bytes, threads, via_cdn)) = parallel_bytes(&candidate, transport, deadline) {
                if let Ok(verified) =
                    install_payload(release, &candidate, Cursor::new(bytes), output, deadline)
                {
                    successful = Some((verified, threads, via_cdn));
                }
            }
            if successful.is_none() {
                reset(output)?;
            }
        }
        let result = if let Some(successful) = successful {
            Ok(successful)
        } else {
            (|| {
                let deadline = Instant::now() + attempt_timeout;
                let (response, url) =
                    fetch(transport, &candidate.url, candidate.family, None, deadline)?;
                identity(&response)?;
                if response.status != 200
                    || response
                        .content_length
                        .is_some_and(|size| size != candidate.size)
                {
                    return Err(invalid("invalid installer download response"));
                }
                install_payload(release, &candidate, response.reader, output, deadline)
                    .map(|verified| (verified, 1, service_cdn(&url)))
            })()
        };
        if let Ok((mut verified, threads, via_cdn)) = result {
            let label = match candidate.family {
                Family::Service if via_cdn => "流年 App · CF CDN",
                Family::Service => "流年 App",
                Family::Mirror => "GitHub 镜像",
                _ => "GitHub",
            };
            verified.source = format!(
                "{label} · {}",
                if threads == 1 {
                    "单线程".into()
                } else {
                    format!("{threads} 线程")
                }
            );
            return Ok(verified);
        }
    }
    reset(output)?;
    Err(invalid("all verified installer download routes failed"))
}

#[derive(Deserialize)]
struct ReleaseMetadata {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<Asset>,
    #[serde(default)]
    body: String,
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

#[cfg(test)]
fn parse_release(current: &str, body: &[u8]) -> io::Result<Option<Release>> {
    let value: serde_json::Value =
        serde_json::from_slice(body).map_err(|_| invalid("invalid release metadata"))?;
    if value.get("has_update").is_some() || value.get("ok").is_some() {
        return parse_service(current, body).map(|observation| observation.release);
    }
    parse_github(current, body).map(|observation| observation.release)
}

fn clean_changelog(text: &str) -> String {
    text.replace("\r\n", "\n")
        .chars()
        .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
        .take(32 * 1024)
        .collect::<String>()
        .trim()
        .to_string()
}

fn version_code(value: &str) -> Option<u64> {
    let [major, minor, patch] = version_number(value)?;
    if minor >= 1000 || patch >= 1000 {
        return None;
    }
    u64::from(major)
        .checked_mul(1_000_000)?
        .checked_add(u64::from(minor) * 1000)?
        .checked_add(u64::from(patch))
}

fn parse_service(current: &str, body: &[u8]) -> io::Result<CheckDetails> {
    let current_number =
        version_number(current).ok_or_else(|| invalid("invalid current version"))?;
    let mut value: serde_json::Value = serde_json::from_slice(body)
        .map_err(|_| invalid("invalid first-party release metadata"))?;
    if value.get("ok").is_some() {
        if value.get("ok").and_then(|v| v.as_bool()) != Some(true) {
            return Err(invalid("first-party update check failed"));
        }
        value = value
            .get("data")
            .cloned()
            .ok_or_else(|| invalid("missing release metadata"))?;
    }
    let has_update = value
        .get("has_update")
        .and_then(|v| v.as_bool())
        .ok_or_else(|| invalid("invalid first-party update status"))?;
    if value
        .get("channel")
        .is_some_and(|v| v.as_str() != Some("stable"))
        || value
            .get("platform")
            .is_some_and(|v| v.as_str() != Some("windows"))
    {
        return Err(invalid(
            "unsupported first-party release channel or platform",
        ));
    }
    let version = value.get("version_name").and_then(|v| v.as_str());
    let Some(version) = version else {
        return if has_update {
            Err(invalid("missing first-party release version"))
        } else {
            Ok(CheckDetails {
                release: None,
                details: None,
            })
        };
    };
    let number = version_number(version).ok_or_else(|| invalid("invalid first-party version"))?;
    let code = version_code(version).ok_or_else(|| invalid("invalid first-party version code"))?;
    if value.get("version_code").and_then(|v| v.as_u64()) != Some(code) {
        return Err(invalid("first-party version code mismatch"));
    }
    let details = ReleaseDetails {
        version: version.to_owned(),
        source: "流年 App".into(),
        changelog: clean_changelog(
            value
                .get("changelog")
                .and_then(|v| v.as_str())
                .unwrap_or(""),
        ),
    };
    // has_update is only a hint. Never accept a same-version rebuild or downgrade.
    if !has_update || number <= current_number {
        return Ok(CheckDetails {
            release: None,
            details: Some(details),
        });
    }
    if value.get("channel").and_then(|v| v.as_str()) != Some("stable")
        || value.get("platform").and_then(|v| v.as_str()) != Some("windows")
    {
        return Err(invalid("missing first-party release channel or platform"));
    }
    let release_id = value
        .get("release_id")
        .and_then(|v| v.as_u64())
        .filter(|id| *id > 0)
        .ok_or_else(|| invalid("invalid first-party release ID"))?;
    let expected_url = format!("https://{SERVICE}/api/v1/app/download/{release_id}");
    if value.get("download_url").and_then(|v| v.as_str()) != Some(&expected_url) {
        return Err(invalid("invalid first-party installer URL"));
    }
    let size = value
        .get("file_size")
        .and_then(|v| v.as_u64())
        .filter(|size| (1..=MAX_INSTALLER_BYTES).contains(size))
        .ok_or_else(|| invalid("invalid first-party installer size"))?;
    let digest = value
        .get("sha256")
        .and_then(|v| v.as_str())
        .filter(|value| value.len() == 64)
        .and_then(|value| parse_sha256(&format!("sha256:{value}")))
        .ok_or_else(|| invalid("first-party installer lacks a valid SHA-256 digest"))?;
    let candidate = Candidate {
        url: Url {
            host: SERVICE.into(),
            path: format!("/api/v1/app/download/{release_id}"),
        },
        resolved: None,
        payload: Payload::Zip,
        family: Family::Service,
        size,
        digest,
    };
    Ok(CheckDetails {
        release: Some(Release {
            version: version.into(),
            #[cfg(test)]
            size,
            #[cfg(test)]
            digest,
            candidates: vec![candidate],
            official: None,
            details: details.clone(),
        }),
        details: Some(details),
    })
}

fn parse_github(current: &str, body: &[u8]) -> io::Result<CheckDetails> {
    let current = version_number(current).ok_or_else(|| invalid("invalid current version"))?;
    let release: ReleaseMetadata =
        serde_json::from_slice(body).map_err(|_| invalid("invalid GitHub release metadata"))?;
    if release.draft || release.prerelease {
        return Ok(CheckDetails {
            release: None,
            details: None,
        });
    }
    let version = release
        .tag_name
        .strip_prefix('v')
        .ok_or_else(|| invalid("invalid GitHub release tag"))?;
    let number =
        version_number(version).ok_or_else(|| invalid("invalid GitHub release version"))?;
    let details = ReleaseDetails {
        version: version.into(),
        source: "GitHub".into(),
        changelog: clean_changelog(&release.body),
    };
    if number <= current {
        return Ok(CheckDetails {
            release: None,
            details: Some(details),
        });
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
    let candidates = vec![
        Candidate {
            url: Url {
                host: MIRROR.into(),
                path: format!("/{}", installer_url(version, &name)),
            },
            resolved: None,
            payload: Payload::Installer,
            family: Family::Mirror,
            size: installer.size,
            digest,
        },
        Candidate {
            url: Url {
                host: "github.com".into(),
                path: format!(
                    "/Liunian520/campus-network-guardian/releases/download/v{version}/{name}"
                ),
            },
            resolved: None,
            payload: Payload::Installer,
            family: Family::Github,
            size: installer.size,
            digest,
        },
    ];
    Ok(CheckDetails {
        release: Some(Release {
            version: version.to_string(),
            #[cfg(test)]
            size: installer.size,
            #[cfg(test)]
            digest,
            candidates,
            official: Some(VerifiedInstaller {
                size: installer.size,
                digest,
                version: version.into(),
                source: "GitHub".into(),
            }),
            details: details.clone(),
        }),
        details: Some(details),
    })
}

// Keep the narrow discovery API for callers that do not need announcement text.
#[allow(dead_code)]
pub(super) fn latest(current: &str) -> io::Result<Option<Release>> {
    check_details(current).map(|checked| checked.release)
}

pub(super) fn check_details(current: &str) -> io::Result<CheckDetails> {
    check_details_with(current, &Network)
}

fn select_observations(
    service: io::Result<CheckDetails>,
    github: io::Result<CheckDetails>,
) -> io::Result<CheckDetails> {
    let (mut service, github) = match (service, github) {
        (Err(_), Err(_)) => return Err(invalid("all trusted update sources failed")),
        (Ok(result), Err(_)) | (Err(_), Ok(result)) => return Ok(result),
        (Ok(service), Ok(github)) => (service, github),
    };
    let selected = match (service.release.take(), github.release) {
        (None, github) => github,
        (service, None) => service,
        (Some(mut service), Some(github)) => {
            match version_number(&service.version).cmp(&version_number(&github.version)) {
                std::cmp::Ordering::Less => Some(github),
                std::cmp::Ordering::Greater => Some(service),
                std::cmp::Ordering::Equal => {
                    service.candidates.extend(github.candidates);
                    service.official = github.official;
                    if service.details.changelog.is_empty() {
                        service.details.changelog = github.details.changelog;
                    }
                    Some(service)
                }
            }
        }
    };
    let details = selected
        .as_ref()
        .map(|release| release.details.clone())
        .or_else(|| match (service.details, github.details) {
            (Some(service), Some(github)) => {
                if version_number(&github.version) > version_number(&service.version)
                    || (github.version == service.version
                        && service.changelog.is_empty()
                        && !github.changelog.is_empty())
                {
                    Some(github)
                } else {
                    Some(service)
                }
            }
            (service, github) => service.or(github),
        });
    Ok(CheckDetails {
        release: selected,
        details,
    })
}

#[cfg(test)]
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

pub(super) fn download(release: &Release, output: &mut File) -> io::Result<VerifiedInstaller> {
    download_with(release, output, &Network)
}

pub(super) fn verify(verified: &VerifiedInstaller, input: &mut impl Read) -> io::Result<()> {
    hash_copy(
        verified,
        input,
        &mut io::sink(),
        Instant::now() + DOWNLOAD_TIMEOUT,
    )
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

    fn service_metadata() -> Value {
        json!({
            "has_update": true,
            "version_name": "0.10.0",
            "version_code": 10000,
            "release_id": 275,
            "platform": "windows",
            "channel": "stable",
            "file_size": 3,
            "sha256": TEST_DIGEST.trim_start_matches("sha256:"),
            "download_url": "https://app.liunianai.cn/api/v1/app/download/275"
        })
    }

    #[test]
    fn accepts_first_party_installer_zip_metadata_without_github() {
        let release = parse("0.6.7", &service_metadata()).unwrap().unwrap();
        assert_eq!(release.version, "0.10.0");
        assert_eq!(release.size, 3);
    }

    #[test]
    fn first_party_numeric_version_prevents_forced_rebuild_or_downgrade() {
        for current in ["0.10.0", "0.11.0"] {
            let mut metadata = service_metadata();
            metadata["force_update"] = json!(true);
            assert!(parse(current, &metadata).unwrap().is_none());
        }
    }

    #[test]
    fn accepts_wrapped_first_party_response() {
        let release = parse("0.6.7", &json!({"ok": true, "data": service_metadata()}))
            .unwrap()
            .unwrap();
        assert_eq!(release.version, "0.10.0");
    }

    #[test]
    fn first_party_requires_exact_windows_stable_installer_metadata() {
        for (field, bad) in [
            ("platform", json!("android")),
            ("channel", json!("beta")),
            ("version_name", json!("0.10.0-rc.1")),
            ("version_code", json!(10001)),
            ("file_size", json!(0)),
            ("file_size", json!(MAX_INSTALLER_BYTES + 1)),
            ("sha256", Value::Null),
            ("sha256", json!("sha256:deadbeef")),
            (
                "download_url",
                json!("http://app.liunianai.cn/api/v1/app/download/275"),
            ),
            (
                "download_url",
                json!("https://user:secret@app.liunianai.cn/api/v1/app/download/275"),
            ),
            (
                "download_url",
                json!("https://app.liunianai.cn/api/v1/app/download/274"),
            ),
            (
                "download_url",
                json!("https://app.liunianai.cn/api/v1/app/download/275?token=secret"),
            ),
            (
                "download_url",
                json!("https://evil.test/api/v1/app/download/275"),
            ),
        ] {
            let mut metadata = service_metadata();
            metadata[field] = bad;
            assert!(parse("0.6.7", &metadata).is_err(), "{field}");
        }
        assert!(parse("0.6.7", &json!({"ok": false, "data": service_metadata()})).is_err());
    }

    struct FakeTransport<F>(F);
    impl<F> Transport for FakeTransport<F>
    where
        F: Fn(&Url, Option<(u64, u64)>) -> io::Result<http::Response> + Sync,
    {
        fn get(
            &self,
            url: &Url,
            range: Option<(u64, u64)>,
            _: Duration,
        ) -> io::Result<http::Response> {
            (self.0)(url, range)
        }
    }

    fn response(bytes: &[u8], range: Option<(u64, u64)>, total: u64) -> http::Response {
        let (status, content_range, data) = if let Some((start, end)) = range {
            (
                206,
                Some(format!("bytes {start}-{end}/{total}")),
                bytes[start as usize..=end as usize].to_vec(),
            )
        } else {
            (200, None, bytes.to_vec())
        };
        http::Response {
            status,
            location: None,
            content_range,
            content_length: Some(data.len() as u64),
            content_encoding: Some("identity".into()),
            reader: Box::new(Cursor::new(data)),
        }
    }

    fn file() -> (std::path::PathBuf, File) {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "guardian-download-test-{}-{}.bin",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        (path, file)
    }

    fn bytes_release(bytes: &[u8]) -> Release {
        let mut release = parse("0.6.7", &metadata()).unwrap().unwrap();
        release.size = bytes.len() as u64;
        release.digest = Sha256::digest(bytes).into();
        release.official = Some(VerifiedInstaller {
            size: release.size,
            digest: release.digest,
            version: release.version.clone(),
            source: "GitHub".into(),
        });
        for candidate in &mut release.candidates {
            candidate.size = release.size;
            candidate.digest = release.digest;
        }
        release
    }

    #[test]
    fn checks_both_trusted_sources_and_uses_service_when_github_is_unreachable() {
        let seen = std::sync::Mutex::new(Vec::new());
        let transport = FakeTransport(|url: &Url, _: Option<(u64, u64)>| {
            seen.lock().unwrap().push(url.host.clone());
            if url.host == SERVICE {
                assert!(url.path.contains("version_code=6007"));
                let data = serde_json::to_vec(&service_metadata()).unwrap();
                Ok(response(&data, None, data.len() as u64))
            } else {
                Err(io::ErrorKind::TimedOut.into())
            }
        });
        let checked = check_details_with("0.6.7", &transport).unwrap();
        assert_eq!(checked.release.unwrap().version, "0.10.0");
        let seen = seen.into_inner().unwrap();
        assert!(seen.contains(&SERVICE.into()) && seen.contains(&GITHUB_API.into()));
    }

    #[test]
    fn current_service_does_not_hide_a_newer_github_release() {
        let transport = FakeTransport(|url: &Url, _: Option<(u64, u64)>| {
            let data = if url.host == SERVICE {
                serde_json::to_vec(&json!({"has_update": false,"channel":"stable","version_name":"0.6.7","version_code":6007})).unwrap()
            } else {
                serde_json::to_vec(&metadata()).unwrap()
            };
            Ok(response(&data, None, data.len() as u64))
        });
        assert_eq!(
            check_details_with("0.6.7", &transport)
                .unwrap()
                .release
                .unwrap()
                .version,
            "0.10.0"
        );
    }

    #[test]
    fn trusted_sources_choose_highest_numeric_version_and_cross_check_only_same_version() {
        let mut service = service_metadata();
        service["version_name"] = json!("0.11.0");
        service["version_code"] = json!(11000);
        let checked = select_observations(
            parse_service("0.6.7", &serde_json::to_vec(&service).unwrap()),
            parse_github("0.6.7", &serde_json::to_vec(&metadata()).unwrap()),
        )
        .unwrap();
        let release = checked.release.unwrap();
        assert_eq!(release.version, "0.11.0");
        assert_eq!(release.candidates.len(), 1);
        assert!(release.official.is_none());
        let checked = select_observations(
            parse_service("0.6.7", &serde_json::to_vec(&service_metadata()).unwrap()),
            parse_github("0.6.7", &serde_json::to_vec(&metadata()).unwrap()),
        )
        .unwrap();
        let release = checked.release.unwrap();
        assert_eq!(release.candidates.len(), 3);
        assert!(release.official.is_some());
    }

    #[test]
    fn all_metadata_errors_fail_but_one_valid_current_response_succeeds() {
        let failed = || Err(invalid("simulated invalid response"));
        assert!(select_observations(failed(), failed()).is_err());
        let current = parse_github("0.10.0", &serde_json::to_vec(&metadata()).unwrap());
        assert!(select_observations(failed(), current)
            .unwrap()
            .release
            .is_none());
    }

    #[test]
    fn rejects_unsafe_redirect_urls_without_exposing_signed_query() {
        for url in [
            "http://r2.lnyun.site/a",
            "https://user:secret@r2.lnyun.site/a",
            "https://r2.lnyun.site:443/a",
            "https://r2.lnyun.site\\@evil.test/a",
            "https://r2.lnyun.site/a#secret",
            "https://r2.lnyun.site/a\r\nCookie:secret",
        ] {
            assert!(parse_url(url).is_err(), "{url}");
        }
        let url = parse_url("https://r2.lnyun.site/a?signature=example%2Fvalue").unwrap();
        assert_eq!(url.host, "r2.lnyun.site");
        assert_eq!(url.path, "/a?signature=example%2Fvalue");
    }

    #[test]
    fn corrupt_source_clears_stage_before_trying_another_source() {
        let release = bytes_release(b"abc");
        let transport = FakeTransport(|url: &Url, range: Option<(u64, u64)>| {
            Ok(response(
                if url.host == MIRROR { b"abd" } else { b"abc" },
                range,
                3,
            ))
        });
        let (path, mut output) = file();
        output
            .write_all(b"old failed bytes must be removed")
            .unwrap();
        let verified = download_with(&release, &mut output, &transport).unwrap();
        output.rewind().unwrap();
        let mut data = Vec::new();
        output.read_to_end(&mut data).unwrap();
        assert_eq!(data, b"abc");
        assert_eq!(verified.size, 3);
        drop(output);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn parallel_ranges_assemble_in_order_and_reject_malformed_ranges_before_single_retry() {
        let data = (0..(3 * 1024 * 1024))
            .map(|i| (i % 251) as u8)
            .collect::<Vec<_>>();
        let release = bytes_release(&data);
        let ranges = std::sync::Mutex::new(Vec::new());
        let transport = FakeTransport(|_: &Url, range: Option<(u64, u64)>| {
            ranges.lock().unwrap().push(range);
            if range.is_some_and(|(start, _)| start == 0) {
                std::thread::sleep(Duration::from_millis(10));
            }
            Ok(response(&data, range, data.len() as u64))
        });
        let (path, mut output) = file();
        download_with(&release, &mut output, &transport).unwrap();
        output.rewind().unwrap();
        let mut actual = Vec::new();
        output.read_to_end(&mut actual).unwrap();
        assert_eq!(actual, data);
        assert!(
            ranges
                .lock()
                .unwrap()
                .iter()
                .flatten()
                .filter(|(start, _)| *start > 0)
                .count()
                >= 1
        );
        drop(output);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn range_unsupported_or_wrong_content_range_falls_back_to_verified_single_stream() {
        let data = vec![0x51; 2 * 1024 * 1024];
        let release = bytes_release(&data);
        for unsupported in [true, false] {
            let single = std::sync::atomic::AtomicUsize::new(0);
            let transport = FakeTransport(|_: &Url, range: Option<(u64, u64)>| {
                if range.is_none() {
                    single.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
                if unsupported {
                    return Ok(response(&data, None, data.len() as u64));
                }
                let mut result = response(&data, range, data.len() as u64);
                if range.is_some_and(|(_, end)| end >= PROBE_BYTES) {
                    result.content_range = Some("bytes 0-10/11".into());
                }
                Ok(result)
            });
            let (path, mut output) = file();
            download_with(&release, &mut output, &transport).unwrap();
            assert!(single.load(std::sync::atomic::Ordering::Relaxed) >= 1);
            output.rewind().unwrap();
            let mut actual = Vec::new();
            output.read_to_end(&mut actual).unwrap();
            assert_eq!(actual, data);
            drop(output);
            std::fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn exhausted_range_deadline_leaves_time_for_single_stream_fallback() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        struct RangeTimeout<'a> {
            bytes: &'a [u8],
            range_requests: AtomicUsize,
            single_requests: AtomicUsize,
        }

        impl Transport for RangeTimeout<'_> {
            fn get(
                &self,
                _: &Url,
                range: Option<(u64, u64)>,
                timeout: Duration,
            ) -> io::Result<http::Response> {
                if range.is_some_and(|(_, end)| end >= PROBE_BYTES) {
                    self.range_requests.fetch_add(1, Ordering::Relaxed);
                    std::thread::sleep(timeout);
                    return Err(io::ErrorKind::TimedOut.into());
                }
                if range.is_none() {
                    self.single_requests.fetch_add(1, Ordering::Relaxed);
                }
                Ok(response(self.bytes, range, self.bytes.len() as u64))
            }
        }

        let bytes = vec![0x51; RANGE_THRESHOLD as usize];
        let mut release = bytes_release(&bytes);
        release.candidates.truncate(1);
        let transport = RangeTimeout {
            bytes: &bytes,
            range_requests: AtomicUsize::new(0),
            single_requests: AtomicUsize::new(0),
        };
        let (path, mut output) = file();
        let result = download_with_timeout(
            &release,
            &mut output,
            &transport,
            Duration::from_millis(300),
        );
        output.rewind().unwrap();
        let mut actual = Vec::new();
        output.read_to_end(&mut actual).unwrap();
        drop(output);
        std::fs::remove_file(path).unwrap();

        assert!(transport.range_requests.load(Ordering::Relaxed) > 0);
        assert_eq!(transport.single_requests.load(Ordering::Relaxed), 1);
        assert!(result.unwrap().source().ends_with("单线程"));
        assert_eq!(actual, bytes);
    }

    fn pe() -> Vec<u8> {
        let mut bytes = vec![0; 256];
        bytes[..2].copy_from_slice(b"MZ");
        bytes[0x3c..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        bytes[0x80..0x84].copy_from_slice(b"PE\0\0");
        bytes[0x84..0x86].copy_from_slice(&0x8664u16.to_le_bytes());
        bytes[0x94..0x96].copy_from_slice(&2u16.to_le_bytes());
        bytes[0x98..0x9a].copy_from_slice(&0x020bu16.to_le_bytes());
        bytes
    }

    fn installer_zip(name: &str, bytes: &[u8]) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        writer
            .start_file(
                name,
                zip::write::FileOptions::<()>::default()
                    .compression_method(zip::CompressionMethod::Stored),
            )
            .unwrap();
        writer.write_all(bytes).unwrap();
        writer.finish().unwrap().into_inner()
    }

    fn service_zip_release(bytes: &[u8]) -> Release {
        let mut metadata = service_metadata();
        metadata["file_size"] = json!(bytes.len());
        metadata["sha256"] = json!(format!("{:x}", Sha256::digest(bytes)));
        parse("0.6.7", &metadata).unwrap().unwrap()
    }

    #[test]
    fn service_zip_is_verified_then_only_exact_installer_is_written_and_reverified() {
        let inner = pe();
        let zip = installer_zip(&installer_name("0.10.0"), &inner);
        let release = service_zip_release(&zip);
        let transport = FakeTransport(|_: &Url, range: Option<(u64, u64)>| {
            Ok(response(&zip, range, zip.len() as u64))
        });
        let (path, mut output) = file();
        let verified = download_with(&release, &mut output, &transport).unwrap();
        assert_eq!(verified.size, inner.len() as u64);
        assert_eq!(verified.version(), "0.10.0");
        output.rewind().unwrap();
        verify(&verified, &mut output).unwrap();
        output.rewind().unwrap();
        let mut actual = Vec::new();
        output.read_to_end(&mut actual).unwrap();
        assert_eq!(actual, inner);
        assert!(!actual.starts_with(b"PK"));
        output.rewind().unwrap();
        output.write_all(b"tamper").unwrap();
        output.rewind().unwrap();
        assert!(verify(&verified, &mut output).is_err());
        drop(output);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn service_zip_rejects_portable_archive_and_conflicting_official_inner_digest() {
        let inner = pe();
        for name in ["campus-network-guardian.exe", &installer_name("0.10.0")] {
            let zip = installer_zip(name, &inner);
            let mut release = service_zip_release(&zip);
            if name.ends_with("-setup.exe") {
                release.official = Some(VerifiedInstaller {
                    size: inner.len() as u64,
                    digest: [0; 32],
                    version: "0.10.0".into(),
                    source: "GitHub".into(),
                });
            }
            let transport = FakeTransport(|_: &Url, range: Option<(u64, u64)>| {
                Ok(response(&zip, range, zip.len() as u64))
            });
            let (path, mut output) = file();
            assert!(download_with(&release, &mut output, &transport).is_err());
            assert_eq!(output.metadata().unwrap().len(), 0);
            drop(output);
            std::fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn adaptive_probe_uses_fastest_available_node_and_survives_probe_failure() {
        for failed_probe in [false, true] {
            let release = bytes_release(b"abc");
            let transport = FakeTransport(|url: &Url, range: Option<(u64, u64)>| {
                if url.host == MIRROR {
                    if failed_probe && range.is_some() {
                        return Err(io::ErrorKind::TimedOut.into());
                    }
                    std::thread::sleep(Duration::from_millis(25));
                }
                Ok(response(b"abc", range, 3))
            });
            let (path, mut output) = file();
            let verified = download_with(&release, &mut output, &transport).unwrap();
            assert!(verified.source().starts_with("GitHub ·"));
            drop(output);
            std::fs::remove_file(path).unwrap();
        }
    }

    fn redirected(location: &str) -> http::Response {
        http::Response {
            status: 302,
            location: Some(location.into()),
            content_range: None,
            content_length: Some(0),
            content_encoding: None,
            reader: Box::new(Cursor::new(Vec::new())),
        }
    }

    #[test]
    fn follows_only_explicit_cdn_hosts_preserving_ephemeral_signature_and_redirect_limit() {
        let transport = FakeTransport(|url: &Url, _: Option<(u64, u64)>| {
            if url.host == SERVICE {
                Ok(redirected(
                    "https://r2.lnyun.site/file?signature=example%2Fvalue",
                ))
            } else {
                assert_eq!(url.path, "/file?signature=example%2Fvalue");
                Ok(response(b"abc", None, 3))
            }
        });
        let start = Url {
            host: SERVICE.into(),
            path: "/api/v1/app/download/275".into(),
        };
        let (_, last) = fetch(
            &transport,
            &start,
            Family::Service,
            None,
            Instant::now() + PROBE_TIMEOUT,
        )
        .unwrap();
        assert_eq!(last.host, "r2.lnyun.site");
        for bad in [
            "https://evil.test/file?signature=hidden",
            "http://r2.lnyun.site/file",
            "https://user:secret@r2.lnyun.site/file",
        ] {
            let transport = FakeTransport(|_: &Url, _: Option<(u64, u64)>| Ok(redirected(bad)));
            let error = fetch(
                &transport,
                &start,
                Family::Service,
                None,
                Instant::now() + PROBE_TIMEOUT,
            )
            .err()
            .unwrap();
            assert!(!error.to_string().contains("hidden") && !error.to_string().contains("secret"));
        }
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let looped = FakeTransport(|_: &Url, _: Option<(u64, u64)>| {
            calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Ok(redirected("https://r2.lnyun.site/loop"))
        });
        assert!(fetch(
            &looped,
            &start,
            Family::Service,
            None,
            Instant::now() + PROBE_TIMEOUT
        )
        .is_err());
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::Relaxed),
            MAX_REDIRECTS + 1
        );
        assert!(allowed_redirect(
            Family::Github,
            &parse_url("https://release-assets.githubusercontent.com/file").unwrap()
        ));
        assert!(allowed_redirect(
            Family::Github,
            &parse_url("https://objects.githubusercontent.com/file").unwrap()
        ));
        assert!(!allowed_redirect(
            Family::Github,
            &parse_url("https://r2.lnyun.site/file").unwrap()
        ));
    }

    #[test]
    fn corrupted_or_truncated_ranges_never_leave_an_accepted_or_partial_installer() {
        let data = vec![0x31; 2 * 1024 * 1024];
        let release = bytes_release(&data);
        for truncated in [false, true] {
            let transport = FakeTransport(|_: &Url, range: Option<(u64, u64)>| {
                let mut result = response(&data, range, data.len() as u64);
                if range.is_none_or(|(_, end)| end >= PROBE_BYTES) {
                    let mut bad = if let Some((start, end)) = range {
                        data[start as usize..=end as usize].to_vec()
                    } else {
                        data.clone()
                    };
                    if truncated {
                        bad.pop();
                    } else {
                        bad[0] ^= 1;
                    }
                    result.reader = Box::new(Cursor::new(bad));
                }
                Ok(result)
            });
            let (path, mut output) = file();
            assert!(download_with(&release, &mut output, &transport).is_err());
            assert_eq!(output.metadata().unwrap().len(), 0);
            drop(output);
            std::fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn encoded_or_oversized_metadata_is_rejected_before_json_processing() {
        for encoding in [None, Some("gzip")] {
            let transport = FakeTransport(|_: &Url, _: Option<(u64, u64)>| {
                let mut result = response(b"{}", None, 2);
                result.content_encoding = encoding.map(str::to_owned);
                if encoding.is_none() {
                    result.content_length = Some(MAX_METADATA_BYTES + 1);
                }
                Ok(result)
            });
            assert!(check_details_with("0.6.7", &transport).is_err());
        }
    }

    #[test]
    fn older_changelog_never_overrides_newer_trusted_version_details() {
        let mut service = service_metadata();
        service["has_update"] = json!(false);
        service["version_name"] = json!("0.11.0");
        service["version_code"] = json!(11000);
        let mut github = metadata();
        github["body"] = json!("Older release notes");
        let checked = select_observations(
            parse_service("0.12.0", &serde_json::to_vec(&service).unwrap()),
            parse_github("0.12.0", &serde_json::to_vec(&github).unwrap()),
        )
        .unwrap();
        assert_eq!(checked.details.unwrap().version, "0.11.0");
    }

    #[test]
    fn expired_probe_signature_is_refreshed_through_original_download_api() {
        let inner = pe();
        let zip = installer_zip(&installer_name("0.10.0"), &inner);
        let release = service_zip_release(&zip);
        let transport = FakeTransport(|url: &Url, range: Option<(u64, u64)>| {
            if url.host == SERVICE {
                Ok(redirected(if range.is_some() {
                    "https://r2.lnyun.site/file?signature=probe"
                } else {
                    "https://r2.lnyun.site/file?signature=fresh"
                }))
            } else if range.is_none() && url.path.contains("signature=probe") {
                Err(io::ErrorKind::PermissionDenied.into())
            } else {
                Ok(response(&zip, range, zip.len() as u64))
            }
        });
        let (path, mut output) = file();
        let result = download_with(&release, &mut output, &transport);
        drop(output);
        std::fs::remove_file(path).unwrap();
        assert!(
            result.is_ok(),
            "single-stream retry should obtain a fresh signed CDN URL"
        );
    }

    #[test]
    fn reports_cdn_used_by_successful_download_even_when_probe_was_direct() {
        for (size, mode) in [(256, "单线程"), (RANGE_THRESHOLD as usize, "2 线程")] {
            let mut inner = pe();
            inner.resize(size, 0);
            let zip = installer_zip(&installer_name("0.10.0"), &inner);
            let release = service_zip_release(&zip);
            let transport = FakeTransport(|url: &Url, range: Option<(u64, u64)>| {
                if url.host == SERVICE && range.is_none_or(|(_, end)| end >= PROBE_BYTES) {
                    return Ok(redirected("https://r2.lnyun.site/file?signature=download"));
                }
                Ok(response(&zip, range, zip.len() as u64))
            });
            let (path, mut output) = file();
            let result = download_with(&release, &mut output, &transport);
            drop(output);
            std::fs::remove_file(path).unwrap();
            assert_eq!(
                result.unwrap().source(),
                format!("流年 App · CF CDN · {mode}")
            );
        }
    }

    #[test]
    fn probe_samples_a_bounded_64_kib_instead_of_only_a_small_ttfb_prefix() {
        let data = vec![0; 1024 * 1024];
        let release = bytes_release(&data);
        let requested = std::sync::Mutex::new(Vec::new());
        let transport = FakeTransport(|_: &Url, range: Option<(u64, u64)>| {
            requested.lock().unwrap().push(range.unwrap());
            Ok(response(&data, range, data.len() as u64))
        });
        ranked_candidates(&release, &transport);
        let requested = requested.into_inner().unwrap();
        assert_eq!(requested.len(), 2);
        assert!(requested
            .iter()
            .all(|&(start, end)| start == 0 && end == 65535));
    }

    #[test]
    fn probe_estimate_accounts_for_different_zip_and_exe_payload_sizes() {
        let exe = vec![0; 1024 * 1024];
        let zip = vec![0; 2 * 1024 * 1024];
        let mut release = bytes_release(&exe);
        release.candidates[0].family = Family::Service;
        release.candidates[0].url.host = SERVICE.into();
        release.candidates[0].size = zip.len() as u64;
        release.candidates[0].payload = Payload::Zip;
        let transport = FakeTransport(|url: &Url, range: Option<(u64, u64)>| {
            let (data, delay) = if url.host == SERVICE {
                (&zip, 20)
            } else {
                (&exe, 30)
            };
            std::thread::sleep(Duration::from_millis(delay));
            Ok(response(data, range, data.len() as u64))
        });
        let ranked = ranked_candidates(&release, &transport);
        assert_eq!(ranked[0].0.family, Family::Github);
    }

    #[test]
    fn adaptive_thread_boundaries_cover_the_current_two_megabyte_installer() {
        for (size, threads) in [
            (1024 * 1024 - 1, 1),
            (1024 * 1024, 2),
            (2_002_486, 2),
            (4 * 1024 * 1024, 4),
        ] {
            let data = vec![0x37; size];
            let release = bytes_release(&data);
            let transport = FakeTransport(|_: &Url, range: Option<(u64, u64)>| {
                Ok(response(&data, range, data.len() as u64))
            });
            let (path, mut output) = file();
            let verified = download_with(&release, &mut output, &transport).unwrap();
            drop(output);
            std::fs::remove_file(path).unwrap();
            let expected = if threads == 1 {
                "单线程".into()
            } else {
                format!("{threads} 线程")
            };
            assert!(
                verified.source().ends_with(&expected),
                "size={size}: {}",
                verified.source()
            );
        }
    }
}
