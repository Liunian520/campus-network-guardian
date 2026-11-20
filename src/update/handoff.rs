use std::{
    ffi::{c_void, OsStr, OsString},
    io,
    os::windows::ffi::OsStrExt,
    path::Path,
    ptr::{null, null_mut},
};
use windows_sys::{
    core::w,
    Win32::{
        Foundation::{
            CloseHandle, GetLastError, LocalFree, ERROR_ALREADY_EXISTS, ERROR_INSUFFICIENT_BUFFER,
            HANDLE, WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT,
        },
        Security::{
            Authorization::{
                ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
            },
            DuplicateTokenEx, EqualSid, GetTokenInformation, SecurityImpersonation, TokenElevation,
            TokenPrimary, TokenUser, PSID, SECURITY_ATTRIBUTES, TOKEN_ASSIGN_PRIMARY,
            TOKEN_DUPLICATE, TOKEN_ELEVATION, TOKEN_QUERY, TOKEN_USER,
        },
        System::Threading::{
            CreateEventW, CreateProcessWithTokenW, GetCurrentProcess, GetCurrentProcessId,
            GetExitCodeProcess, OpenProcess, OpenProcessToken, WaitForMultipleObjects,
            WaitForSingleObject, CREATE_NO_WINDOW, LOGON_WITH_PROFILE, PROCESS_INFORMATION,
            PROCESS_QUERY_LIMITED_INFORMATION, STARTUPINFOW,
        },
        UI::WindowsAndMessaging::{
            FindWindowW, GetShellWindow, GetWindowThreadProcessId, PostMessageW, WM_COMMAND,
        },
    },
};

fn command_line(executable: &Path, arguments: &[OsString]) -> io::Result<Vec<u16>> {
    const QUOTE: u16 = b'"' as u16;
    const BACKSLASH: u16 = b'\\' as u16;
    let mut line = Vec::new();
    for argument in
        std::iter::once(executable.as_os_str()).chain(arguments.iter().map(OsString::as_os_str))
    {
        if !line.is_empty() {
            line.push(b' ' as u16);
        }
        line.push(QUOTE);
        let mut backslashes = 0;
        for unit in argument.encode_wide() {
            if unit == 0 {
                return Err(io::ErrorKind::InvalidInput.into());
            }
            if unit == BACKSLASH {
                backslashes += 1;
                continue;
            }
            if unit == QUOTE {
                line.extend(std::iter::repeat_n(BACKSLASH, backslashes * 2 + 1));
            } else {
                line.extend(std::iter::repeat_n(BACKSLASH, backslashes));
            }
            backslashes = 0;
            line.push(unit);
        }
        line.extend(std::iter::repeat_n(BACKSLASH, backslashes * 2));
        line.push(QUOTE);
    }
    line.push(0);
    Ok(line)
}

