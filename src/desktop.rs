//! Linux: the app's own menu entry and icon in `~/.local/share`. Desktops on
//! Wayland (KDE, GNOME) take a window's icon from the `.desktop` file that
//! matches its app id, not from the window, so without these files the window
//! has no icon.

use std::io;
use std::path::{Path, PathBuf};

const ID: &str = "tidal-playlist-editor";
/// Marks a `.desktop` file as written by the app. One without it was made by
/// hand or by a package, and is left alone.
const MARK: &str = "X-TidalPlaylistEditor-Managed=true";
const TEMPLATE: &str = include_str!("../packaging/tidal-playlist-editor.desktop");
const ICON: &[u8] = include_bytes!("../packaging/icon.png");

fn entry_file(share: &Path) -> PathBuf {
    share.join("applications").join(format!("{ID}.desktop"))
}

fn icon_file(share: &Path) -> PathBuf {
    share.join("icons/hicolor/256x256/apps").join(format!("{ID}.png"))
}

/// The program the menu should start: the AppImage itself when run from one
/// (the running binary is then inside a temporary mount), else this binary.
pub fn program() -> Option<PathBuf> {
    std::env::var_os("APPIMAGE").map(PathBuf::from).or_else(|| std::env::current_exe().ok())
}

/// `path` as one argument of an `Exec` line: quoted, with the characters the
/// Desktop Entry spec reserves escaped, then escaped again as a string value.
fn exec_arg(path: &str) -> String {
    let mut quoted = String::from("\"");
    for c in path.chars() {
        match c {
            '"' | '`' | '$' | '\\' => {
                quoted.push('\\');
                quoted.push(c);
            }
            '%' => quoted.push_str("%%"),
            c => quoted.push(c),
        }
    }
    quoted.push('"');
    string_value(&quoted)
}

/// `s` escaped as a Desktop Entry string value: backslashes, and line breaks
/// and tabs, which would otherwise end the line or start a new key.
fn string_value(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out
}

/// The `.desktop` file for a program at `exe`.
pub fn entry_text(exe: &str) -> String {
    let mut out = String::new();
    for line in TEMPLATE.lines() {
        if line.starts_with("Exec=") {
            out.push_str(&format!("Exec={}\nTryExec={}\n", exec_arg(exe), string_value(exe)));
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    out.push_str(MARK);
    out.push('\n');
    out
}

fn ours(path: &Path) -> bool {
    std::fs::read_to_string(path).is_ok_and(|t| t.lines().any(|l| l == MARK))
}

fn write_if_changed(path: &Path, bytes: &[u8]) -> io::Result<bool> {
    if std::fs::read(path).is_ok_and(|old| old == bytes) {
        return Ok(false);
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, bytes)?;
    Ok(true)
}

/// Writes the menu entry for `exe` and the icon under `share` when missing or
/// out of date (the program moved). Returns true when something was written.
pub fn install(share: &Path, exe: &Path) -> io::Result<bool> {
    let entry = entry_file(share);
    if entry.exists() && !ours(&entry) {
        return Ok(false);
    }
    let Some(exe) = exe.to_str() else {
        return Err(io::Error::other("program path is not UTF-8"));
    };
    let icon = write_if_changed(&icon_file(share), ICON)?;
    Ok(write_if_changed(&entry, entry_text(exe).as_bytes())? || icon)
}

/// Removes the menu entry and icon, if the app wrote them.
pub fn remove(share: &Path) -> io::Result<()> {
    let entry = entry_file(share);
    if !ours(&entry) {
        return Ok(());
    }
    std::fs::remove_file(entry)?;
    match std::fs::remove_file(icon_file(share)) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exec_line_quotes_and_escapes() {
        assert_eq!(exec_arg("/opt/app/editor"), "\"/opt/app/editor\"");
        assert_eq!(exec_arg("/home/a b/x"), "\"/home/a b/x\"");
        // `$` gets a backslash inside the quotes; that backslash is itself escaped.
        assert_eq!(exec_arg("/p/$x%"), "\"/p/\\\\$x%%\"");
        let text = entry_text("/home/me/Apps/Editor.AppImage");
        assert!(text.contains("\nExec=\"/home/me/Apps/Editor.AppImage\"\n"), "{text}");
        assert!(text.contains("\nTryExec=/home/me/Apps/Editor.AppImage\n"));
        assert!(text.contains("\nIcon=tidal-playlist-editor\n"));
        assert!(text.ends_with(&format!("{MARK}\n")));
        // A line break in the path cannot start a new key; a backslash survives TryExec.
        let text = entry_text("/a\nIcon=evil\\b");
        assert!(text.contains("\nExec=\"/a\\nIcon=evil\\\\\\\\b\"\n"), "{text}");
        assert!(text.contains("\nTryExec=/a\\nIcon=evil\\\\b\n"), "{text}");
        assert!(!text.contains("\nIcon=evil"));
    }

    #[test]
    fn installs_updates_and_removes_only_its_own_files() {
        let dir = tempfile::tempdir().unwrap();
        let share = dir.path();
        assert!(install(share, Path::new("/a/editor")).unwrap());
        assert!(!install(share, Path::new("/a/editor")).unwrap(), "nothing to do the second time");
        assert!(icon_file(share).exists());
        assert!(install(share, Path::new("/b/editor")).unwrap(), "follows the program when it moves");
        assert!(std::fs::read_to_string(entry_file(share)).unwrap().contains("Exec=\"/b/editor\""));
        remove(share).unwrap();
        assert!(!entry_file(share).exists() && !icon_file(share).exists());
        // A hand-made entry is neither replaced nor removed.
        std::fs::write(entry_file(share), "[Desktop Entry]\nExec=mine\n").unwrap();
        assert!(!install(share, Path::new("/a/editor")).unwrap());
        remove(share).unwrap();
        assert!(entry_file(share).exists());
    }
}
