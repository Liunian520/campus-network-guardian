mod archive;
mod download;
mod handoff;
mod stage;

use std::{
    ffi::OsString,
    fs, io,
    os::windows::{ffi::OsStringExt, process::CommandExt},
    path::{Path, PathBuf},
    process::Command,
    sync::{mpsc, Arc, Mutex},
    thread,
    time::Duration,
};
use windows_sys::Win32::{
    Foundation::{ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND},
    System::{
        Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_SZ},
        Threading::CREATE_NO_WINDOW,
    },
    UI::Shell::IsUserAnAdmin,
};

const EXECUTABLE: &str = "campus-network-guardian.exe";
const INSTALLED_KEY: &str =
    r"Software\Microsoft\Windows\CurrentVersion\Uninstall\CampusNetworkGuardian.Windows_is1";
const CHECK_PERIOD: Duration = Duration::from_secs(6 * 60 * 60);
const RETRY_PERIOD: Duration = Duration::from_secs(30 * 60);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpdateStatus {
    Idle,
    Checking,
    Current,
    Installing,
    Failed,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UpdateInfo {
    pub version: String,
    pub source: String,
    pub changelog: String,
    pub progress: String,
}

enum CheckEvent {
    Announcement(UpdateInfo),
    SelectedSource { version: String, source: String },
    Progress(&'static str),
    Installing,
}

#[derive(Clone)]
pub struct UpdateHandle {
    request: mpsc::Sender<()>,
    status: Arc<Mutex<UpdateStatus>>,
    info: Arc<Mutex<UpdateInfo>>,
}

impl UpdateHandle {
    pub fn request_check(&self) -> bool {
        let mut status = self.status.lock().unwrap_or_else(|err| err.into_inner());
        if matches!(*status, UpdateStatus::Checking | UpdateStatus::Installing) {
            return false;
        }
        if self.request.send(()).is_err() {
            *status = UpdateStatus::Failed;
            self.info
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .progress = "更新检查暂不可用".into();
            return false;
        }
        *status = UpdateStatus::Checking;
        self.info
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .progress = "正在检查更新".into();
        true
    }

    pub fn status(&self) -> UpdateStatus {
        *self.status.lock().unwrap_or_else(|err| err.into_inner())
    }

    pub fn info(&self) -> UpdateInfo {
        self.info
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone()
    }
}

pub fn start() -> UpdateHandle {
    let delay = if std::env::args_os().any(|arg| arg == "--from-update") {
        CHECK_PERIOD
    } else {
        Duration::from_secs(20)
    };
    spawn_worker(delay, |publish| {
        if cfg!(test) {
            return Err(io::ErrorKind::Unsupported.into());
        }
        check_once(publish)
    })
}

fn spawn_worker(
    mut delay: Duration,
    mut check: impl FnMut(&mut dyn FnMut(CheckEvent)) -> io::Result<()> + Send + 'static,
) -> UpdateHandle {
    let (request, wake) = mpsc::channel();
    let status = Arc::new(Mutex::new(UpdateStatus::Idle));
    let info = Arc::new(Mutex::new(UpdateInfo::default()));
    let worker_status = Arc::clone(&status);
    let worker_info = Arc::clone(&info);
    let spawned = thread::Builder::new()
        .name("release-updater".into())
        .spawn(move || {
            while let Ok(()) | Err(mpsc::RecvTimeoutError::Timeout) = wake.recv_timeout(delay) {
                {
                    let mut status = worker_status.lock().unwrap_or_else(|err| err.into_inner());
                    // A click racing the scheduled timeout belongs to this check.
                    // Drain it under the same lock used to publish a manual request.
                    let _ = wake.try_recv();
                    *status = UpdateStatus::Checking;
                    worker_info
                        .lock()
                        .unwrap_or_else(|err| err.into_inner())
                        .progress = "正在检查更新".into();
                }
                let success = check(&mut |event| match event {
                    CheckEvent::Announcement(details) => {
                        let mut info = worker_info.lock().unwrap_or_else(|err| err.into_inner());
                        if info.version != details.version || !details.changelog.trim().is_empty() {
                            info.changelog = details.changelog;
                        }
                        info.version = details.version;
                        info.source = details.source;
                    }
                    CheckEvent::SelectedSource { version, source } => {
                        let mut info = worker_info.lock().unwrap_or_else(|err| err.into_inner());
                        info.version = version;
                        info.source = source;
                    }
                    CheckEvent::Progress(progress) => {
                        worker_info
                            .lock()
                            .unwrap_or_else(|err| err.into_inner())
                            .progress = progress.into();
                    }
                    CheckEvent::Installing => {
                        let mut status =
                            worker_status.lock().unwrap_or_else(|err| err.into_inner());
                        *status = UpdateStatus::Installing;
                        worker_info
                            .lock()
                            .unwrap_or_else(|err| err.into_inner())
                            .progress = "正在启动更新安装".into();
                    }
                })
                .is_ok();
                {
                    let mut status = worker_status.lock().unwrap_or_else(|err| err.into_inner());
                    *status = if success {
                        UpdateStatus::Current
                    } else {
                        UpdateStatus::Failed
                    };
                    worker_info
                        .lock()
                        .unwrap_or_else(|err| err.into_inner())
                        .progress = if success {
                        "更新检查已完成"
                    } else {
                        "更新失败，可稍后重试"
                    }
                    .into();
                }
                delay = if success { CHECK_PERIOD } else { RETRY_PERIOD };
            }
        });
    if spawned.is_err() {
        *status.lock().unwrap_or_else(|err| err.into_inner()) = UpdateStatus::Failed;
        info.lock().unwrap_or_else(|err| err.into_inner()).progress = "更新检查暂不可用".into();
    }
    UpdateHandle {
        request,
        status,
        info,
    }
}

fn check_once(publish: &mut dyn FnMut(CheckEvent)) -> io::Result<()> {
    let executable = std::env::current_exe()?;
    if !executable
        .file_name()
        .is_some_and(|name| name.eq_ignore_ascii_case(EXECUTABLE))
    {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "updates require the standard Guardian executable name",
        ));
    }
    let directory = executable.parent().ok_or(io::ErrorKind::InvalidInput)?;
    let elevated = unsafe { IsUserAnAdmin() } != 0;
    let stage_parent = stage::parent(elevated)?;
    stage::clean_old(&stage_parent);
    let checked = download::check_details(env!("CARGO_PKG_VERSION"))?;
    if let Some(details) = checked.details {
        publish(CheckEvent::Announcement(UpdateInfo {
            version: details.version,
            source: details.source,
            changelog: details.changelog,
            progress: String::new(),
        }));
    }
    let Some(release) = checked.release else {
        return Ok(());
    };
    let portable = !registered_installation(&executable)?;
    let stage = stage::Stage::create(&stage_parent, elevated)?;
    publish(CheckEvent::Progress("正在选择线路并下载"));
    let verified = {
        let mut output = stage.create_file()?;
        let verified = download::download(&release, &mut output)?;
        output.sync_all()?;
        verified
    };
    publish(CheckEvent::SelectedSource {
        version: verified.version().to_owned(),
        source: verified.source().to_owned(),
    });
    publish(CheckEvent::Progress("正在校验并准备更新"));
    let mut locked = stage.lock_file()?;
    download::verify(&verified, &mut locked)?;
    let args = installer_args(directory, portable);
    publish(CheckEvent::Installing);
    if elevated {
        let stage_name = stage
            .directory()
            .file_name()
            .ok_or(io::ErrorKind::InvalidInput)?;
        return handoff::launch_and_handoff(
            &stage.installer(),
            stage.directory(),
            &args,
            stage_name,
        );
    }
    let mut installer = Command::new(stage.installer())
        .args(args)
        .current_dir(stage.directory())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()?;
    // Hold the verified file open without sharing writes or deletes while the
    // installer starts. Inno Setup owns graceful shutdown and restart.
    let result = installer.wait()?;
    drop(locked);
    if result.success() {
        Ok(())
    } else {
        Err(io::Error::other("silent release update failed"))
    }
}

