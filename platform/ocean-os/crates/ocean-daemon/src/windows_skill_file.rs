//! Windows skill-file reads anchored to retained directory handles.
//!
//! Only absolute local-drive paths are accepted. Each component is opened
//! relative to the preceding handle and every reparse point is refused.

use std::ffi::OsStr;
use std::fs::{File, OpenOptions};
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::path::{Component, Path, Prefix};
use windows_sys::Wdk::Foundation::OBJECT_ATTRIBUTES;
use windows_sys::Wdk::Storage::FileSystem::{
    NtCreateFile, FILE_DIRECTORY_FILE, FILE_NON_DIRECTORY_FILE, FILE_OPEN, FILE_OPEN_REPARSE_POINT,
    FILE_SYNCHRONOUS_IO_NONALERT,
};
use windows_sys::Win32::Foundation::{OBJ_CASE_INSENSITIVE, OBJ_DONT_REPARSE, UNICODE_STRING};
use windows_sys::Win32::Storage::FileSystem::{
    GetFileType, FILE_ATTRIBUTE_NORMAL, FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS,
    FILE_FLAG_OPEN_REPARSE_POINT, FILE_GENERIC_READ, FILE_SHARE_DELETE, FILE_SHARE_READ,
    FILE_SHARE_WRITE, FILE_TYPE_DISK,
};
use windows_sys::Win32::System::IO::IO_STATUS_BLOCK;

fn refused() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "skill path could not be opened without following a reparse point",
    )
}

fn validate(file: File, directory: bool) -> io::Result<File> {
    let metadata = file.metadata()?;
    // SAFETY: GetFileType only inspects this live borrowed file handle.
    if unsafe { GetFileType(file.as_raw_handle()) } != FILE_TYPE_DISK
        || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
        || if directory {
            !metadata.is_dir()
        } else {
            !metadata.is_file()
        }
    {
        return Err(refused());
    }
    Ok(file)
}

