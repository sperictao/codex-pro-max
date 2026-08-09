use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(1);

pub(crate) trait AtomicFileWriter: Send + Sync {
    fn replace(&self, target: &Path, bytes: &[u8]) -> Result<(), String>;
}

#[derive(Debug, Default)]
pub(crate) struct PlatformAtomicFileWriter;

impl AtomicFileWriter for PlatformAtomicFileWriter {
    fn replace(&self, target: &Path, bytes: &[u8]) -> Result<(), String> {
        let parent = target
            .parent()
            .ok_or_else(|| "atomic target has no parent directory".to_string())?;
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create atomic target directory: {error}"))?;

        let target_permissions = match std::fs::metadata(target) {
            Ok(metadata) => {
                let permissions = metadata.permissions();
                if permissions.readonly() {
                    return Err("atomic target is read-only".to_string());
                }
                Some(permissions)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(format!("failed to inspect atomic target: {error}")),
        };

        let temp = temp_path(target)?;
        let result = (|| {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp)
                .map_err(|error| format!("failed to create atomic temp file: {error}"))?;
            file.write_all(bytes)
                .map_err(|error| format!("failed to write atomic temp file: {error}"))?;
            file.flush()
                .map_err(|error| format!("failed to flush atomic temp file: {error}"))?;
            if let Some(permissions) = target_permissions {
                file.set_permissions(permissions).map_err(|error| {
                    format!("failed to preserve atomic target permissions: {error}")
                })?;
            }
            replace_file(&temp, target)
        })();

        if result.is_err() {
            let _ = std::fs::remove_file(&temp);
        }
        result
    }
}

fn temp_path(target: &Path) -> Result<PathBuf, String> {
    let parent = target
        .parent()
        .ok_or_else(|| "atomic target has no parent directory".to_string())?;
    let name = target
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "atomic target filename is not valid UTF-8".to_string())?;
    let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
    Ok(parent.join(format!(".{name}.dashi-{}-{id}.tmp", std::process::id())))
}

#[cfg(unix)]
fn replace_file(temp: &Path, target: &Path) -> Result<(), String> {
    std::fs::rename(temp, target)
        .map_err(|error| format!("failed to replace atomic target: {error}"))
}

#[cfg(windows)]
fn replace_file(temp: &Path, target: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    let temp_wide: Vec<u16> = temp.as_os_str().encode_wide().chain(Some(0)).collect();
    let target_wide: Vec<u16> = target.as_os_str().encode_wide().chain(Some(0)).collect();
    unsafe {
        MoveFileExW(
            PCWSTR(temp_wide.as_ptr()),
            PCWSTR(target_wide.as_ptr()),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    }
    .map_err(|error| format!("failed to replace atomic target: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_writer_refuses_to_replace_readonly_target() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("config.json");
        std::fs::write(&target, b"old").unwrap();
        let original_permissions = std::fs::metadata(&target).unwrap().permissions();
        let mut readonly_permissions = original_permissions.clone();
        readonly_permissions.set_readonly(true);
        std::fs::set_permissions(&target, readonly_permissions).unwrap();

        let result = PlatformAtomicFileWriter.replace(&target, b"new");

        assert_eq!(result.unwrap_err(), "atomic target is read-only");
        assert_eq!(std::fs::read(&target).unwrap(), b"old");
        std::fs::set_permissions(&target, original_permissions).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn platform_writer_preserves_existing_unix_mode() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("config.json");
        std::fs::write(&target, b"old").unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600)).unwrap();

        PlatformAtomicFileWriter.replace(&target, b"new").unwrap();

        assert_eq!(std::fs::read(&target).unwrap(), b"new");
        assert_eq!(
            std::fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
