//! Reads that stay inside an opened directory.
//!
//! The library folder is opened first. The pack root is then opened from that
//! directory fd with `O_NOFOLLOW | O_DIRECTORY`, or on Linux `openat2` with
//! `RESOLVE_NO_SYMLINKS` and `RESOLVE_BENEATH`. A symlink in the pack root's
//! place is refused. Each relative component is opened from the pack fd, so
//! replacing a directory inside the pack with a symlink cannot change which
//! file is read. On Linux, `openat2` does that inner walk in one call when the
//! kernel supports it. Otherwise, and on macOS, each directory component is
//! `openat` with `O_NOFOLLOW | O_DIRECTORY | O_CLOEXEC`, and the final file is
//! `openat` with `O_NOFOLLOW`. The same `O_NOFOLLOW` open is the fallback when
//! `openat2` returns `ENOSYS`. Bytes come only from the final fd.

use std::io::{Error, ErrorKind, Read};
use std::path::Path;

use rustix::fd::OwnedFd;
use rustix::fs::{AtFlags, FileType, Mode, OFlags, fstat, openat, statat};

/// A relative read that did not return bytes.
#[derive(Debug)]
pub(crate) enum WalkFail {
    /// A component was a symlink.
    Symlink,
    /// The file has another directory entry (`st_nlink > 1`).
    Hardlink,
    /// The relative path is absolute, empty, or contains `..`.
    Escape,
    /// Open, stat, or read failed.
    Io(Error),
}

impl std::fmt::Display for WalkFail {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Symlink => f.write_str("refusing to follow a symlink"),
            Self::Hardlink => f.write_str("file has more than one link"),
            Self::Escape => f.write_str("path escapes the root"),
            Self::Io(err) => write!(f, "{err}"),
        }
    }
}

/// UTF-8 text of `rel` under `root`, without following a symlink.
///
/// # Errors
///
/// Returns [`WalkFail`] when a component is a symlink, the file has more than
/// one link, the relative path leaves the root, or the file cannot be read
/// as UTF-8.
pub(crate) fn read_text_under(root: &Path, rel: &str) -> Result<String, WalkFail> {
    to_text(read_bytes(root, rel, true)?)
}

fn to_text(bytes: Vec<u8>) -> Result<String, WalkFail> {
    String::from_utf8(bytes).map_err(|err| WalkFail::Io(Error::new(ErrorKind::InvalidData, err)))
}

fn read_bytes(root: &Path, rel: &str, try_at2: bool) -> Result<Vec<u8>, WalkFail> {
    let segments = segments(rel)?;
    let root_fd = open_root(root)?;
    let opened = if try_at2 {
        open_relative(&root_fd, &segments)?
    } else {
        open_walk(&root_fd, &segments)?
    };
    read_fd(regular_file(opened)?)
}

fn segments(rel: &str) -> Result<Vec<&str>, WalkFail> {
    if rel.starts_with('/') || rel.starts_with('\\') {
        return Err(WalkFail::Escape);
    }
    let mut out = Vec::new();
    for segment in rel.split(['/', '\\']) {
        if segment.is_empty() || segment == "." || segment == ".." {
            return Err(WalkFail::Escape);
        }
        out.push(segment);
    }
    if out.is_empty() {
        return Err(WalkFail::Escape);
    }
    Ok(out)
}

fn open_root(root: &Path) -> Result<OwnedFd, WalkFail> {
    let name = root
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(WalkFail::Escape)?;
    let parent = open_parent(root)?;
    open_pack_root(&parent, name)
}

fn open_parent(root: &Path) -> Result<OwnedFd, WalkFail> {
    let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC;
    let parent = root.parent().filter(|path| !path.as_os_str().is_empty());
    let path = parent.unwrap_or(Path::new("."));
    openat(rustix::fs::CWD, path, flags, Mode::empty())
        .map_err(|err| WalkFail::Io(Error::from(err)))
}

