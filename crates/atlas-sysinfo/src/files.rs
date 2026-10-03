//! Showing a file in the desktop's file manager, for Open File Location.
//!
//! The freedesktop `FileManager1` interface opens the folder with the file
//! selected (Dolphin, Nautilus and Nemo all answer it, starting if they
//! must). Without one, the caller opens the folder through the desktop's
//! URL handler instead, by [`file_uri`] of [`Path::parent`].
//!
//! Blocks for up to [`DEADLINE`]: run it on a thread of its own.

use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::time::Duration;

/// Long enough for a file manager to start for the call.
const DEADLINE: Duration = Duration::from_secs(10);

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
                .method_timeout(DEADLINE)
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
            Ok(Some(())) | Err(_) => Shown::Yes,
            Ok(None) => Shown::NoFileManager,
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
