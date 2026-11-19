mod download;
mod handoff;
mod stage;

use std::{
    ffi::OsString,
    fs, io,
    os::windows::{ffi::OsStringExt, process::CommandExt},
    path::{Path, PathBuf},
    process::Command,
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

pub fn start() {
    if cfg!(test) {
        return;
    }
    let delay = if std::env::args_os().any(|arg| arg == "--from-update") {
        CHECK_PERIOD
    } else {
        Duration::from_secs(20)
    };
    let _ = thread::Builder::new()
        .name("release-updater".into())
        .spawn(move || {
            thread::sleep(delay);
            loop {
                let delay = if check_once().is_ok() {
                    CHECK_PERIOD
                } else {
                    RETRY_PERIOD
                };
                thread::sleep(delay);
            }
        });
}

fn check_once() -> io::Result<()> {
    let executable = std::env::current_exe()?;
    if !executable
        .file_name()
        .is_some_and(|name| name.eq_ignore_ascii_case(EXECUTABLE))
    {
        return Ok(());
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
