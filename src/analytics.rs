use serde_json::json;
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read},
    os::windows::fs::OpenOptionsExt,
    path::PathBuf,
    sync::mpsc,
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use windows_sys::Win32::{
    Security::Cryptography::{BCryptGenRandom, BCRYPT_USE_SYSTEM_PREFERRED_RNG},
    System::Registry::{
        RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_BINARY, RRF_RT_REG_BINARY,
    },
};

const HOST: &str = "app.liunianai.cn";
const ENDPOINT: &str = "/api/v1/app/analytics/events";
const PACKAGE: &str = "cn.liunianai.campusnetworkguardian.desktop";
const SETTINGS_KEY: &str = r"Software\CampusNetworkGuardian";
const SETTINGS_VALUE: &str = "AnalyticsConsent";
const RECORD_VERSION: u8 = 1;
const RECORD_SIZE: usize = 25;
const BEIJING_OFFSET_MS: u64 = 8 * 60 * 60 * 1000;
const DAY_MS: u64 = 24 * 60 * 60 * 1000;
const RECHECK: Duration = Duration::from_secs(30 * 60);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Consent {
    id: Option<[u8; 16]>,
    last_day: u64,
}

fn encode(state: Consent) -> [u8; RECORD_SIZE] {
    let mut record = [0; RECORD_SIZE];
    record[0] = RECORD_VERSION;
    record[1..17].copy_from_slice(&state.id.unwrap_or_default());
    record[17..].copy_from_slice(&state.last_day.to_le_bytes());
    record
}

fn decode(record: &[u8]) -> Option<Consent> {
    if record.len() != RECORD_SIZE || record[0] != RECORD_VERSION {
        return None;
    }
    let id: [u8; 16] = record[1..17].try_into().ok()?;
    Some(Consent {
        id: (id != [0; 16]).then_some(id),
        last_day: u64::from_le_bytes(record[17..].try_into().ok()?),
    })
}

fn registry_names() -> (Vec<u16>, Vec<u16>) {
    (
        SETTINGS_KEY.encode_utf16().chain(Some(0)).collect(),
        SETTINGS_VALUE.encode_utf16().chain(Some(0)).collect(),
    )
}

fn load() -> Consent {
    let (key, name) = registry_names();
    let mut record = [0u8; RECORD_SIZE];
    let mut size = RECORD_SIZE as u32;
    // The response must be the exact versioned consent record or statistics stay disabled.
    let result = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_BINARY,
            std::ptr::null_mut(),
            record.as_mut_ptr().cast(),
            &mut size,
        )
    };
    if result == 0 && size as usize == RECORD_SIZE {
        decode(&record).unwrap_or_default()
    } else {
        Consent::default()
    }
}

fn persist(state: Consent) -> io::Result<()> {
    let (key, name) = registry_names();
    let record = encode(state);
    let result = unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            REG_BINARY,
            record.as_ptr().cast(),
            record.len() as u32,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(result as i32))
    }
}

// The app's main mutex is session-local. This per-user file serializes consent
// writes and daily reservations even when that user has another Windows session.
fn consent_lock() -> io::Result<File> {
    let directory = PathBuf::from(std::env::var_os("LOCALAPPDATA").ok_or(io::ErrorKind::NotFound)?)
        .join("CampusNetworkGuardian");
    fs::create_dir_all(&directory)?;
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .share_mode(0)
        .open(directory.join("analytics.lock"))
}

fn random_id() -> io::Result<[u8; 16]> {
    let mut id = [0; 16];
    let status = unsafe {
        BCryptGenRandom(
            std::ptr::null_mut(),
            id.as_mut_ptr(),
            id.len() as u32,
            BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        )
    };
    if status != 0 {
        return Err(io::Error::other("Windows RNG unavailable"));
    }
    id[6] = (id[6] & 0x0f) | 0x40;
    id[8] = (id[8] & 0x3f) | 0x80;
    Ok(id)
}