fn ready_name(stage_name: &OsStr) -> io::Result<String> {
    let guid = stage_name
        .to_str()
        .and_then(|name| name.strip_prefix("CampusNetworkGuardian.Update."))
        .filter(|suffix| suffix.len() == 32 && suffix.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .ok_or(io::ErrorKind::InvalidInput)?;
    Ok(format!("CampusNetworkGuardian.UpdateReady.{guid}"))
}

struct Handle(HANDLE);

impl Handle {
    fn new(raw: HANDLE) -> io::Result<Self> {
        if raw.is_null() {
            Err(io::Error::last_os_error())
        } else {
            Ok(Self(raw))
        }
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        // The wrapper owns a real Win32 handle, never a pseudo-handle.
        unsafe { CloseHandle(self.0) };
    }
}

struct LocalAllocation(*mut c_void);

impl Drop for LocalAllocation {
    fn drop(&mut self) {
        unsafe { LocalFree(self.0) };
    }
}

fn check(result: i32) -> io::Result<()> {
    if result == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn open_token(process: HANDLE, access: u32) -> io::Result<Handle> {
    let mut token = null_mut();
    // The caller supplies a valid process or the current-process pseudo-handle.
    check(unsafe { OpenProcessToken(process, access, &mut token) })?;
    Handle::new(token)
}

struct TokenUserData(Vec<usize>);

impl TokenUserData {
    fn read(token: &Handle) -> io::Result<Self> {
        let mut size = 0;
        let result = unsafe { GetTokenInformation(token.0, TokenUser, null_mut(), 0, &mut size) };
        if result != 0 || unsafe { GetLastError() } != ERROR_INSUFFICIENT_BUFFER {
            return Err(io::Error::last_os_error());
        }
        if (size as usize) < std::mem::size_of::<TOKEN_USER>() {
            return Err(io::ErrorKind::InvalidData.into());
        }
        // Pointer alignment is required for the TOKEN_USER and its embedded SID.
        let mut storage = vec![0usize; (size as usize).div_ceil(std::mem::size_of::<usize>())];
        check(unsafe {
            GetTokenInformation(
                token.0,
                TokenUser,
                storage.as_mut_ptr().cast(),
                size,
                &mut size,
            )
        })?;
        Ok(Self(storage))
    }

    fn sid(&self) -> PSID {
        // GetTokenInformation populated the aligned TOKEN_USER and SID in this buffer.
        unsafe { (*self.0.as_ptr().cast::<TOKEN_USER>()).User.Sid }
    }
}

fn shell_token() -> io::Result<(Handle, TokenUserData)> {
    let shell = unsafe { GetShellWindow() };
    let mut process_id = 0;
    if shell.is_null()
        || unsafe { GetWindowThreadProcessId(shell, &mut process_id) } == 0
        || process_id == 0
    {
        return Err(io::Error::other("the desktop shell is unavailable"));
    }
    let process =
        Handle::new(unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, process_id) })?;
    let shell = open_token(process.0, TOKEN_QUERY | TOKEN_DUPLICATE)?;
    let current = open_token(unsafe { GetCurrentProcess() }, TOKEN_QUERY)?;
    let user = TokenUserData::read(&shell)?;
    let expected_user = TokenUserData::read(&current)?;
    let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
    let mut returned = 0;
    check(unsafe {
        GetTokenInformation(
            shell.0,
            TokenElevation,
            (&mut elevation as *mut TOKEN_ELEVATION).cast(),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut returned,
        )
    })?;
    if returned as usize != std::mem::size_of::<TOKEN_ELEVATION>()
        || elevation.TokenIsElevated != 0
        || unsafe { EqualSid(user.sid(), expected_user.sid()) } == 0
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "no unelevated shell for the same user",
        ));
    }
    let mut primary = null_mut();
    check(unsafe {
        DuplicateTokenEx(
            shell.0,
            TOKEN_QUERY | TOKEN_ASSIGN_PRIMARY | TOKEN_DUPLICATE,
            null(),
            SecurityImpersonation,
            TokenPrimary,
            &mut primary,
        )
    })?;
    Ok((Handle::new(primary)?, user))
}

fn ready_event(name: &str, sid: PSID) -> io::Result<Handle> {
    let mut sid_buffer = null_mut();
    check(unsafe { ConvertSidToStringSidW(sid, &mut sid_buffer) })?;
    let allocation = LocalAllocation(sid_buffer.cast());
    // ConvertSidToStringSidW returns a null-terminated Windows string.
    let length = unsafe { (0..).take_while(|n| *sid_buffer.add(*n) != 0).count() };
    let sid_text =
        unsafe { String::from_utf16_lossy(std::slice::from_raw_parts(sid_buffer, length)) };
    drop(allocation);
    // Allow the same medium-integrity user to signal readiness, but give
    // full access only to administrators and SYSTEM.
    let descriptor = format!("D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;0x2;;;{sid_text})S:(ML;;NW;;;ME)");
    let descriptor = wide(OsStr::new(&descriptor));
    let mut attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: null_mut(),
        bInheritHandle: 0,
    };
    check(unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            descriptor.as_ptr(),
            1,
            &mut attributes.lpSecurityDescriptor,
            null_mut(),
        )
    })?;
    let _allocation = LocalAllocation(attributes.lpSecurityDescriptor);
    let name = wide(OsStr::new(&format!("Local\\{name}")));
    let raw = unsafe { CreateEventW(&attributes, 1, 0, name.as_ptr()) };
    let error = unsafe { GetLastError() };
    let event = Handle::new(raw)?;
    if error == ERROR_ALREADY_EXISTS {
        return Err(io::ErrorKind::AlreadyExists.into());
    }
    Ok(event)
}

fn wide(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(Some(0)).collect()
}

fn start_install(
    token: &Handle,
    installer: &Path,
    directory: &Path,
    arguments: &[OsString],
) -> io::Result<Handle> {
    let mut command = command_line(installer, arguments)?;
    let installer = wide(installer.as_os_str());
    let directory = wide(directory.as_os_str());
    let mut desktop = wide(OsStr::new(r"winsta0\default"));
    // Win32 structs are zero initialized, then the fields required by the API are set.
    let mut startup: STARTUPINFOW = unsafe { std::mem::zeroed() };
    startup.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
    startup.lpDesktop = desktop.as_mut_ptr();
    let mut process: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    check(unsafe {
        CreateProcessWithTokenW(
            token.0,
            LOGON_WITH_PROFILE,
            installer.as_ptr(),
            command.as_mut_ptr(),
            CREATE_NO_WINDOW,
            null(),
            directory.as_ptr(),
            &startup,
            &mut process,
        )
    })?;
    let _thread = Handle::new(process.hThread)?;
    Handle::new(process.hProcess)
}

