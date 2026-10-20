//! Windows credential files: protected owner-only ACLs, checked on handles.
//!
//! Used by observer credentials and the daemon's local operator key. No API
//! here changes an existing ACL to make an unsafe credential acceptable.

use std::ffi::c_void;
use std::fs::File;
use std::io;
use std::mem::{offset_of, size_of};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::Path;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{
    LocalFree, ERROR_INSUFFICIENT_BUFFER, GENERIC_READ, GENERIC_WRITE, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, GetSecurityInfo,
    SDDL_REVISION_1, SE_FILE_OBJECT,
};
use windows_sys::Win32::Security::{
    EqualSid, GetAce, GetSecurityDescriptorControl, GetTokenInformation, IsValidAcl, IsValidSid,
    TokenUser, ACCESS_ALLOWED_ACE, ACE_HEADER, ACL, DACL_SECURITY_INFORMATION,
    OWNER_SECURITY_INFORMATION, PSID, SECURITY_ATTRIBUTES, SE_DACL_PROTECTED, TOKEN_QUERY,
    TOKEN_USER,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, GetFileInformationByHandle, GetFileType, MoveFileExW, BY_HANDLE_FILE_INFORMATION,
    CREATE_NEW, FILE_ALL_ACCESS, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL,
    FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_DELETE, FILE_SHARE_READ,
    FILE_TYPE_DISK, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, OPEN_EXISTING,
};
use windows_sys::Win32::System::SystemServices::ACCESS_ALLOWED_ACE_TYPE;
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

struct LocalAllocation(*mut c_void);

impl Drop for LocalAllocation {
    fn drop(&mut self) {
        // SAFETY: every instance owns one allocation returned by a documented
        // LocalAlloc-based Windows security API, and is never copied.
        unsafe { LocalFree(self.0) };
    }
}

struct ProcessUser(Vec<usize>);

impl ProcessUser {
    fn load() -> io::Result<Self> {
        let mut raw = null_mut();
        // SAFETY: process pseudo-handle is valid; raw is writable. The token
        // handle is transferred exactly once to OwnedHandle on success.
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut raw) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let token = unsafe { OwnedHandle::from_raw_handle(raw) };
        let mut needed = 0;
        // SAFETY: a null, zero-length buffer requests the required byte count.
        unsafe {
            GetTokenInformation(token.as_raw_handle(), TokenUser, null_mut(), 0, &mut needed)
        };
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(ERROR_INSUFFICIENT_BUFFER as i32) {
            return Err(error);
        }
        if needed < size_of::<TOKEN_USER>() as u32 || needed > 4096 {
            return Err(io::Error::other("invalid process user information size"));
        }
        // usize provides TOKEN_USER alignment, unlike a Vec<u8>.
        let mut storage = vec![0usize; (needed as usize).div_ceil(size_of::<usize>())];
        // SAFETY: storage is aligned and has at least needed writable bytes.
        if unsafe {
            GetTokenInformation(
                token.as_raw_handle(),
                TokenUser,
                storage.as_mut_ptr().cast(),
                needed,
                &mut needed,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(storage))
    }

    fn sid(&self) -> PSID {
        // SAFETY: successful TokenUser retrieval initialized this aligned
        // TOKEN_USER and its SID within the still-owned buffer.
        unsafe { (*self.0.as_ptr().cast::<TOKEN_USER>()).User.Sid }
    }