fn registered_installation(executable: &Path) -> io::Result<bool> {
    let key: Vec<_> = INSTALLED_KEY.encode_utf16().chain(Some(0)).collect();
    let name: Vec<_> = "InstallLocation".encode_utf16().chain(Some(0)).collect();
    let mut buffer = vec![0u16; 32768];
    let mut size = (buffer.len() * 2) as u32;
    let result = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            buffer.as_mut_ptr().cast(),
            &mut size,
        )
    };
    match result {
        0 => {}
        ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND => return Ok(false),
        code => return Err(io::Error::from_raw_os_error(code as i32)),
    }
    if size < 2 || size % 2 != 0 || size as usize > buffer.len() * 2 {
        return Err(io::ErrorKind::InvalidData.into());
    }
    let path = PathBuf::from(OsString::from_wide(&buffer[..size as usize / 2 - 1]));
    if !path.is_absolute() {
        return Err(io::ErrorKind::InvalidData.into());
    }
    match fs::canonicalize(path.join(EXECUTABLE)) {
        Ok(installed) => Ok(fs::canonicalize(executable)?
            .as_os_str()
            .eq_ignore_ascii_case(installed.as_os_str())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

fn installer_args(directory: &Path, portable: bool) -> Vec<OsString> {
    let mut arguments = [
        "/VERYSILENT",
        "/SUPPRESSMSGBOXES",
        "/NORESTART",
        "/SP-",
        "/NOCANCEL",
        "/NOCLOSEAPPLICATIONS",
        "/NORESTARTAPPLICATIONS",
        "/GUARDIANUPDATE",
    ]
    .into_iter()
    .map(OsString::from)
    .collect::<Vec<_>>();
    if portable {
        arguments.push("/GUARDIANPORTABLE".into());
    }
    let mut target = OsString::from("/DIR=");
    target.push(directory.as_os_str());
    arguments.push(target);
    arguments
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{sync::mpsc, time::Instant};

    fn wait_for_status(handle: &UpdateHandle, expected: UpdateStatus) {
        let deadline = Instant::now() + Duration::from_secs(3);
        while handle.status() != expected && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(handle.status(), expected);
    }

    #[test]
    fn manual_check_wakes_early_and_coalesces_clicks_through_installation() {
        let (entered, checking) = mpsc::channel();
        let (advance, proceed) = mpsc::channel();
        let handle = spawn_worker(CHECK_PERIOD, move |installing| {
            entered.send(()).unwrap();
            proceed.recv().unwrap();
            installing(CheckEvent::Installing);
            proceed.recv().unwrap();
            Ok(())
        });

        assert_eq!(handle.status(), UpdateStatus::Idle);
        assert!(handle.request_check());
        checking.recv_timeout(Duration::from_secs(3)).unwrap();
        assert_eq!(handle.status(), UpdateStatus::Checking);
        assert!(!handle.request_check());
        advance.send(()).unwrap();
        wait_for_status(&handle, UpdateStatus::Installing);
        assert!(!handle.request_check());
        advance.send(()).unwrap();
        wait_for_status(&handle, UpdateStatus::Current);
        assert!(checking.recv_timeout(Duration::from_millis(30)).is_err());
    }

    #[test]
    fn automatic_check_and_manual_retry_share_the_same_status() {
        let mut first = true;
        let handle = spawn_worker(Duration::ZERO, move |_| {
            if std::mem::take(&mut first) {
                Err(io::Error::other("simulated release check failed"))
            } else {
                Ok(())
            }
        });
        wait_for_status(&handle, UpdateStatus::Failed);
        assert!(handle.request_check());
        wait_for_status(&handle, UpdateStatus::Current);
    }

    fn announcement() -> UpdateInfo {
        UpdateInfo {
            version: "0.6.8".into(),
            source: "流年 App · CF CDN".into(),
            changelog: "修复网络恢复后的更新检查。".into(),
            progress: String::new(),
        }
    }

    fn assert_announcement(handle: &UpdateHandle) {
        let info = handle.info();
        let expected = announcement();
        assert_eq!(info.version, expected.version);
        assert_eq!(info.source, expected.source);
        assert_eq!(info.changelog, expected.changelog);
    }

    #[test]
    fn automatic_and_manual_checks_share_and_preserve_announcement_snapshot() {
        let (entered, checking) = mpsc::channel();
        let (advance, proceed) = mpsc::channel();
        let mut calls = 0;
        let handle = spawn_worker(Duration::ZERO, move |publish| {
            calls += 1;
            if calls == 1 {
                publish(CheckEvent::Announcement(announcement()));
            }
            entered.send(calls).unwrap();
            proceed.recv().unwrap();
            if calls == 2 {
                Err(io::Error::other("simulated metadata check failed"))
            } else {
                Ok(())
            }
        });
        let clone = handle.clone();
        assert_eq!(checking.recv_timeout(Duration::from_secs(3)).unwrap(), 1);
        assert_eq!(clone.status(), UpdateStatus::Checking);
        assert_announcement(&clone);
        advance.send(()).unwrap();
        wait_for_status(&handle, UpdateStatus::Current);
        assert_announcement(&handle);

        assert!(clone.request_check());
        assert_eq!(checking.recv_timeout(Duration::from_secs(3)).unwrap(), 2);
        assert_eq!(handle.status(), UpdateStatus::Checking);
        assert_announcement(&handle);
        advance.send(()).unwrap();
        wait_for_status(&clone, UpdateStatus::Failed);
        assert_announcement(&clone);

        assert!(handle.request_check());
        assert_eq!(checking.recv_timeout(Duration::from_secs(3)).unwrap(), 3);
        advance.send(()).unwrap();
        wait_for_status(&clone, UpdateStatus::Current);
        assert_announcement(&clone);
    }

    #[test]
    fn same_version_without_notes_preserves_cached_notes_but_new_version_clears_them() {
        let mut calls = 0;
        let handle = spawn_worker(Duration::ZERO, move |publish| {
            calls += 1;
            let mut details = announcement();
            if calls > 1 {
                details.changelog.clear();
            }
            if calls == 3 {
                details.version = "0.6.9".into();
            }
            publish(CheckEvent::Announcement(details));
            Ok(())
        });
        wait_for_status(&handle, UpdateStatus::Current);
        assert_announcement(&handle);
        assert!(handle.request_check());
        wait_for_status(&handle, UpdateStatus::Current);
        assert_announcement(&handle);
        assert!(handle.request_check());
        wait_for_status(&handle, UpdateStatus::Current);
        assert_eq!(handle.info().version, "0.6.9");
        assert!(handle.info().changelog.is_empty());
    }

    #[test]
    fn failed_installation_retains_announcement_and_never_reports_current() {
        let (entered, installing) = mpsc::channel();
        let (advance, proceed) = mpsc::channel();
        let handle = spawn_worker(CHECK_PERIOD, move |publish| {
            publish(CheckEvent::Announcement(announcement()));
            publish(CheckEvent::Progress("正在校验并准备更新"));
            publish(CheckEvent::Installing);
            entered.send(()).unwrap();
            proceed.recv().unwrap();
            Err(io::Error::other("simulated installer launch failure"))
        });
        assert!(handle.request_check());
        installing.recv_timeout(Duration::from_secs(3)).unwrap();
        assert_eq!(handle.status(), UpdateStatus::Installing);
        advance.send(()).unwrap();
        wait_for_status(&handle, UpdateStatus::Failed);
        assert_announcement(&handle);
        assert!(!handle.info().progress.contains('%'));
    }

    #[test]
    fn download_failure_does_not_start_installation_or_discard_release_notes() {
        let handle = spawn_worker(Duration::ZERO, move |publish| {
            publish(CheckEvent::Announcement(announcement()));
            publish(CheckEvent::Progress("正在选择线路并下载"));
            Err(io::Error::other("simulated download failure"))
        });
        wait_for_status(&handle, UpdateStatus::Failed);
        assert_announcement(&handle);
    }

    #[test]
    fn verified_download_publishes_actual_selected_source_and_logical_progress() {
        let (entered, downloading) = mpsc::channel();
        let (advance, proceed) = mpsc::channel();
        let handle = spawn_worker(Duration::ZERO, move |publish| {
            publish(CheckEvent::Announcement(announcement()));
            publish(CheckEvent::SelectedSource {
                version: "0.6.8".into(),
                source: "GitHub 镜像 · 单线程".into(),
            });
            publish(CheckEvent::Progress("正在校验并准备更新"));
            entered.send(()).unwrap();
            proceed.recv().unwrap();
            Ok(())
        });
        downloading.recv_timeout(Duration::from_secs(3)).unwrap();
        let info = handle.info();
        assert_eq!(info.version, "0.6.8");
        assert_eq!(info.source, "GitHub 镜像 · 单线程");
        assert_eq!(info.changelog, announcement().changelog);
        assert_eq!(info.progress, "正在校验并准备更新");
        advance.send(()).unwrap();
        wait_for_status(&handle, UpdateStatus::Current);
    }

    #[test]
    fn installs_only_over_the_current_directory_and_preserves_tasks() {
        let directory = Path::new(r"C:\Program Files\Campus Guardian");
        let args = installer_args(directory, false);
        assert!(args.iter().any(|arg| arg == "/GUARDIANUPDATE"));
        assert!(args.iter().any(|arg| arg == "/VERYSILENT"));
        assert!(args
            .iter()
            .any(|arg| arg == "/DIR=C:\\Program Files\\Campus Guardian"));
        assert!(!args.iter().any(|arg| arg == "/GUARDIANPORTABLE"));
        assert!(!args
            .iter()
            .any(|arg| arg.to_string_lossy().starts_with("/TASKS")));
    }

    #[test]
    fn a_portable_update_uses_explicit_portable_setup_mode() {
        let args = installer_args(Path::new(r"D:\校园网工具"), true);
        assert!(args.iter().any(|arg| arg == "/GUARDIANPORTABLE"));
    }
}
