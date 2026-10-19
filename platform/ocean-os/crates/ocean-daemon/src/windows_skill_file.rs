//! Windows skill and Room-package reads anchored to retained directory handles.
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
    FileDirectoryInformation, NtCreateFile, NtQueryDirectoryFile, FILE_DIRECTORY_FILE,
    FILE_DIRECTORY_INFORMATION, FILE_NON_DIRECTORY_FILE, FILE_OPEN, FILE_OPEN_REPARSE_POINT,
    FILE_SYNCHRONOUS_IO_NONALERT,
};
use windows_sys::Win32::Foundation::{
    RtlNtStatusToDosError, OBJ_CASE_INSENSITIVE, OBJ_DONT_REPARSE, STATUS_NO_MORE_FILES,
    UNICODE_STRING,
};
use windows_sys::Win32::Storage::FileSystem::{
    GetFileType, FILE_ATTRIBUTE_NORMAL, FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS,
    FILE_FLAG_OPEN_REPARSE_POINT, FILE_GENERIC_READ, FILE_SHARE_DELETE, FILE_SHARE_READ,
    FILE_SHARE_WRITE, FILE_TYPE_DISK,
};
use windows_sys::Win32::System::IO::IO_STATUS_BLOCK;

fn refused() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "path could not be opened without following a reparse point",
    )
}

fn validate(file: File, directory: Option<bool>) -> io::Result<File> {
    let metadata = file.metadata()?;
    // SAFETY: GetFileType only inspects this live borrowed file handle.
    if unsafe { GetFileType(file.as_raw_handle()) } != FILE_TYPE_DISK
        || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
        || match directory {
            Some(true) => !metadata.is_dir(),
            Some(false) => !metadata.is_file(),
            None => !metadata.is_dir() && !metadata.is_file(),
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
    validate(file, Some(true))
}

fn open_child(parent: &File, name: &OsStr, directory: bool) -> io::Result<File> {
    open_child_kind(parent, name, Some(directory), true)
}

fn open_child_kind(
    parent: &File,
    name: &OsStr,
    directory: Option<bool>,
    case_insensitive: bool,
) -> io::Result<File> {
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
        Attributes: OBJ_DONT_REPARSE
            | if case_insensitive {
                OBJ_CASE_INSENSITIVE
            } else {
                0
            },
        SecurityDescriptor: std::ptr::null(),
        SecurityQualityOfService: std::ptr::null(),
    };
    let mut handle = std::ptr::null_mut();
    let mut status_block = IO_STATUS_BLOCK::default();
    let kind = match directory {
        Some(true) => FILE_DIRECTORY_FILE,
        Some(false) => FILE_NON_DIRECTORY_FILE,
        None => 0,
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
    if status < 0 {
        // SAFETY: pure conversion of an NT status; no borrowed pointers.
        return Err(io::Error::from_raw_os_error(
            unsafe { RtlNtStatusToDosError(status) } as i32,
        ));
    }
    if handle.is_null() {
        return Err(refused());
    }
    // SAFETY: NtCreateFile returned one fresh owned handle on success.
    validate(unsafe { File::from_raw_handle(handle) }, directory)
}

pub(super) fn open(path: &Path) -> io::Result<File> {
    open_path(path, false)
}

pub(super) fn open_directory(path: &Path) -> io::Result<File> {
    open_path(path, true)
}

/// Enumerated names are exact, including on case-sensitive Windows directories.
pub(super) fn open_directory_at(parent: &File, name: &str) -> io::Result<File> {
    open_child_kind(parent, OsStr::new(name), Some(true), false)
}

pub(super) fn open_entry_at(parent: &File, name: &str) -> io::Result<File> {
    open_child_kind(parent, OsStr::new(name), None, false)
}

fn open_path(path: &Path, directory: bool) -> io::Result<File> {
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
        current = open_child(&current, name, directory || index + 1 < names.len())?;
    }
    Ok(current)
}

#[derive(Debug)]
pub(super) enum DirectoryReadError {
    Unavailable,
    InvalidName,
    Limit,
}