    fn descriptor(&self) -> io::Result<LocalAllocation> {
        let mut text = null_mut();
        // SAFETY: sid remains valid for the call; text receives a LocalAlloc
        // NUL-terminated UTF-16 string owned by allocation below.
        if unsafe { ConvertSidToStringSidW(self.sid(), &mut text) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let allocation = LocalAllocation(text.cast());
        let mut len = 0;
        // SAFETY: ConvertSidToStringSidW guarantees a terminating NUL.
        while unsafe { *text.add(len) } != 0 {
            len += 1;
        }
        let sid = String::from_utf16(unsafe { std::slice::from_raw_parts(text, len) })
            .map_err(|_| io::Error::other("invalid process SID encoding"))?;
        drop(allocation);
        let sddl: Vec<u16> = format!("O:{sid}D:P(A;;FA;;;{sid})")
            .encode_utf16()
            .chain([0])
            .collect();
        let mut descriptor = null_mut();
        // SAFETY: sddl is NUL-terminated and descriptor is writable. D:P
        // disables inherited grants; the sole ACE grants this process user.
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                null_mut(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(LocalAllocation(descriptor))
    }
}

fn wide_path(path: &Path) -> io::Result<Vec<u16>> {
    let mut wide: Vec<_> = path.as_os_str().encode_wide().collect();
    if wide.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "credential path contains NUL",
        ));
    }
    wide.push(0);
    Ok(wide)
}

fn denied() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "credential must be a single-link regular file owned only by the process user",
    )
}