fn uuid(id: [u8; 16]) -> String {
    let mut output = String::with_capacity(36);
    for (index, byte) in id.iter().enumerate() {
        if matches!(index, 4 | 6 | 8 | 10) {
            output.push('-');
        }
        use std::fmt::Write;
        let _ = write!(output, "{byte:02x}");
    }
    output
}

fn event_body(installation: [u8; 16], session: [u8; 16], timestamp_ms: u64) -> Vec<u8> {
    json!({
        "events": [{
            "name": "app_open",
            "device_id": uuid(installation),
            "session_id": uuid(session),
            "ts": timestamp_ms,
            "props": { "platform": "windows", "channel": "stable" }
        }]
    })
    .to_string()
    .into_bytes()
}

fn event_headers() -> String {
    let major = env!("CARGO_PKG_VERSION_MAJOR").parse::<u64>().unwrap();
    let minor = env!("CARGO_PKG_VERSION_MINOR").parse::<u64>().unwrap();
    let patch = env!("CARGO_PKG_VERSION_PATCH").parse::<u64>().unwrap();
    let code = major * 1_000_000 + minor * 1000 + patch;
    let version = env!("CARGO_PKG_VERSION");
    format!(
        "Content-Type: application/json\r\nAccept: application/json\r\nX-App-Package: {PACKAGE}\r\nX-App-Version-Code: {code}\r\nX-App-Version-Name: {version}\r\nX-App-Channel: stable\r\nX-App-Platform: windows\r\n"
    )
}

fn reserve_day(
    state: Consent,
    day: u64,
    write: impl FnOnce(Consent) -> io::Result<()>,
) -> io::Result<Option<[u8; 16]>> {
    let Some(id) = state.id else {
        return Ok(None);
    };
    if state.last_day >= day {
        return Ok(None);
    }
    // The service does not document an idempotency key. Reserve before sending,
    // so a network failure or an app restart doesn't repeat the day's event.
    write(Consent {
        last_day: day,
        ..state
    })?;
    Ok(Some(id))
}

fn post_event<SendGuard>(
    body: &[u8],
    before_send: impl FnOnce() -> io::Result<SendGuard>,
) -> io::Result<()> {
    let response = crate::winhttp::post(
        HOST,
        ENDPOINT,
        &event_headers(),
        body,
        Duration::from_secs(12),
        before_send,
    )?;
    if !(200..300).contains(&response.status) {
        return Err(io::Error::other(format!(
            "analytics request returned HTTP {}",
            response.status
        )));
    }
    let mut reply = Vec::new();
    response.reader.take(8193).read_to_end(&mut reply)?;
    if reply.len() > 8192 {
        return Err(io::Error::other("analytics response is too large"));
    }
    let reply: serde_json::Value = serde_json::from_slice(&reply)?;
    if reply.get("ok").and_then(serde_json::Value::as_bool) == Some(true) {
        Ok(())
    } else {
        Err(io::Error::other(
            "analytics service did not accept the event",
        ))
    }
}

#[derive(Clone)]
pub struct Handle {
    wake: mpsc::Sender<()>,
}

impl Handle {
    pub fn enabled(&self) -> bool {
        load().id.is_some()
    }

    pub fn set_enabled(&self, enabled: bool) -> Result<(), String> {
        let _lock = consent_lock().map_err(|_| "使用统计设置正在使用，请重试".to_owned())?;
        let mut state = load();
        if enabled == state.id.is_some() {
            return Ok(());
        }
        state.id = if enabled {
            Some(random_id().map_err(|_| "无法生成匿名安装标识".to_owned())?)
        } else {
            None
        };
        persist(state).map_err(|_| "无法保存使用统计设置".to_owned())?;
        let _ = self.wake.send(());
        Ok(())
    }
}