fn open_drive(drive: u8) -> io::Result<File> {
    if !drive.is_ascii_alphabetic() {
        return Err(refused());
    }
    let root = format!(r"\\?\{}:\", char::from(drive));
    let file = OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(root)?;
    validate(file, true)
}

fn open_child(parent: &File, name: &OsStr, directory: bool) -> io::Result<File> {
    let units: Vec<u16> = name.encode_wide().collect();
    if units.is_empty()
        || units
            .iter()
            .any(|unit| [0, u16::from(b'/'), u16::from(b'\\'), u16::from(b':')].contains(unit))
        || name == OsStr::new(".")
        || name == OsStr::new("..")
    {
        return Err(refused());
    }
    let length = units
        .len()
        .checked_mul(2)
        .and_then(|n| u16::try_from(n).ok())
        .ok_or_else(refused)?;
    let name = UNICODE_STRING {
        Length: length,
        MaximumLength: length,
        Buffer: units.as_ptr().cast_mut(),
    };
    let attributes = OBJECT_ATTRIBUTES {
        Length: std::mem::size_of::<OBJECT_ATTRIBUTES>() as u32,
        RootDirectory: parent.as_raw_handle(),
        ObjectName: &name,
        Attributes: OBJ_CASE_INSENSITIVE | OBJ_DONT_REPARSE,
        SecurityDescriptor: std::ptr::null(),
        SecurityQualityOfService: std::ptr::null(),
    };
    let mut handle = std::ptr::null_mut();
    let mut status_block = IO_STATUS_BLOCK::default();
    let kind = if directory {
        FILE_DIRECTORY_FILE
    } else {
        FILE_NON_DIRECTORY_FILE
    };
    // SAFETY: every pointer names initialized storage for this synchronous
    // call. The name is one validated component and the parent handle remains
    // owned throughout. Successful output transfers exactly once into File.
    let status = unsafe {
        NtCreateFile(
            &mut handle,
            FILE_GENERIC_READ,
            &attributes,
            &mut status_block,
            std::ptr::null(),
            FILE_ATTRIBUTE_NORMAL,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            FILE_OPEN,
            kind | FILE_OPEN_REPARSE_POINT | FILE_SYNCHRONOUS_IO_NONALERT,
            std::ptr::null(),
            0,
        )
    };
    if status < 0 || handle.is_null() {
        return Err(refused());
    }
    // SAFETY: NtCreateFile returned one fresh owned handle on success.
    validate(unsafe { File::from_raw_handle(handle) }, directory)
}

pub(super) fn open(path: &Path) -> io::Result<File> {
    let mut components = path.components();
    let drive = match components.next() {
        Some(Component::Prefix(prefix)) => match prefix.kind() {
            Prefix::Disk(drive) | Prefix::VerbatimDisk(drive) => drive,
            _ => return Err(refused()),
        },
        _ => return Err(refused()),
    };
    if components.next() != Some(Component::RootDir) {
        return Err(refused());
    }
    let names = components
        .map(|component| match component {
            Component::Normal(name) => Ok(name),
            _ => Err(refused()),
        })
        .collect::<io::Result<Vec<_>>>()?;
    if names.is_empty() {
        return Err(refused());
    }
    let mut current = open_drive(drive)?;
    for (index, name) in names.iter().enumerate() {
        current = open_child(&current, name, index + 1 < names.len())?;
    }
    Ok(current)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::os::windows::fs::{symlink_dir, symlink_file};

    fn read(path: &Path) -> io::Result<String> {
        let mut body = String::new();
        open(path)?.read_to_string(&mut body)?;
        Ok(body)
    }

    #[test]
    fn local_and_verbatim_paths_read_the_same_unicode_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("skill é.md");
        std::fs::write(&path, "inside").unwrap();
        assert_eq!(read(&path).unwrap(), "inside");
        assert_eq!(
            read(&std::fs::canonicalize(&path).unwrap()).unwrap(),
            "inside"
        );
        assert!(open(dir.path()).is_err());
    }

    #[test]
    fn final_and_ancestor_reparse_points_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let target = outside.path().join("SKILL.md");
        std::fs::write(&target, "outside").unwrap();
        let file_link = dir.path().join("file-link");
        symlink_file(&target, &file_link).unwrap();
        assert!(read(&file_link).is_err());
        let directory_link = dir.path().join("directory-link");
        symlink_dir(outside.path(), &directory_link).unwrap();
        assert!(read(&directory_link.join("SKILL.md")).is_err());
        assert_eq!(std::fs::read_to_string(target).unwrap(), "outside");
    }

    #[test]
    fn relative_parent_device_unc_and_stream_paths_are_refused() {
        for path in [
            r"relative\SKILL.md",
            r"C:SKILL.md",
            r"C:\a\..\SKILL.md",
            r"\\.\NUL",
            r"\\server\share\SKILL.md",
            r"C:\a\SKILL.md:secret",
        ] {
            assert!(open(Path::new(path)).is_err(), "accepted {path}");
        }
    }

    #[test]
    fn a_retained_parent_does_not_follow_its_replacement() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let parent = dir.path().join("parent");
        std::fs::create_dir(&parent).unwrap();
        std::fs::write(parent.join("SKILL.md"), "inside").unwrap();
        std::fs::write(outside.path().join("SKILL.md"), "outside").unwrap();
        // Traverse to the directory with the same guarded primitives, then
        // retarget its pathname before the final relative open.
        let canonical = std::fs::canonicalize(&parent).unwrap();
        let mut parts = canonical.components();
        let Some(Component::Prefix(prefix)) = parts.next() else {
            panic!("drive prefix")
        };
        let Prefix::VerbatimDisk(drive) = prefix.kind() else {
            panic!("verbatim drive")
        };
        assert_eq!(parts.next(), Some(Component::RootDir));
        let mut held = open_drive(drive).unwrap();
        for part in parts {
            let Component::Normal(name) = part else {
                panic!("normal component")
            };
            held = open_child(&held, name, true).unwrap();
        }
        std::fs::rename(&parent, dir.path().join("retained")).unwrap();
        symlink_dir(outside.path(), &parent).unwrap();
        let mut body = String::new();
        open_child(&held, OsStr::new("SKILL.md"), false)
            .unwrap()
            .read_to_string(&mut body)
            .unwrap();
        assert_eq!(body, "inside");
        assert!(read(&parent.join("SKILL.md")).is_err());
    }
}
