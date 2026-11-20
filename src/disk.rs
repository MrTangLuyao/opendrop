//! Disk space numbers and preallocation.

use std::fs::File;
use std::io;
use std::path::Path;

use crate::settings::{Settings, StorageMode};
use crate::util::MIB;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiskInfo {
    pub total: u64,
    pub avail: u64,
    /// Inodes of the file system; 0 when it has no fixed number (btrfs).
    pub inodes: u64,
    pub inodes_avail: u64,
}

/// Size of the file system holding `path` (statvfs: blocks and blocks
/// available to unprivileged users, times the fragment size; inodes and
/// inodes available to unprivileged users).
#[cfg(unix)]
pub fn disk_info(path: &Path) -> io::Result<DiskInfo> {
    let s = rustix::fs::statvfs(path)?;
    Ok(DiskInfo {
        total: s.f_blocks.saturating_mul(s.f_frsize),
        avail: s.f_bavail.saturating_mul(s.f_frsize),
        inodes: s.f_files,
        inodes_avail: s.f_favail,
    })
}

/// Inodes always kept free, like [`reserve`] for bytes: every file costs
/// one, and a file system out of inodes fails every write on the host.
pub fn inode_reserve(inodes: u64) -> u64 {
    INODE_RESERVE_MIN.max(inodes / 100)
}

pub const INODE_RESERVE_MIN: u64 = 4096;

#[cfg(not(unix))]
pub fn disk_info(_path: &Path) -> io::Result<DiskInfo> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "statvfs needs unix",
    ))
}

/// Space always kept free on the disk, independent of the quota.
pub fn reserve(total: u64) -> u64 {
    (512 * MIB).max(total / 100)
}

/// Bytes opendrop may use in total.
pub fn quota(settings: &Settings, total: u64) -> u64 {
    match settings.storage_mode {
        StorageMode::Percent => {
            let q = u128::from(total) * u128::from(settings.storage_percent) / 100;
            u64::try_from(q).unwrap_or(u64::MAX)
        }
        StorageMode::Bytes => settings.storage_bytes,
    }
}

/// Reserves `len` bytes for `file` up front so a full disk fails before any
/// data is written. File systems without support are silently accepted.
#[cfg(target_os = "linux")]
pub fn preallocate(file: &File, len: u64) -> io::Result<()> {
    use rustix::io::Errno;
    if len == 0 {
        return Ok(());
    }
    match rustix::fs::fallocate(file, rustix::fs::FallocateFlags::empty(), 0, len) {
        Ok(()) => Ok(()),
        Err(e) if e == Errno::NOSPC || e == Errno::DQUOT => Err(e.into()),
        Err(_) => Ok(()),
    }
}

#[cfg(not(target_os = "linux"))]
pub fn preallocate(_file: &File, _len: u64) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::GIB;

    #[test]
    fn quota_and_reserve() {
        let mut s = Settings::defaults(0);
        s.storage_percent = 30;
        assert_eq!(quota(&s, 1000 * GIB), 300 * GIB);
        s.storage_mode = StorageMode::Bytes;
        s.storage_bytes = 7 * GIB;
        assert_eq!(quota(&s, 1000 * GIB), 7 * GIB);
        assert_eq!(reserve(10 * GIB), 512 * MIB);
        assert_eq!(reserve(1000 * GIB), 10 * GIB);
        assert_eq!(inode_reserve(0), INODE_RESERVE_MIN);
        assert_eq!(inode_reserve(1_500_000), 15_000);
        s.storage_mode = StorageMode::Percent;
        s.storage_percent = 95;
        assert_eq!(
            quota(&s, u64::MAX),
            u64::try_from(u128::from(u64::MAX) * 95 / 100).unwrap_or(0)
        );
    }

    #[cfg(unix)]
    #[test]
    fn statvfs_works() {
        let d = disk_info(Path::new("/")).expect("statvfs");
        assert!(d.total > 0);
        assert!(d.avail <= d.total);
    }
}