fn report_today() -> io::Result<()> {
    let state = load();
    let Some(id) = state.id else {
        return Ok(());
    };
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(io::Error::other)?;
    let timestamp = u64::try_from(elapsed.as_millis()).map_err(io::Error::other)?;
    let day = timestamp
        .checked_add(BEIJING_OFFSET_MS)
        .ok_or(io::ErrorKind::InvalidData)?
        / DAY_MS;
    if state.last_day >= day {
        return Ok(());
    }
    let body = event_body(id, random_id()?, timestamp);
    post_event(&body, || {
        // Check and reserve at the send boundary, then hold the lock only
        // until WinHTTP consumes the body. Revocation and sending serialize.
        let lock = consent_lock()?;
        let current = load();
        if current.id != Some(id) || reserve_day(current, day, persist)?.is_none() {
            return Err(io::ErrorKind::Interrupted.into());
        }
        Ok(lock)
    })
}

pub fn start() -> Handle {
    let (wake, receive) = mpsc::channel();
    let _ = thread::Builder::new()
        .name("anonymous-usage".into())
        .spawn(move || {
            // Give the guardian time to restore network access after Windows login.
            let mut delay = Duration::from_secs(20);
            while !matches!(
                receive.recv_timeout(delay),
                Err(mpsc::RecvTimeoutError::Disconnected)
            ) {
                let _ = report_today();
                delay = RECHECK;
            }
        });
    Handle { wake }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const ID: [u8; 16] = [1; 16];
    const DAY: u64 = 20_000;

    #[test]
    fn disabled_state_never_reserves_an_event() {
        assert!(
            reserve_day(Consent::default(), DAY, |_| panic!("must not persist"))
                .unwrap()
                .is_none()
        );
        assert!(decode(&[]).is_none());
        assert!(decode(&[0; 25]).is_none());
    }

    #[test]
    fn reserve_persists_before_sending_and_suppresses_restart_or_clock_rollback() {
        let state = Consent {
            id: Some(ID),
            last_day: 0,
        };
        let mut saved = None;
        assert_eq!(
            reserve_day(state, DAY, |value| {
                saved = Some(encode(value));
                Ok(())
            })
            .unwrap(),
            Some(ID)
        );
        let restored = decode(&saved.unwrap()).unwrap();
        for day in [DAY, DAY - 1] {
            assert_eq!(
                reserve_day(restored, day, |_| unreachable!()).unwrap(),
                None
            );
        }
        assert_eq!(
            reserve_day(restored, DAY + 1, |_| Ok(())).unwrap(),
            Some(ID)
        );
    }

    #[test]
    fn turning_statistics_off_erases_the_id_but_keeps_the_daily_cap() {
        let mut state = Consent {
            id: Some(ID),
            last_day: DAY,
        };
        state.id = None;
        let stored = encode(state);
        assert!(stored[1..17].iter().all(|byte| *byte == 0));
        let mut restored = decode(&stored).unwrap();
        assert_eq!(restored.id, None);
        restored.id = Some([2; 16]);
        assert_eq!(
            reserve_day(restored, DAY, |_| panic!("cannot report twice")).unwrap(),
            None
        );
    }

    #[test]
    fn failure_to_persist_day_does_not_grant_a_request() {
        let state = Consent {
            id: Some(ID),
            last_day: 0,
        };
        assert!(reserve_day(state, DAY, |_| Err(io::ErrorKind::PermissionDenied.into())).is_err());
    }

    #[test]
    fn public_request_has_only_the_minimal_anonymous_fields() {
        let data = event_body(ID, [2; 16], 1_764_144_000_000);
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&data).unwrap(),
            json!({
                "events": [{
                    "name": "app_open",
                    "device_id": "01010101-0101-0101-0101-010101010101",
                    "session_id": "02020202-0202-0202-0202-020202020202",
                    "ts": 1_764_144_000_000u64,
                    "props": { "platform": "windows", "channel": "stable" }
                }]
            })
        );
        let headers = event_headers();
        assert!(headers.contains("X-App-Package: cn.liunianai.campusnetworkguardian.desktop\r\n"));
        assert!(headers.contains("X-App-Platform: windows\r\n"));
        assert!(!headers.contains("Authorization:"));
        assert!(!headers.contains("Cookie:"));
    }
}