fn open_pack_root(parent: &OwnedFd, name: &str) -> Result<OwnedFd, WalkFail> {
    if let Some(opened) = open_pack_at2(parent, name) {
        return opened;
    }
    open_dir(parent, name)
}

#[cfg(target_os = "linux")]
fn open_pack_at2(parent: &OwnedFd, name: &str) -> Option<Result<OwnedFd, WalkFail>> {
    match rustix::fs::openat2(
        parent,
        name,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
        rustix::fs::ResolveFlags::NO_SYMLINKS | rustix::fs::ResolveFlags::BENEATH,
    ) {
        Ok(fd) => Some(Ok(fd)),
        Err(err) if err == rustix::io::Errno::NOSYS || err == rustix::io::Errno::NOTSUP => None,
        Err(err) => Some(Err(refuse_symlink(parent, name, err))),
    }
}

#[cfg(not(target_os = "linux"))]
fn open_pack_at2(_parent: &OwnedFd, _name: &str) -> Option<Result<OwnedFd, WalkFail>> {
    None
}

fn refuse_symlink(dir: &OwnedFd, name: &str, err: rustix::io::Errno) -> WalkFail {
    if component_is_symlink(dir, name) {
        WalkFail::Symlink
    } else {
        classify(err)
    }
}

fn open_relative(root: &OwnedFd, segments: &[&str]) -> Result<OwnedFd, WalkFail> {
    if let Some(opened) = open_at2(root, segments) {
        return opened;
    }
    open_walk(root, segments)
}

#[cfg(target_os = "linux")]
fn open_at2(root: &OwnedFd, segments: &[&str]) -> Option<Result<OwnedFd, WalkFail>> {
    let rel = segments.join("/");
    match rustix::fs::openat2(
        root,
        rel.as_str(),
        OFlags::RDONLY | OFlags::CLOEXEC,
        Mode::empty(),
        rustix::fs::ResolveFlags::NO_SYMLINKS | rustix::fs::ResolveFlags::BENEATH,
    ) {
        Ok(fd) => Some(Ok(fd)),
        Err(err) if err == rustix::io::Errno::NOSYS || err == rustix::io::Errno::NOTSUP => None,
        Err(err) => Some(Err(classify(err))),
    }
}

#[cfg(not(target_os = "linux"))]
fn open_at2(_root: &OwnedFd, _segments: &[&str]) -> Option<Result<OwnedFd, WalkFail>> {
    None
}

fn open_walk(root: &OwnedFd, segments: &[&str]) -> Result<OwnedFd, WalkFail> {
    let Some((last, dirs)) = segments.split_last() else {
        return Err(WalkFail::Escape);
    };
    let mut held = None;
    for name in dirs {
        let next = open_dir(parent(held.as_ref(), root), name)?;
        held = Some(next);
    }
    open_file(parent(held.as_ref(), root), last)
}

fn parent<'a>(held: Option<&'a OwnedFd>, root: &'a OwnedFd) -> &'a OwnedFd {
    match held {
        Some(fd) => fd,
        None => root,
    }
}

fn open_dir(dir: &OwnedFd, name: &str) -> Result<OwnedFd, WalkFail> {
    match openat(
        dir,
        name,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::empty(),
    ) {
        Ok(fd) => Ok(fd),
        Err(err) => Err(refuse_symlink(dir, name, err)),
    }
}

fn component_is_symlink(dir: &OwnedFd, name: &str) -> bool {
    statat(dir, name, AtFlags::SYMLINK_NOFOLLOW)
        .is_ok_and(|stat| FileType::from_raw_mode(stat.st_mode).is_symlink())
}

fn open_file(dir: &OwnedFd, name: &str) -> Result<OwnedFd, WalkFail> {
    openat(
        dir,
        name,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::empty(),
    )
    .map_err(classify)
}

