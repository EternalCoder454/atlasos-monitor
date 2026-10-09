//! Showing a file in the desktop's file manager, for Open File Location.
//!
//! The freedesktop `FileManager1` interface opens the folder with the file
//! selected (Dolphin, Nautilus and Nemo all answer it, starting if they
//! must). Without one, the caller opens the folder through the desktop's
//! URL handler instead, by [`file_uri`] of [`Path::parent`].
//!
//! Blocks for up to [`DEADLINE`]: run it on a thread of its own.

use std::fs::OpenOptions;
use std::io::{self, Read};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::time::Duration;

/// Long enough for a file manager to start for the call.
const DEADLINE: Duration = Duration::from_secs(10);

/// A file that another program may have made (a desktop file, an icon
/// theme's `index.theme`, a container list), read whole: opened without
/// blocking, so a FIFO put there cannot hang the reader in `open`; it must be a
/// regular file (a device or a FIFO is refused, a link is followed to one, as
/// Flatpak and dotfile managers make them); and it must be at most `max`
/// bytes, so one cannot fill the memory. The file is checked once it is open, by
/// its descriptor, so what was checked is what is read.
pub fn read_capped(path: &Path, max: u64) -> io::Result<Vec<u8>> {
    let (bytes, cut) = read_prefix(path, max)?;
    if cut {
        return Err(io::Error::new(
            io::ErrorKind::FileTooLarge,
            "file too large",
        ));
    }
    Ok(bytes)
}

/// Like [`read_capped`], but a file over `max` bytes gives its first `max`
/// and `true` instead of an error: for a file whose start is what matters.
pub fn read_prefix(path: &Path, max: u64) -> io::Result<(Vec<u8>, bool)> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not a regular file",
        ));
    }
    let mut bytes = Vec::new();
    file.take(max.saturating_add(1)).read_to_end(&mut bytes)?;
    let cut = bytes.len() as u64 > max;
    bytes.truncate(usize::try_from(max).unwrap_or(usize::MAX));
    Ok((bytes, cut))
}

/// [`read_capped`] as text, non-UTF-8 bytes replaced; `None` if it cannot be read.
pub fn read_text_capped(path: &Path, max: u64) -> Option<String> {
    let bytes = read_capped(path, max).ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// What came of asking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shown {
    /// A file manager took it.
    Yes,
    /// There is no session bus, or nothing on it answers for `FileManager1`.
    NoFileManager,
}

/// Asks the session's file manager to show `path`, selected in its folder.
///
/// A call that runs out of time counts as taken: the file manager may be
/// slow to start and still open, and a fallback would open a second window.
pub fn show_in_file_manager(path: &Path) -> Shown {
    let Ok(rt) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        return Shown::NoFileManager;
    };
    let uri = file_uri(path);
    rt.block_on(async {
        let call = async {
            let conn = zbus::connection::Builder::session()
                .ok()?
                .build()
                .await
                .ok()?;
            conn.call_method(
                Some("org.freedesktop.FileManager1"),
                "/org/freedesktop/FileManager1",
                Some("org.freedesktop.FileManager1"),
                "ShowItems",
                &(vec![uri.as_str()], ""),
            )
            .await
            .ok()
            .map(|_| ())
        };
        match tokio::time::timeout(DEADLINE, call).await {
            Ok(Some(())) => Shown::Yes,
            Ok(None) => Shown::NoFileManager,
            Err(_) => {
                log::warn!("the file manager didn't answer in {DEADLINE:?}; taking it as shown");
                Shown::Yes
            }
        }
    })
}

/// A `file://` URI for an absolute path, every byte outside RFC 3986's
/// unreserved set and `/` percent-encoded. A path isn't always UTF-8, and
/// a URI carries its bytes either way.
pub fn file_uri(path: &Path) -> String {
    let mut uri = String::from("file://");
    for &b in path.as_os_str().as_bytes() {
        if b.is_ascii_alphanumeric() || b"-._~/".contains(&b) {
            uri.push(char::from(b));
        } else {
            uri.push_str(&format!("%{b:02X}"));
        }
    }
    uri
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;

    #[test]
    fn hostile_files_are_not_read() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        std::fs::write(d.join("ok"), "hello").unwrap();
        assert_eq!(read_capped(&d.join("ok"), 5).unwrap(), b"hello");
        // One byte over the cap.
        let e = read_capped(&d.join("ok"), 4).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::FileTooLarge);
        // A link to a regular file is followed; one to a device is refused.
        std::os::unix::fs::symlink(d.join("ok"), d.join("link")).unwrap();
        assert_eq!(read_capped(&d.join("link"), 64).unwrap(), b"hello");
        std::os::unix::fs::symlink("/dev/zero", d.join("zero")).unwrap();
        assert!(read_capped(&d.join("zero"), 64).is_err());
        // A directory, and a file that is not there.
        assert!(read_capped(d, 64).is_err());
        assert!(read_capped(&d.join("absent"), 64).is_err());
        // A FIFO with no writer: opening it must not wait.
        let fifo = d.join("fifo");
        let c = std::ffi::CString::new(fifo.as_os_str().as_bytes()).unwrap();
        // SAFETY: a valid NUL-terminated path.
        assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
        let started = std::time::Instant::now();
        assert!(read_capped(&fifo, 64).is_err());
        assert!(started.elapsed() < Duration::from_secs(5));
        assert_eq!(read_text_capped(&fifo, 64), None);
        assert_eq!(
            read_text_capped(&d.join("ok"), 64).as_deref(),
            Some("hello")
        );
    }

    #[test]
    fn uris_encode_what_a_path_can_hold() {
        assert_eq!(file_uri(Path::new("/usr/bin/bash")), "file:///usr/bin/bash");
        assert_eq!(
            file_uri(Path::new("/opt/My App/a#b?c%d")),
            "file:///opt/My%20App/a%23b%3Fc%25d"
        );
        assert_eq!(file_uri(Path::new("/home/zoë")), "file:///home/zo%C3%AB");
        assert_eq!(
            file_uri(Path::new(OsStr::from_bytes(b"/tmp/\xff"))),
            "file:///tmp/%FF"
        );
    }
}

#[cfg(test)]
mod props {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #![proptest_config(crate::hostile::cases(150))]

        /// A file is read whole if it is within the cap, and refused
        /// otherwise; with a prefix, the first bytes and whether it was cut.
        #[test]
        fn the_cap_is_exact(len in 0usize..300, max in 0u64..300) {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("f");
            std::fs::write(&path, vec![b'x'; len]).unwrap();
            let whole = read_capped(&path, max);
            prop_assert_eq!(whole.is_ok(), len as u64 <= max);
            let (prefix, cut) = read_prefix(&path, max).unwrap();
            prop_assert_eq!(cut, len as u64 > max);
            prop_assert_eq!(prefix.len() as u64, (len as u64).min(max));
        }
    }
}