fn validate(file: &File, user: &ProcessUser) -> io::Result<()> {
    let handle = file.as_raw_handle();
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: borrowed live file handle and writable initialized output.
    if unsafe { GetFileInformationByHandle(handle, &mut info) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: GetFileType only inspects the live handle.
    if unsafe { GetFileType(handle) } != FILE_TYPE_DISK
        || info.dwFileAttributes & (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT) != 0
        || info.nNumberOfLinks != 1
    {
        return Err(denied());
    }
    let mut owner = null_mut();
    let mut dacl: *mut ACL = null_mut();
    let mut descriptor = null_mut();
    // SAFETY: all out-pointers are writable. Owner/DACL borrow the returned
    // self-relative descriptor and remain live until its allocation is dropped.
    let result = unsafe {
        GetSecurityInfo(
            handle,
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut owner,
            null_mut(),
            &mut dacl,
            null_mut(),
            &mut descriptor,
        )
    };
    if result != 0 {
        return Err(io::Error::from_raw_os_error(result as i32));
    }
    let _descriptor = LocalAllocation(descriptor);
    let mut control = 0;
    let mut revision = 0;
    // SAFETY: all pointers come from successful GetSecurityInfo. Null values
    // are refused before APIs that require a SID/ACL. No ACE pointers escape.
    unsafe {
        if descriptor.is_null()
            || owner.is_null()
            || dacl.is_null()
            || IsValidSid(owner) == 0
            || EqualSid(owner, user.sid()) == 0
            || IsValidAcl(dacl) == 0
            || (*dacl).AceCount != 1
        {
            return Err(denied());
        }
        if GetSecurityDescriptorControl(descriptor, &mut control, &mut revision) == 0 {
            return Err(io::Error::last_os_error());
        }
        if control & SE_DACL_PROTECTED == 0 {
            return Err(denied());
        }
        let mut raw_ace = null_mut();
        if GetAce(dacl, 0, &mut raw_ace) == 0 {
            return Err(io::Error::last_os_error());
        }
        if raw_ace.is_null() {
            return Err(denied());
        }
        let header = raw_ace.cast::<ACE_HEADER>().read_unaligned();
        let sid_offset = offset_of!(ACCESS_ALLOWED_ACE, SidStart);
        if header.AceType != ACCESS_ALLOWED_ACE_TYPE as u8
            || header.AceFlags != 0
            || (header.AceSize as usize) < sid_offset + 8
        {
            return Err(denied());
        }
        let ace = raw_ace.cast::<ACCESS_ALLOWED_ACE>().read_unaligned();
        if ace.Mask != FILE_ALL_ACCESS {
            return Err(denied());
        }
        let sid = raw_ace.cast::<u8>().add(sid_offset);
        let sid_len = 8 + 4 * usize::from(*sid.add(1));
        if sid_offset + sid_len > usize::from(ace.Header.AceSize)
            || IsValidSid(sid.cast()) == 0
            || EqualSid(sid.cast(), user.sid()) == 0
        {
            return Err(denied());
        }
    }
    Ok(())
}

fn open_file(path: &Path, creating: bool) -> io::Result<File> {
    let user = ProcessUser::load()?;
    let descriptor = user.descriptor()?;
    let attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.0,
        bInheritHandle: 0,
    };
    let path = wide_path(path)?;
    // SAFETY: path is terminated, attributes/descriptor stay alive during the
    // call, and inheritance is disabled. Existing entries are never truncated.
    let handle = unsafe {
        CreateFileW(
            path.as_ptr(),
            GENERIC_READ | if creating { GENERIC_WRITE } else { 0 },
            FILE_SHARE_READ | FILE_SHARE_DELETE,
            if creating { &attributes } else { null() },
            if creating { CREATE_NEW } else { OPEN_EXISTING },
            FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OPEN_REPARSE_POINT,
            null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: successful CreateFileW returns one owned file handle.
    let file = unsafe { File::from_raw_handle(handle) };
    validate(&file, &user)?;
    Ok(file)
}

/// Create a credential with its protected owner-only ACL already applied.
/// Fails rather than replacing an existing entry or repairing unsafe ACLs.
pub fn create(path: &Path) -> io::Result<File> {
    open_file(path, true)
}

/// Read a credential only after handle-based owner, ACL, type and link checks.
pub fn open(path: &Path) -> io::Result<File> {
    open_file(path, false)
}

/// Publish a fully written, synced, closed temporary credential. Windows uses
/// a write-through move; no cross-volume copy fallback is permitted.
pub(crate) fn publish(source: &Path, destination: &Path, replace: bool) -> io::Result<()> {
    let source = wide_path(source)?;
    let destination = wide_path(destination)?;
    let flags = MOVEFILE_WRITE_THROUGH
        | if replace {
            MOVEFILE_REPLACE_EXISTING
        } else {
            0
        };
    // SAFETY: both paths are terminated and live for the call.
    if unsafe { MoveFileExW(source.as_ptr(), destination.as_ptr(), flags) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use windows_sys::Win32::Security::Authorization::SetNamedSecurityInfoW;
    use windows_sys::Win32::Security::{
        GetSecurityDescriptorDacl, PROTECTED_DACL_SECURITY_INFORMATION,
        UNPROTECTED_DACL_SECURITY_INFORMATION,
    };

    fn write_private(path: &Path, bytes: &[u8]) {
        let mut file = create(path).unwrap();
        file.write_all(bytes).unwrap();
        file.sync_all().unwrap();
    }

    fn read_private(path: &Path) -> Vec<u8> {
        let mut bytes = Vec::new();
        open(path).unwrap().read_to_end(&mut bytes).unwrap();
        bytes
    }

    fn replace_test_dacl(path: &Path, sddl: &str, protected: bool) {
        let sddl: Vec<_> = sddl.encode_utf16().chain([0]).collect();
        let mut raw = null_mut();
        // SAFETY: test strings are terminated, output is writable, and the
        // returned descriptor stays owned until SetNamedSecurityInfo returns.
        unsafe {
            assert_ne!(
                ConvertStringSecurityDescriptorToSecurityDescriptorW(
                    sddl.as_ptr(),
                    SDDL_REVISION_1,
                    &mut raw,
                    null_mut()
                ),
                0
            );
            let _allocation = LocalAllocation(raw);
            let mut present = 0;
            let mut defaulted = 0;
            let mut dacl = null_mut();
            assert_ne!(
                GetSecurityDescriptorDacl(raw, &mut present, &mut dacl, &mut defaulted),
                0
            );
            assert_ne!(present, 0);
            assert_eq!(
                SetNamedSecurityInfoW(
                    wide_path(path).unwrap().as_ptr(),
                    SE_FILE_OBJECT,
                    DACL_SECURITY_INFORMATION
                        | if protected {
                            PROTECTED_DACL_SECURITY_INFORMATION
                        } else {
                            UNPROTECTED_DACL_SECURITY_INFORMATION
                        },
                    null_mut(),
                    null_mut(),
                    dacl,
                    null_mut()
                ),
                0
            );
        }
    }

    #[test]
    fn private_file_round_trip_and_create_never_truncates() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("credential");
        write_private(&path, b"first credential");
        assert_eq!(read_private(&path), b"first credential");
        assert_eq!(
            create(&path).unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
        assert_eq!(read_private(&path), b"first credential");
    }

    #[test]
    fn broad_and_null_acls_are_rejected_without_repair() {
        let directory = tempfile::tempdir().unwrap();
        for (name, sddl) in [
            ("everyone", "D:P(A;;FA;;;WD)"),
            ("null", "D:NO_ACCESS_CONTROL"),
        ] {
            let path = directory.path().join(name);
            write_private(&path, b"unchanged");
            replace_test_dacl(&path, sddl, true);
            assert_eq!(
                open(&path).unwrap_err().kind(),
                io::ErrorKind::PermissionDenied
            );
            assert_eq!(std::fs::read(&path).unwrap(), b"unchanged");
            // A second rejection proves the verifier did not silently repair it.
            assert!(open(&path).is_err());
        }
    }

    #[test]
    fn inheritable_credentials_are_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("credential");
        write_private(&path, b"private");
        // Keep the exact owner-only ACE, but remove protection from inherited
        // permissions. The verifier must refuse that unstable boundary too.
        let user = ProcessUser::load().unwrap();
        let descriptor = user.descriptor().unwrap();
        let mut present = 0;
        let mut defaulted = 0;
        let mut dacl = null_mut();
        // SAFETY: descriptor remains owned throughout, and the only target is
        // this test's private temporary file.
        unsafe {
            assert_ne!(
                GetSecurityDescriptorDacl(descriptor.0, &mut present, &mut dacl, &mut defaulted),
                0
            );
            assert_eq!(
                SetNamedSecurityInfoW(
                    wide_path(&path).unwrap().as_ptr(),
                    SE_FILE_OBJECT,
                    DACL_SECURITY_INFORMATION | UNPROTECTED_DACL_SECURITY_INFORMATION,
                    null_mut(),
                    null_mut(),
                    dacl,
                    null_mut()
                ),
                0
            );
        }
        assert!(open(&path).is_err());
    }

    #[test]
    fn directories_and_multiple_links_are_rejected() {
        let directory = tempfile::tempdir().unwrap();
        assert!(open(directory.path()).is_err());
        let path = directory.path().join("credential");
        let link = directory.path().join("second-name");
        write_private(&path, b"private");
        std::fs::hard_link(&path, &link).unwrap();
        assert!(open(&path).is_err());
        assert!(open(&link).is_err());
        std::fs::remove_file(link).unwrap();
        assert_eq!(read_private(&path), b"private");
    }

    #[test]
    fn file_reparse_points_are_rejected_without_following_or_repairing() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("credential");
        let link = directory.path().join("link");
        write_private(&path, b"private");
        // Native security validation requires a runner with symlink privileges;
        // absence is a failing fixture, never silently skipped coverage.
        std::os::windows::fs::symlink_file(&path, &link).unwrap();
        assert!(open(&link).is_err());
        assert!(create(&link).is_err());
        assert_eq!(read_private(&path), b"private");
        assert!(std::fs::symlink_metadata(link)
            .unwrap()
            .file_type()
            .is_symlink());
    }

    #[test]
    fn publication_preserves_a_winner_and_rotation_replaces_it() {
        let directory = tempfile::tempdir().unwrap();
        let first = directory.path().join("first");
        let second = directory.path().join("second");
        let published = directory.path().join("credential");
        write_private(&first, b"first");
        publish(&first, &published, false).unwrap();
        write_private(&second, b"second");
        assert_eq!(
            publish(&second, &published, false).unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
        assert_eq!(read_private(&published), b"first");
        publish(&second, &published, true).unwrap();
        assert_eq!(read_private(&published), b"second");
        assert!(!first.exists() && !second.exists());
    }
}