fn classify(err: rustix::io::Errno) -> WalkFail {
    if err == rustix::io::Errno::LOOP {
        WalkFail::Symlink
    } else if err == rustix::io::Errno::XDEV {
        WalkFail::Escape
    } else {
        WalkFail::Io(Error::from(err))
    }
}

fn regular_file(fd: OwnedFd) -> Result<OwnedFd, WalkFail> {
    let stat = fstat(&fd).map_err(|err| WalkFail::Io(Error::from(err)))?;
    let kind = FileType::from_raw_mode(stat.st_mode);
    if kind.is_symlink() {
        return Err(WalkFail::Symlink);
    }
    if !kind.is_file() {
        return Err(WalkFail::Io(Error::new(
            ErrorKind::InvalidInput,
            "not a regular file",
        )));
    }
    if stat.st_nlink > 1 {
        return Err(WalkFail::Hardlink);
    }
    Ok(fd)
}

fn read_fd(fd: OwnedFd) -> Result<Vec<u8>, WalkFail> {
    let mut file = std::fs::File::from(fd);
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).map_err(WalkFail::Io)?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::WalkFail;

    fn temp() -> PathBuf {
        static N: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "rig-nofollow-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).expect("temp");
        path
    }

    #[test]
    fn component_walk_refuses_a_symlink_and_reads_the_real_file() {
        let root = temp();
        let outside = root.join("outside");
        std::fs::create_dir_all(root.join("sub")).expect("sub");
        std::fs::create_dir_all(&outside).expect("outside");
        std::fs::write(root.join("sub").join("notes.md"), "inside").expect("inside");
        std::fs::write(outside.join("notes.md"), "OUTSIDE-SECRET-BYTES").expect("secret");
        std::os::unix::fs::symlink(&outside, root.join("linked")).expect("link");
        std::fs::hard_link(outside.join("notes.md"), root.join("hard.md")).expect("hard");
        let nested = super::read_bytes(&root, "sub/notes.md", false).expect("walk");
        assert_eq!(nested, b"inside");
        let via_at2 = super::read_text_under(&root, "sub/notes.md").expect("at2");
        assert_eq!(via_at2, "inside");
        let linked = super::read_bytes(&root, "linked/notes.md", false).expect_err("symlink");
        assert!(matches!(linked, WalkFail::Symlink), "{linked:?}");
        let linked_at2 = super::read_text_under(&root, "linked/notes.md").expect_err("at2 symlink");
        assert!(matches!(linked_at2, WalkFail::Symlink), "{linked_at2:?}");
        let hard = super::read_text_under(&root, "hard.md").expect_err("hardlink");
        assert!(matches!(hard, WalkFail::Hardlink), "{hard:?}");
        drop(std::fs::remove_dir_all(root));
    }

    #[test]
    fn pack_root_symlink_is_refused_on_both_walks() {
        let library = temp();
        let pack = library.join("pack");
        std::fs::create_dir_all(&pack).expect("pack");
        std::fs::write(pack.join("notes.md"), "inside").expect("inside");
        let outside = library.join("outside");
        std::fs::create_dir_all(&outside).expect("outside");
        std::fs::write(outside.join("notes.md"), "OUTSIDE-SECRET-BYTES").expect("secret");
        let parked = library.join("parked");
        std::fs::rename(&pack, &parked).expect("mv");
        std::os::unix::fs::symlink(&outside, &pack).expect("link");
        for try_at2 in [true, false] {
            let err = super::read_bytes(&pack, "notes.md", try_at2).expect_err("root symlink");
            assert!(
                matches!(err, WalkFail::Symlink),
                "try_at2={try_at2} {err:?}"
            );
            assert!(!err.to_string().contains("OUTSIDE-SECRET-BYTES"));
        }
        let real = super::read_bytes(&parked, "notes.md", false).expect("real");
        assert_eq!(real, b"inside");
        drop(std::fs::remove_dir_all(library));
    }
}
