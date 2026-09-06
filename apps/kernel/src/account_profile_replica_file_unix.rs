use super::*;
use std::ffi::CString;
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::fs::OpenOptionsExt;

/// Keep the parent directory open across snapshot, publication and rollback.
/// A provider replacing a path with a symlink cannot redirect these operations.
pub(super) struct ReplicaFile {
    parent: fs::File,
    name: CString,
}

impl ReplicaFile {
    pub(super) fn open(root: &Path, relative: &Path) -> Result<Self, DaemonError> {
        let mut parent = fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(root)
            .map_err(registry_io("open account refresh root"))?;
        let mut components = relative.components().peekable();
        while let Some(component) = components.next() {
            let std::path::Component::Normal(component) = component else {
                return Err(registry_error(
                    "refresh account profile",
                    "invalid account file path",
                ));
            };
            use std::os::unix::ffi::OsStrExt;
            let name = CString::new(component.as_bytes()).map_err(|_| {
                registry_error("refresh account profile", "invalid account filename")
            })?;
            if components.peek().is_none() {
                return Ok(Self { parent, name });
            }
            let flags = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
            let child = match open_at(&parent, &name, flags) {
                Ok(child) => child,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    // SAFETY: the directory fd and NUL-terminated component are live.
                    let result = unsafe { libc::mkdirat(parent.as_raw_fd(), name.as_ptr(), 0o700) };
                    if result < 0 {
                        let error = std::io::Error::last_os_error();
                        if error.kind() != std::io::ErrorKind::AlreadyExists {
                            return Err(registry_io("create account refresh directory")(error));
                        }
                    }
                    parent
                        .sync_all()
                        .map_err(registry_io("sync account refresh directory"))?;
                    open_at(&parent, &name, flags)
                        .map_err(registry_io("open account refresh directory"))?
                }
                Err(error) => return Err(registry_io("open account refresh directory")(error)),
            };
            parent = child;
        }
        Err(registry_error(
            "refresh account profile",
            "empty account file path",
        ))
    }

    pub(super) fn read(&self, maximum: usize) -> Result<Option<Vec<u8>>, DaemonError> {
        let file = match open_at(
            &self.parent,
            &self.name,
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
        ) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(registry_io("read account refresh backup")(error)),
        };
        let metadata = file
            .metadata()
            .map_err(registry_io("read account refresh backup"))?;
        if !metadata.is_file() || metadata.len() > maximum as u64 {
            return Err(registry_error(
                "refresh account profile",
                "portable account file must be regular and within its safety limit",
            ));
        }
        let mut contents = Vec::with_capacity(metadata.len() as usize);
        file.take(maximum.saturating_add(1) as u64)
            .read_to_end(&mut contents)
            .map_err(registry_io("read account refresh backup"))?;
        if contents.len() > maximum {
            return Err(registry_error(
                "refresh account profile",
                "portable account file exceeds its safety limit",
            ));
        }
        Ok(Some(contents))
    }

    pub(super) fn replace(&self, contents: Option<&[u8]>) -> Result<(), DaemonError> {
        if let Some(contents) = contents {
            let suffix: String = rand::thread_rng()
                .sample_iter(&Alphanumeric)
                .take(16)
                .map(char::from)
                .collect();
            let temporary =
                CString::new(format!(".account-refresh-{suffix}.tmp")).expect("ASCII filename");
            let mut file = open_at(
                &self.parent,
                &temporary,
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
            .map_err(registry_io("write account refresh"))?;
            let result = (|| {
                file.write_all(contents)
                    .map_err(registry_io("write account refresh"))?;
                file.sync_all()
                    .map_err(registry_io("sync account refresh"))?;
                // SAFETY: both names and the shared parent descriptor remain live.
                let result = unsafe {
                    libc::renameat(
                        self.parent.as_raw_fd(),
                        temporary.as_ptr(),
                        self.parent.as_raw_fd(),
                        self.name.as_ptr(),
                    )
                };
                if result < 0 {
                    return Err(registry_io("publish account refresh")(
                        std::io::Error::last_os_error(),
                    ));
                }
                Ok(())
            })();
            if result.is_err() {
                let _ = unlink_at(&self.parent, &temporary);
            }
            result?;
        } else {
            unlink_at(&self.parent, &self.name)
                .map_err(registry_io("remove revoked account file"))?;
        }
        self.parent
            .sync_all()
            .map_err(registry_io("sync account refresh directory"))
    }
}

fn open_at(parent: &fs::File, name: &CString, flags: i32) -> std::io::Result<fs::File> {
    // SAFETY: live fd and valid C string; the returned descriptor gets one owner.
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            flags,
            0o600 as libc::c_uint,
        )
    };
    if fd < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(unsafe { fs::File::from_raw_fd(fd) })
    }
}

fn unlink_at(parent: &fs::File, name: &CString) -> std::io::Result<()> {
    // SAFETY: live fd and valid C string. unlinkat does not follow the final name.
    if unsafe { libc::unlinkat(parent.as_raw_fd(), name.as_ptr(), 0) } < 0 {
        let error = std::io::Error::last_os_error();
        if error.kind() != std::io::ErrorKind::NotFound {
            return Err(error);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account_profile::materialization_tests::ProfileFixture;
    use std::os::unix::fs::{symlink, PermissionsExt};

    #[test]
    fn refresh_and_rollback_do_not_follow_a_replaced_parent_directory() {
        let fixture = ProfileFixture::new();
        let root = fixture.root.join("refresh");
        let outside = fixture.root.join("outside");
        fs::create_dir(&root).unwrap();
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("auth.json"), b"outside-marker").unwrap();
        let file = ReplicaFile::open(&root, Path::new("data/opencode/auth.json")).unwrap();
        file.replace(Some(b"old-auth")).unwrap();
        let previous = file.read(64).unwrap().unwrap();
        let original = root.join("original-data");
        fs::rename(root.join("data/opencode"), &original).unwrap();
        symlink(&outside, root.join("data/opencode")).unwrap();

        file.replace(Some(b"new-auth")).unwrap();
        assert_eq!(fs::read(original.join("auth.json")).unwrap(), b"new-auth");
        file.replace(Some(&previous)).unwrap();
        assert_eq!(fs::read(original.join("auth.json")).unwrap(), b"old-auth");
        assert_eq!(
            fs::metadata(original.join("auth.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        file.replace(None).unwrap();
        assert!(!original.join("auth.json").exists());
        assert_eq!(
            fs::read(outside.join("auth.json")).unwrap(),
            b"outside-marker"
        );
        assert!(ReplicaFile::open(&root, Path::new("data/opencode/auth.json")).is_err());
        assert!(fs::read_dir(&original).unwrap().next().is_none());
    }

    #[test]
    fn refresh_rejects_symlink_and_fifo_files_without_blocking() {
        let fixture = ProfileFixture::new();
        let root = fixture.root.join("refresh");
        fs::create_dir(&root).unwrap();
        let file = ReplicaFile::open(&root, Path::new("auth.json")).unwrap();
        let outside = fixture.root.join("outside-auth");
        fs::write(&outside, b"outside-marker").unwrap();
        symlink(&outside, root.join("auth.json")).unwrap();
        assert!(file.read(64).is_err());
        fs::remove_file(root.join("auth.json")).unwrap();
        // SAFETY: the parent fd and filename are valid for this disposable FIFO.
        assert_eq!(
            unsafe { libc::mkfifoat(file.parent.as_raw_fd(), file.name.as_ptr(), 0o600) },
            0
        );
        assert!(file.read(64).is_err());
        assert_eq!(fs::read(outside).unwrap(), b"outside-marker");
    }
}
