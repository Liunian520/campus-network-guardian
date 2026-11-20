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

#[derive(Clone)]
pub struct UpdateHandle {
    request: mpsc::Sender<()>,
    status: Arc<Mutex<UpdateStatus>>,
}

impl UpdateHandle {
    pub fn request_check(&self) -> bool {
        let mut status = self.status.lock().unwrap_or_else(|err| err.into_inner());
        if matches!(*status, UpdateStatus::Checking | UpdateStatus::Installing) {
            return false;
        }
        if self.request.send(()).is_err() {
            *status = UpdateStatus::Failed;
            return false;
        }
        *status = UpdateStatus::Checking;
        true
    }

    pub fn status(&self) -> UpdateStatus {
        *self.status.lock().unwrap_or_else(|err| err.into_inner())
    }
}

pub fn start() -> UpdateHandle {
    let delay = if std::env::args_os().any(|arg| arg == "--from-update") {
        CHECK_PERIOD
    } else {
        Duration::from_secs(20)
    };
    spawn_worker(delay, |installing| {
        if cfg!(test) {
            return Err(io::ErrorKind::Unsupported.into());
        }
        check_once(installing)
    })
}

fn spawn_worker(
    mut delay: Duration,
    mut check: impl FnMut(&mut dyn FnMut()) -> io::Result<()> + Send + 'static,
) -> UpdateHandle {
    let (request, wake) = mpsc::channel();
    let status = Arc::new(Mutex::new(UpdateStatus::Idle));
    let worker_status = Arc::clone(&status);
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
                }
                let success = check(&mut || {
                    *worker_status.lock().unwrap_or_else(|err| err.into_inner()) =
                        UpdateStatus::Installing;
                })
                .is_ok();
                *worker_status.lock().unwrap_or_else(|err| err.into_inner()) = if success {
                    UpdateStatus::Current
                } else {
                    UpdateStatus::Failed
                };
                delay = if success { CHECK_PERIOD } else { RETRY_PERIOD };
            }
        });
    if spawned.is_err() {
        *status.lock().unwrap_or_else(|err| err.into_inner()) = UpdateStatus::Failed;
    }
    UpdateHandle { request, status }
}

fn check_once(installing: &mut dyn FnMut()) -> io::Result<()> {
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
    let Some(release) = download::latest(env!("CARGO_PKG_VERSION"))? else {
        return Ok(());
    };
    let portable = !registered_installation(&executable)?;
    let stage = stage::Stage::create(&stage_parent, elevated)?;
    {
        let mut output = stage.create_file()?;
        download::download(&release, &mut output)?;
        output.sync_all()?;
    }
    let mut locked = stage.lock_file()?;
    download::verify(&release, &mut locked)?;
    let args = installer_args(directory, portable);
    installing();
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
            installing();
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