fn directory_entry_name(bytes: &[u8]) -> Result<String, DirectoryReadError> {
    let name_offset = std::mem::offset_of!(FILE_DIRECTORY_INFORMATION, FileName);
    let length_offset = std::mem::offset_of!(FILE_DIRECTORY_INFORMATION, FileNameLength);
    if bytes.len() < name_offset || bytes[..4] != [0; 4] {
        return Err(DirectoryReadError::InvalidName);
    }
    let length =
        u32::from_le_bytes(bytes[length_offset..length_offset + 4].try_into().unwrap()) as usize;
    if length == 0 || !length.is_multiple_of(2) || length > bytes.len() - name_offset {
        return Err(DirectoryReadError::InvalidName);
    }
    let units = bytes[name_offset..name_offset + length]
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect::<Vec<_>>();
    let name = String::from_utf16(&units).map_err(|_| DirectoryReadError::InvalidName)?;
    if name.contains('/')
        || name.contains('\\')
        || name.contains(':')
        || name.chars().any(char::is_control)
    {
        return Err(DirectoryReadError::InvalidName);
    }
    Ok(name)
}

/// Enumerate through the retained handle, never its current filesystem pathname.
/// One entry per synchronous query keeps parsing and the entry budget explicit.
pub(super) fn directory_names(
    directory: &File,
    remaining: usize,
) -> Result<Vec<String>, DirectoryReadError> {
    // The NT directory record requires eight-byte alignment. Keep the bounded
    // buffer on the heap so recursive package capture cannot exhaust the stack.
    let mut buffer = vec![0_u64; 8192];
    let mut names = Vec::new();
    let mut restart = true;
    loop {
        let mut status_block = IO_STATUS_BLOCK::default();
        // SAFETY: directory owns a synchronous handle. The aligned initialized
        // buffer and status block remain live throughout this synchronous call;
        // no event or callback is installed. Request exactly one directory row.
        let status = unsafe {
            NtQueryDirectoryFile(
                directory.as_raw_handle(),
                std::ptr::null_mut(),
                None,
                std::ptr::null(),
                &mut status_block,
                buffer.as_mut_ptr().cast(),
                (buffer.len() * 8) as u32,
                FileDirectoryInformation,
                true,
                std::ptr::null(),
                restart,
            )
        };
        if status == STATUS_NO_MORE_FILES {
            break;
        }
        if status != 0
            || status_block.Information == 0
            || status_block.Information > buffer.len() * 8
        {
            return Err(DirectoryReadError::Unavailable);
        }
        restart = false;
        // SAFETY: Information is checked against the initialized allocation and
        // the API reports exactly the bytes written for this completed query.
        let bytes = unsafe {
            std::slice::from_raw_parts(buffer.as_ptr().cast::<u8>(), status_block.Information)
        };
        let name = directory_entry_name(bytes)?;
        if name == "." || name == ".." {
            continue;
        }
        if names.len() >= remaining {
            return Err(DirectoryReadError::Limit);
        }
        names.push(name);
    }
    names.sort();
    Ok(names)
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

    #[test]
    fn package_directory_reads_are_sorted_bounded_and_handle_relative() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("prompts")).unwrap();
        std::fs::write(root.path().join("AGENT.md"), "agent").unwrap();
        std::fs::write(root.path().join("prompts/é.md"), "instructions").unwrap();
        let directory = open_directory(root.path()).unwrap();
        assert_eq!(
            directory_names(&directory, 2).unwrap(),
            ["AGENT.md", "prompts"]
        );
        assert!(matches!(
            directory_names(&directory, 1),
            Err(DirectoryReadError::Limit)
        ));
        // Restarting enumeration on the same retained handle is deterministic.
        assert_eq!(directory_names(&directory, 2).unwrap().len(), 2);
        let prompts = open_directory_at(&directory, "prompts").unwrap();
        assert_eq!(directory_names(&prompts, 1).unwrap(), ["é.md"]);
        let mut body = String::new();
        open_entry_at(&prompts, "é.md")
            .unwrap()
            .read_to_string(&mut body)
            .unwrap();
        assert_eq!(body, "instructions");
        assert!(open_entry_at(&directory, "prompts")
            .unwrap()
            .metadata()
            .unwrap()
            .is_dir());
        assert!(open_directory_at(&directory, "AGENT.md").is_err());
        assert_eq!(
            open_directory_at(&directory, "missing").unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
    }

    #[test]
    fn package_enumeration_keeps_the_retained_root_after_retargeting() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let package = root.path().join("package");
        std::fs::create_dir(&package).unwrap();
        std::fs::write(package.join("inside.md"), "inside").unwrap();
        std::fs::write(outside.path().join("outside.md"), "outside").unwrap();
        let captured = open_directory(&package).unwrap();
        std::fs::rename(&package, root.path().join("retained")).unwrap();
        symlink_dir(outside.path(), &package).unwrap();
        assert_eq!(directory_names(&captured, 1).unwrap(), ["inside.md"]);
        let mut body = String::new();
        open_entry_at(&captured, "inside.md")
            .unwrap()
            .read_to_string(&mut body)
            .unwrap();
        assert_eq!(body, "inside");
        assert!(open_directory(&package).is_err());
        assert!(open_entry_at(&captured, "outside.md").is_err());
    }

    #[test]
    fn package_entries_refuse_reparse_points_and_path_components() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let target = outside.path().join("target.md");
        std::fs::write(&target, "outside").unwrap();
        symlink_file(&target, root.path().join("file-link")).unwrap();
        symlink_dir(outside.path(), root.path().join("directory-link")).unwrap();
        let captured = open_directory(root.path()).unwrap();
        for name in [
            "file-link",
            "directory-link",
            ".",
            "..",
            "target.md:secret",
            "a/b",
            "a\\b",
        ] {
            assert!(open_entry_at(&captured, name).is_err(), "accepted {name}");
        }
        assert_eq!(std::fs::read_to_string(target).unwrap(), "outside");
    }

    #[test]
    fn empty_package_directory_respects_zero_budget() {
        let root = tempfile::tempdir().unwrap();
        let captured = open_directory(root.path()).unwrap();
        assert!(directory_names(&captured, 0).unwrap().is_empty());
        std::fs::write(root.path().join("one"), "content").unwrap();
        assert!(matches!(
            directory_names(&captured, 0),
            Err(DirectoryReadError::Limit)
        ));
    }

    #[test]
    fn directory_records_reject_incomplete_or_ambiguous_names() {
        fn record(units: &[u16]) -> Vec<u8> {
            let offset = std::mem::offset_of!(FILE_DIRECTORY_INFORMATION, FileName);
            let length_offset = std::mem::offset_of!(FILE_DIRECTORY_INFORMATION, FileNameLength);
            let mut bytes = vec![0; offset];
            bytes[length_offset..length_offset + 4]
                .copy_from_slice(&((units.len() * 2) as u32).to_le_bytes());
            for unit in units {
                bytes.extend_from_slice(&unit.to_le_bytes());
            }
            bytes
        }
        let valid = record(&"é.md".encode_utf16().collect::<Vec<_>>());
        assert_eq!(directory_entry_name(&valid).unwrap(), "é.md");
        assert!(directory_entry_name(&[]).is_err());
        assert!(directory_entry_name(&valid[..valid.len() - 1]).is_err());
        assert!(directory_entry_name(&record(&[0xd800])).is_err());
        for name in ["a/b", "a\\b", "a:stream", "a\0b", "a\nb", ""] {
            assert!(
                directory_entry_name(&record(&name.encode_utf16().collect::<Vec<_>>())).is_err()
            );
        }
        let mut chained = valid.clone();
        chained[0] = 4;
        assert!(directory_entry_name(&chained).is_err());
        let mut odd = valid;
        odd[std::mem::offset_of!(FILE_DIRECTORY_INFORMATION, FileNameLength)] = 1;
        assert!(directory_entry_name(&odd).is_err());
    }
}