pub(super) fn launch_and_handoff(
    installer: &Path,
    directory: &Path,
    args: &[OsString],
    stage_name: &OsStr,
) -> io::Result<()> {
    if !installer.is_absolute() || !directory.is_absolute() {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    let name = ready_name(stage_name)?;
    let (token, user) = shell_token()?;
    let event = ready_event(&name, user.sid())?;
    let owner = unsafe { GetCurrentProcessId() };
    let mut arguments = args.to_vec();
    arguments.push(format!("/GUARDIANOWNER={owner}").into());
    arguments.push(format!("/GUARDIANREADY={name}").into());
    let process = start_install(&token, installer, directory, &arguments)?;
    let handles = [process.0, event.0];
    // Prefer process termination when both handles are signaled.
    match unsafe { WaitForMultipleObjects(handles.len() as u32, handles.as_ptr(), 0, 30_000) } {
        value if value == WAIT_OBJECT_0 + 1 => {}
        WAIT_FAILED => return Err(io::Error::last_os_error()),
        WAIT_TIMEOUT => return Err(io::ErrorKind::TimedOut.into()),
        _ => {
            return Err(io::Error::other(
                "the unelevated installer exited before readiness",
            ))
        }
    }
    if unsafe { WaitForSingleObject(process.0, 0) } != WAIT_TIMEOUT {
        return Err(io::Error::other(
            "the unelevated installer is no longer running",
        ));
    }
    let window = unsafe { FindWindowW(w!("CampusNetworkGuardianWindow"), null()) };
    let mut window_owner = 0;
    if window.is_null()
        || unsafe { GetWindowThreadProcessId(window, &mut window_owner) } == 0
        || window_owner != owner
    {
        return Err(io::Error::other(
            "could not verify the current Guardian window",
        ));
    }
    check(unsafe { PostMessageW(window, WM_COMMAND, 204, 0) })?;
    // Keep the parent's verified installer handle and stage alive during the
    // handoff. Normally this process exits before the wait completes.
    match unsafe { WaitForSingleObject(process.0, 30_000) } {
        WAIT_OBJECT_0 => {
            let mut exit_code = 0;
            check(unsafe { GetExitCodeProcess(process.0, &mut exit_code) })?;
            if exit_code != 0 {
                return Err(io::Error::other("silent release update failed"));
            }
            Ok(())
        }
        WAIT_TIMEOUT => Err(io::ErrorKind::TimedOut.into()),
        _ => Err(io::Error::last_os_error()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::{Foundation::LocalFree, UI::Shell::CommandLineToArgvW};

    #[test]
    fn command_line_round_trips_windows_paths_and_escaped_arguments() {
        let path = Path::new(r"C:\Program Files\校园网守护\setup.exe");
        let arguments = [
            OsString::new(),
            "/VERYSILENT".into(),
            r"/DIR=C:\Users\新 用户\校园网\".into(),
            r#"quote"inside"#.into(),
            r#"backslash\"quote\"#.into(),
        ];
        let line = command_line(path, &arguments).unwrap();
        let mut count = 0;
        let decoded = unsafe { CommandLineToArgvW(line.as_ptr(), &mut count) };
        assert!(!decoded.is_null());
        let actual: Vec<_> = (0..count as usize)
            .map(|index| unsafe {
                let value = *decoded.add(index);
                let length = (0..).take_while(|offset| *value.add(*offset) != 0).count();
                OsString::from_wide(std::slice::from_raw_parts(value, length))
            })
            .collect();
        unsafe { LocalFree(decoded.cast()) };
        let expected = std::iter::once(path.as_os_str().to_os_string())
            .chain(arguments)
            .collect::<Vec<_>>();
        assert_eq!(actual, expected);
    }

    #[test]
    fn rejects_an_embedded_nul() {
        let bad = OsString::from_wide(&[b'a' as u16, 0, b'b' as u16]);
        assert!(command_line(Path::new("setup.exe"), &[bad]).is_err());
    }

    #[test]
    fn event_parameter_uses_only_the_stage_guid() {
        let guid = "ad5a0cbd2dbc078942bbc8a8d7873e68";
        assert_eq!(
            ready_name(OsStr::new(&format!("CampusNetworkGuardian.Update.{guid}"))).unwrap(),
            format!("CampusNetworkGuardian.UpdateReady.{guid}")
        );
        assert!(ready_name(OsStr::new("CampusNetworkGuardian.Update.98")).is_err());
        assert!(ready_name(OsStr::new(&format!("CampusNetworkGuardian.Update.{guid}x"))).is_err());
    }
}
