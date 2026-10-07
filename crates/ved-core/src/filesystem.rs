//! Descriptor-relative artifact access. Explicit roots are trusted; descendants are not.
//! NOFOLLOW prevents symlink substitution during traversal. A root owner concurrently
//! moving already-open directories outside the root remains outside this guarantee.
use crate::error::{Result, VedError};
#[cfg(unix)]
use std::path::Component;
use std::path::Path;

pub(crate) use crate::identifier::validate_identifier;

#[cfg(unix)]
mod platform {
    use super::*;
    use rustix::fd::OwnedFd;
    use rustix::fs::{self, AtFlags, FileType, Mode, OFlags};
    use std::ffi::{OsStr, OsString};
    use std::fs::File;
    use std::io::{Read, Write};
    use std::os::unix::ffi::OsStrExt;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    fn io(e: rustix::io::Errno) -> VedError {
        std::io::Error::from(e).into()
    }
    fn components(path: &Path) -> Result<Vec<&OsStr>> {
        path.components()
            .map(|c| match c {
                Component::Normal(s) if !s.as_bytes().contains(&0) => Ok(s),
                _ => Err(VedError::config(format!(
                    "Unsafe relative filesystem path '{}'",
                    path.display()
                ))),
            })
            .collect()
    }
    fn directory(fd: &OwnedFd, name: &OsStr, create: bool) -> Result<OwnedFd> {
        if create {
            match fs::mkdirat(fd, name, Mode::from_bits_truncate(0o755)) {
                Ok(()) | Err(rustix::io::Errno::EXIST) => (),
                Err(e) => return Err(io(e)),
            }
        }
        fs::openat(
            fd,
            name,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(io)
    }
    pub(crate) struct ConfinedDir {
        fd: OwnedFd,
    }
    impl ConfinedDir {
        pub(crate) fn open_existing(root: impl AsRef<Path>) -> Result<Self> {
            // Ancestors are explicitly selected authority (e.g. /var -> /private/var).
            // Resolve only the parent; atomically open the final root with NOFOLLOW.
            let supplied = root.as_ref();
            let absolute = if supplied.is_absolute() {
                supplied.to_path_buf()
            } else {
                std::env::current_dir()?.join(supplied)
            };
            let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
            let fd = if let Some(name) = absolute.file_name() {
                let parent = std::fs::canonicalize(absolute.parent().unwrap_or(Path::new("/")))?;
                let parent_fd = fs::open(parent, flags, Mode::empty()).map_err(io)?;
                fs::openat(parent_fd, name, flags, Mode::empty()).map_err(io)?
            } else {
                // A final root/parent component is intrinsic directory traversal,
                // never a symlink filename. Its ancestors are explicit authority.
                let selected = std::fs::canonicalize(&absolute)?;
                fs::open(selected, flags, Mode::empty()).map_err(io)?
            };
            Ok(Self { fd })
        }
        pub(crate) fn create(root: impl AsRef<Path>) -> Result<Self> {
            let root = root.as_ref();
            match std::fs::symlink_metadata(root) {
                Ok(m) if m.file_type().is_symlink() => {
                    return Err(VedError::config("Symlink filesystem root rejected"))
                }
                Ok(_) => return Self::open_existing(root),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                Err(e) => return Err(e.into()),
            }
            let absolute = if root.is_absolute() {
                root.to_path_buf()
            } else {
                std::env::current_dir()?.join(root)
            };
            let mut ancestor = absolute.as_path();
            let mut missing = Vec::new();
            loop {
                match std::fs::symlink_metadata(ancestor) {
                    Ok(_) => break,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        missing.push(
                            ancestor
                                .file_name()
                                .ok_or_else(|| VedError::config("Invalid output root"))?
                                .to_os_string(),
                        );
                        ancestor = ancestor
                            .parent()
                            .ok_or_else(|| VedError::config("Invalid output root"))?;
                    }
                    Err(e) => return Err(e.into()),
                }
            }
            let mut dir = Self::open_existing(ancestor)?;
            for name in missing.into_iter().rev() {
                dir.fd = directory(&dir.fd, &name, true)?;
            }
            Ok(dir)
        }
        fn walk(&self, parts: &[&OsStr], create: bool) -> Result<OwnedFd> {
            let mut fd = fs::openat(
                &self.fd,
                ".",
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(io)?;
            for part in parts {
                fd = directory(&fd, part, create)?;
            }
            Ok(fd)
        }
        pub(crate) fn preflight(&self, relative: impl AsRef<Path>) -> Result<()> {
            let parts = components(relative.as_ref())?;
            let mut fd = self.walk(&[], false)?;
            for (i, part) in parts.iter().enumerate() {
                let stat = match fs::statat(&fd, *part, AtFlags::SYMLINK_NOFOLLOW) {
                    Ok(s) => s,
                    Err(rustix::io::Errno::NOENT) => return Ok(()),
                    Err(e) => return Err(io(e)),
                };
                if FileType::from_raw_mode(stat.st_mode) == FileType::Symlink {
                    return Err(VedError::config("Symlink descendant rejected"));
                }
                if i + 1 < parts.len() {
                    fd = directory(&fd, part, false)?;
                }
            }
            Ok(())
        }
        pub(crate) fn preflight_tree(&self, relative: impl AsRef<Path>) -> Result<()> {
            self.preflight(relative.as_ref())?;
            if !self.exists(relative.as_ref())? {
                return Ok(());
            }
            let fd = self.walk(&components(relative.as_ref())?, false)?;
            let child = Self { fd };
            for name in child.read_dir(Path::new(""))? {
                let stat = fs::statat(&child.fd, &name, AtFlags::SYMLINK_NOFOLLOW).map_err(io)?;
                match FileType::from_raw_mode(stat.st_mode) {
                    FileType::Symlink => {
                        return Err(VedError::config("Symlink descendant rejected"))
                    }
                    FileType::Directory => child.preflight_tree(Path::new(&name))?,
                    _ => (),
                }
            }
            Ok(())
        }
        pub(crate) fn exists(&self, relative: impl AsRef<Path>) -> Result<bool> {
            self.preflight(relative.as_ref())?;
            let parts = components(relative.as_ref())?;
            if parts.is_empty() {
                return Ok(true);
            }
            let fd = match self.walk(&parts[..parts.len() - 1], false) {
                Ok(fd) => fd,
                Err(VedError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {
                    return Ok(false)
                }
                Err(e) => return Err(e),
            };
            match fs::statat(fd, parts[parts.len() - 1], AtFlags::SYMLINK_NOFOLLOW) {
                Ok(s) if FileType::from_raw_mode(s.st_mode) != FileType::Symlink => Ok(true),
                Ok(_) => Err(VedError::config("Symlink descendant rejected")),
                Err(rustix::io::Errno::NOENT) => Ok(false),
                Err(e) => Err(io(e)),
            }
        }
        pub(crate) fn create_dir_all(&self, relative: impl AsRef<Path>) -> Result<()> {
            self.walk(&components(relative.as_ref())?, true).map(|_| ())
        }
        pub(crate) fn read(&self, relative: impl AsRef<Path>) -> Result<Vec<u8>> {
            self.read_impl(relative.as_ref(), None)
        }
        pub(crate) fn read_bounded(
            &self,
            relative: impl AsRef<Path>,
            cap: usize,
        ) -> Result<Vec<u8>> {
            self.read_impl(relative.as_ref(), Some(cap))
        }
        fn read_impl(&self, relative: &Path, cap: Option<usize>) -> Result<Vec<u8>> {
            let parts = components(relative)?;
            let (name, dirs) = parts
                .split_last()
                .ok_or_else(|| VedError::config("Expected file path"))?;
            let parent = self.walk(dirs, false)?;
            let fd = fs::openat(
                parent,
                *name,
                OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(io)?;
            let mut file = File::from(fd);
            if !file.metadata()?.is_file() {
                return Err(VedError::config("Expected regular file"));
            }
            let mut bytes = Vec::new();
            match cap {
                Some(cap) => {
                    file.take((cap as u64).saturating_add(1))
                        .read_to_end(&mut bytes)?;
                }
                None => {
                    file.read_to_end(&mut bytes)?;
                }
            }
            Ok(bytes)
        }
        pub(crate) fn read_dir(&self, relative: impl AsRef<Path>) -> Result<Vec<OsString>> {
            let fd = self.walk(&components(relative.as_ref())?, false)?;
            let mut names = Vec::new();
            for entry in fs::Dir::new(fd).map_err(io)? {
                let entry = entry.map_err(io)?;
                let bytes = entry.file_name().to_bytes();
                if bytes != b"." && bytes != b".." {
                    names.push(OsStr::from_bytes(bytes).to_os_string());
                }
            }
            Ok(names)
        }
        pub(crate) fn file_names(&self, relative: impl AsRef<Path>) -> Result<Vec<OsString>> {
            let fd = self.walk(&components(relative.as_ref())?, false)?;
            let mut names = Vec::new();
            for name in self.read_dir(relative)? {
                let stat = fs::statat(&fd, &name, AtFlags::SYMLINK_NOFOLLOW).map_err(io)?;
                match FileType::from_raw_mode(stat.st_mode) {
                    FileType::Symlink => {
                        return Err(VedError::config("Symlink source config rejected"))
                    }
                    FileType::RegularFile => names.push(name),
                    _ => (),
                }
            }
            Ok(names)
        }
        pub(crate) fn write(&self, relative: impl AsRef<Path>, bytes: &[u8]) -> Result<()> {
            let parts = components(relative.as_ref())?;
            let (name, dirs) = parts
                .split_last()
                .ok_or_else(|| VedError::config("Expected file path"))?;
            let parent = self.walk(dirs, false)?;
            self.preflight(relative.as_ref())?;
            let temp = format!(
                ".ved-tmp-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            );
            let fd = fs::openat(
                &parent,
                temp.as_str(),
                OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::from_bits_truncate(0o644),
            )
            .map_err(io)?;
            let result = (|| {
                let mut file = File::from(fd);
                file.write_all(bytes)?;
                fs::renameat(&parent, temp.as_str(), &parent, *name).map_err(io)
            })();
            if result.is_err() {
                let _ = fs::unlinkat(&parent, temp.as_str(), AtFlags::empty());
            }
            result
        }
    }
}
#[cfg(unix)]
pub(crate) use platform::ConfinedDir;

#[cfg(not(unix))]
pub(crate) struct ConfinedDir;
#[cfg(not(unix))]
impl ConfinedDir {
    fn unsupported<T>() -> Result<T> {
        Err(VedError::config(
            "Confined filesystem access requires a Unix platform",
        ))
    }
    pub(crate) fn open_existing(_: impl AsRef<Path>) -> Result<Self> {
        Self::unsupported()
    }
    pub(crate) fn create(_: impl AsRef<Path>) -> Result<Self> {
        Self::unsupported()
    }
    pub(crate) fn preflight(&self, _: impl AsRef<Path>) -> Result<()> {
        Self::unsupported()
    }
    pub(crate) fn preflight_tree(&self, _: impl AsRef<Path>) -> Result<()> {
        Self::unsupported()
    }
    pub(crate) fn exists(&self, _: impl AsRef<Path>) -> Result<bool> {
        Self::unsupported()
    }
    pub(crate) fn create_dir_all(&self, _: impl AsRef<Path>) -> Result<()> {
        Self::unsupported()
    }
    pub(crate) fn write(&self, _: impl AsRef<Path>, _: &[u8]) -> Result<()> {
        Self::unsupported()
    }
    pub(crate) fn read(&self, _: impl AsRef<Path>) -> Result<Vec<u8>> {
        Self::unsupported()
    }
    pub(crate) fn read_bounded(&self, _: impl AsRef<Path>, _: usize) -> Result<Vec<u8>> {
        Self::unsupported()
    }
    pub(crate) fn file_names(&self, _: impl AsRef<Path>) -> Result<Vec<std::ffi::OsString>> {
        Self::unsupported()
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::symlink;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    struct Fixture(std::path::PathBuf);
    impl Fixture {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!(
                "ved-confined-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&p).unwrap();
            Self(p)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn identifiers_preserve_safe_names() {
        for good in ["Api", "my_service", "api.v2", "a-1"] {
            validate_identifier("workload", good).unwrap();
        }
        for bad in [
            "", ".", "..", "/a", "a/b", "a\\b", "a:b", "a b", "-a", "a\n",
        ] {
            assert!(validate_identifier("workload", bad).is_err(), "{bad}");
        }
    }
    #[test]
    fn confined_io_and_missing_preflight() {
        let f = Fixture::new();
        let root = f.0.join("new/root");
        let dir = ConfinedDir::create(&root).unwrap();
        dir.preflight("missing/file").unwrap();
        assert!(!root.join("missing").exists());
        dir.create_dir_all("config").unwrap();
        dir.write("config/a", b"hello").unwrap();
        assert_eq!(dir.read("config/a").unwrap(), b"hello");
        assert_eq!(
            dir.file_names("config").unwrap(),
            vec![std::ffi::OsString::from("a")]
        );
        for bad in ["../escape", "/escape", "config/../escape"] {
            assert!(dir.write(bad, b"x").is_err());
        }
    }
    #[test]
    fn bounded_read_returns_at_most_cap_plus_one() {
        let fixture = Fixture::new();
        let dir = ConfinedDir::open_existing(&fixture.0).unwrap();
        fs::write(fixture.0.join("source"), b"0123456789").unwrap();
        assert_eq!(dir.read_bounded("source", 3).unwrap(), b"0123");
        assert_eq!(dir.read_bounded("source", 10).unwrap(), b"0123456789");
        symlink(fixture.0.join("source"), fixture.0.join("link")).unwrap();
        assert!(dir.read_bounded("link", 3).is_err());
    }
    #[test]
    fn symlinks_and_dangling_links_rejected() {
        let f = Fixture::new();
        fs::create_dir(f.0.join("root")).unwrap();
        fs::create_dir(f.0.join("outside")).unwrap();
        let dir = ConfinedDir::open_existing(f.0.join("root")).unwrap();
        symlink(f.0.join("outside"), f.0.join("root/link")).unwrap();
        symlink(f.0.join("absent"), f.0.join("root/dangling")).unwrap();
        for path in ["link/file", "dangling"] {
            assert!(dir.preflight(path).is_err());
            assert!(dir.write(path, b"bad").is_err());
            assert!(dir.read(path).is_err());
        }
        assert!(dir.preflight_tree("").is_err());
        assert!(dir.file_names("").is_err());
        symlink(f.0.join("root"), f.0.join("alias")).unwrap();
        assert!(ConfinedDir::create(f.0.join("alias")).is_err());
        assert!(ConfinedDir::open_existing(f.0.join("alias")).is_err());
        assert!(!f.0.join("outside/file").exists());
    }
    #[test]
    fn roots_allow_explicit_parent_components_and_platform_ancestors() {
        let f = Fixture::new();
        fs::create_dir(f.0.join("real")).unwrap();
        fs::create_dir(f.0.join("real/root")).unwrap();
        symlink(f.0.join("real"), f.0.join("ancestor")).unwrap();
        let dir = ConfinedDir::open_existing(f.0.join("ancestor/root/.")).unwrap();
        dir.write("file", b"allowed").unwrap();
        assert_eq!(fs::read(f.0.join("real/root/file")).unwrap(), b"allowed");
        assert!(ConfinedDir::open_existing(f.0.join("real/root/../root")).is_ok());
        let created = ConfinedDir::create(f.0.join("real/root/../new-output")).unwrap();
        created.write("file", b"new").unwrap();
        assert_eq!(fs::read(f.0.join("real/new-output/file")).unwrap(), b"new");
        assert!(ConfinedDir::open_existing(f.0.join("real/root/..")).is_ok());
        assert!(created.write("../escape", b"bad").is_err());
        assert!(ConfinedDir::open_existing(".").is_ok());
        assert!(ConfinedDir::open_existing("/").is_ok());
    }
    #[test]
    fn writes_replace_hardlinks_without_truncating_external_file() {
        let f = Fixture::new();
        let external = f.0.join("external");
        fs::write(&external, b"original").unwrap();
        fs::create_dir(f.0.join("root")).unwrap();
        fs::hard_link(&external, f.0.join("root/file")).unwrap();
        let dir = ConfinedDir::open_existing(f.0.join("root")).unwrap();
        dir.write("file", b"new").unwrap();
        assert_eq!(fs::read(external).unwrap(), b"original");
        assert_eq!(dir.read("file").unwrap(), b"new");
    }
}
