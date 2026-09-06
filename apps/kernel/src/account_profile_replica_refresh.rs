use super::*;

#[cfg(unix)]
#[path = "account_profile_replica_file_unix.rs"]
mod replica_file;
#[cfg(unix)]
use replica_file::ReplicaFile;

/// Refresh account-owned files without replacing directories used by a running
/// provider. Backups are bounded portable files, never the provider database.
pub(super) struct ReplicaFileRefresh {
    previous: Vec<(ReplicaFile, Option<Vec<u8>>)>,
    applied: Vec<usize>,
}

impl ReplicaFileRefresh {
    pub(super) fn publish(
        root: &Path,
        staging_root: &Path,
        files: &[(PathBuf, Vec<u8>)],
    ) -> Result<Self, DaemonError> {
        let mut refresh = Self {
            previous: Vec::new(),
            applied: Vec::new(),
        };
        let mut desired: BTreeMap<PathBuf, Option<&[u8]>> =
            BTreeMap::from([(PathBuf::from("data/opencode/auth.json"), None)]);
        for name in OPENCODE_CONFIG_FILES {
            desired.insert(Path::new("config/opencode").join(name), None);
        }
        for (staged, contents) in files {
            let relative = staged
                .strip_prefix(staging_root)
                .map_err(|error| registry_error("refresh account profile", error.to_string()))?;
            let value = desired.get_mut(relative).ok_or_else(|| {
                registry_error(
                    "refresh account profile",
                    "OpenCode account refresh accepts only portable account files",
                )
            })?;
            *value = Some(contents);
        }
        let mut remaining = MAX_MATERIALIZATION_BYTES;
        // Validate and snapshot every destination before changing credentials.
        for relative in desired.keys() {
            let destination = ReplicaFile::open(root, relative)?;
            let previous = destination.read(remaining)?;
            remaining = remaining.saturating_sub(previous.as_ref().map_or(0, Vec::len));
            refresh.previous.push((destination, previous));
        }
        for (index, contents) in desired.values().enumerate() {
            if refresh.previous[index].1.as_deref() == *contents {
                continue;
            }
            // An atomic write can fail at directory sync after publishing.
            refresh.applied.push(index);
            if let Err(error) = refresh.previous[index].0.replace(*contents) {
                refresh.rollback().map_err(|rollback| {
                    registry_error(
                        "refresh account profile",
                        format!("{error}; credential rollback failed: {rollback}"),
                    )
                })?;
                return Err(error);
            }
        }
        Ok(refresh)
    }

    pub(super) fn rollback(&mut self) -> Result<(), DaemonError> {
        while let Some(index) = self.applied.last().copied() {
            let (path, previous) = &self.previous[index];
            path.replace(previous.as_deref())?;
            self.applied.pop();
        }
        Ok(())
    }

    pub(super) fn commit(&mut self) {
        self.applied.clear();
    }
}

#[cfg(not(unix))]
struct ReplicaFile(PathBuf);

#[cfg(not(unix))]
impl ReplicaFile {
    fn open(root: &Path, relative: &Path) -> Result<Self, DaemonError> {
        let destination = root.join(relative);
        ensure_regular_parents(root, &destination)?;
        Ok(Self(destination))
    }

    fn read(&self, maximum: usize) -> Result<Option<Vec<u8>>, DaemonError> {
        read_bounded_regular_file_no_follow(&self.0, maximum, "portable account file")
    }

    fn replace(&self, contents: Option<&[u8]>) -> Result<(), DaemonError> {
        write_or_remove(&self.0, contents)
    }
}

#[cfg(not(unix))]
fn write_or_remove(path: &Path, contents: Option<&[u8]>) -> Result<(), DaemonError> {
    if let Some(contents) = contents {
        return atomic_write_private(path, contents);
    }
    match fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(registry_io("remove revoked account file")(error)),
    }
    sync_directory(path.parent().expect("validated file parent"))
}

impl Drop for ReplicaFileRefresh {
    fn drop(&mut self) {
        // Explicit error paths report rollback failures; this also protects
        // early returns while the account registry is being committed.
        let _ = self.rollback();
    }
}

#[cfg(not(unix))]
fn ensure_regular_parents(root: &Path, destination: &Path) -> Result<(), DaemonError> {
    let parent = destination.parent().expect("validated file parent");
    let relative = parent
        .strip_prefix(root)
        .map_err(|error| registry_error("refresh account profile", error.to_string()))?;
    let mut current = root.to_path_buf();
    for component in std::iter::once(None).chain(relative.components().map(Some)) {
        if let Some(component) = component {
            current.push(component.as_os_str());
        }
        let metadata = match fs::symlink_metadata(&current) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(&current).map_err(registry_io("prepare account refresh"))?;
                set_private_dir_permissions(&current)?;
                fs::symlink_metadata(&current).map_err(registry_io("prepare account refresh"))?
            }
            Err(error) => return Err(registry_io("prepare account refresh")(error)),
        };
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(registry_error(
                "refresh account profile",
                "provider account refresh parents must be regular directories",
            ));
        }
    }
    Ok(())
}
