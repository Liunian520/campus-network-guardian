use std::{
    ffi::{c_void, OsString},
    fs::{self, File, OpenOptions},
    io,
    os::windows::{
        ffi::{OsStrExt, OsStringExt},
        fs::{MetadataExt, OpenOptionsExt},
    },
    path::{Path, PathBuf},
    ptr::null_mut,
    time::{Duration, SystemTime},
};
use windows_sys::{
    core::GUID,
    Win32::{
        Foundation::LocalFree,
        Security::{
            Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW,
            SECURITY_ATTRIBUTES,
        },
        Storage::FileSystem::{
            CreateDirectoryW, FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_OPEN_REPARSE_POINT,
            FILE_SHARE_READ,
        },
        System::Com::{CoCreateGuid, CoTaskMemFree},
        UI::Shell::{FOLDERID_ProgramFiles, SHGetKnownFolderPath, KF_FLAG_DEFAULT_PATH},
    },
};

const PREFIX: &str = "CampusNetworkGuardian.Update.";
const INSTALLER: &str = "setup.exe";

pub(super) fn parent(elevated: bool) -> io::Result<PathBuf> {
    if !elevated {
        return Ok(std::env::temp_dir());
    }
    // A standard user's temporary directory is writable by their non-elevated
    // processes. Stage elevated updates beneath the system's Program Files.
    let mut value = null_mut();
    let result = unsafe {
        SHGetKnownFolderPath(
            &FOLDERID_ProgramFiles,
            KF_FLAG_DEFAULT_PATH as u32,
            null_mut(),
            &mut value,
        )
    };
    if result != 0 || value.is_null() {
        return Err(io::Error::other("Program Files location unavailable"));
    }
    let location = unsafe {
        let units = (0..).take_while(|n| *value.add(*n) != 0).count();
        let location = PathBuf::from(OsString::from_wide(std::slice::from_raw_parts(
            value, units,
        )));
        CoTaskMemFree(value.cast());
        location
    };
    if !plain_directory(&fs::symlink_metadata(&location)?) {
        return Err(io::Error::other(
            "Program Files is not a local staging directory",
        ));
    }
    Ok(location)
}

fn plain_directory(metadata: &fs::Metadata) -> bool {
    metadata.is_dir() && metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0
}

fn stage_name(name: &str) -> bool {
    name.strip_prefix(PREFIX).is_some_and(|suffix| {
        suffix.len() == 32 && suffix.bytes().all(|byte| byte.is_ascii_hexdigit())
    })
}

pub(super) fn clean_old(parent: &Path) {
    let Ok(entries) = fs::read_dir(parent) else {
        return;
    };
    let cutoff = Duration::from_secs(2 * 24 * 60 * 60);
    for entry in entries.flatten() {
        if !entry.file_name().to_str().is_some_and(stage_name) {
            continue;
        }
        let Ok(metadata) = fs::symlink_metadata(entry.path()) else {
            continue;
        };
        let old = metadata
            .modified()
            .ok()
            .and_then(|modified| SystemTime::now().duration_since(modified).ok())
            .is_some_and(|age| age >= cutoff);
        if old && plain_directory(&metadata) {
            let _ = fs::remove_file(entry.path().join(INSTALLER));
            let _ = fs::remove_dir(entry.path());
        }
    }
}

pub(super) struct Stage(PathBuf);

impl Stage {
    pub(super) fn create(parent: &Path, elevated: bool) -> io::Result<Self> {
        let mut guid = GUID::from_u128(0);
        if unsafe { CoCreateGuid(&mut guid) } != 0 {
            return Err(io::Error::other("could not reserve an update stage"));
        }
        let mut name = format!(
            "{PREFIX}{:08x}{:04x}{:04x}",
            guid.data1, guid.data2, guid.data3
        );
        use std::fmt::Write;
        for byte in guid.data4 {
            write!(name, "{byte:02x}").expect("writing to a String succeeds");
        }
        let directory = parent.join(name);
        if elevated {
            create_admin_directory(&directory)?;
        } else {
            fs::create_dir(&directory)?;
        }
        Ok(Self(directory))
    }

    pub(super) fn directory(&self) -> &Path {
        &self.0
    }

    pub(super) fn installer(&self) -> PathBuf {
        self.0.join(INSTALLER)
    }

    pub(super) fn create_file(&self) -> io::Result<File> {
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .share_mode(0)
            .open(self.installer())
    }

    pub(super) fn lock_file(&self) -> io::Result<File> {
        let file = OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(self.installer())?;
        let metadata = file.metadata()?;
        if !metadata.is_file() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(io::ErrorKind::InvalidData.into());
        }
        Ok(file)
    }
}

impl Drop for Stage {
    fn drop(&mut self) {
        let _ = fs::remove_file(self.installer());
        let _ = fs::remove_dir(&self.0);
    }
}

fn create_admin_directory(path: &Path) -> io::Result<()> {
    // The Administrators group owns the directory. The protected, inheritable
    // DACL gives ordinary users read/execute access to the official installer
    // while reserving writes and directory ownership to Administrators/System.
    let sddl: Vec<_> = "O:BAG:BAD:P(A;OICI;FA;;;BA)(A;OICI;FA;;;SY)(A;OICI;GRGX;;;BU)"
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let path: Vec<_> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut descriptor: *mut c_void = null_mut();
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            1,
            &mut descriptor,
            null_mut(),
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let security = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor,
        bInheritHandle: 0,
    };
    let result = if unsafe { CreateDirectoryW(path.as_ptr(), &security) } == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    };
    unsafe { LocalFree(descriptor) };
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_verified_handle_blocks_writes_and_deletion() {
        let stage = Stage::create(&std::env::temp_dir(), false).unwrap();
        drop(stage.create_file().unwrap());
        let verified = stage.lock_file().unwrap();
        assert!(OpenOptions::new()
            .write(true)
            .open(stage.installer())
            .is_err());
        assert!(fs::remove_file(stage.installer()).is_err());
        drop(verified);
        fs::remove_file(stage.installer()).unwrap();
    }
}
