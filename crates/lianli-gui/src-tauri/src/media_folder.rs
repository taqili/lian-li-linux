use crate::command_error::CommandError;
use lianli_shared::playlist::{PlaylistItemKind, MAX_ITEMS};
use lianli_shared::ui_message::UiMessage;
use serde::Serialize;
use std::path::{Path, PathBuf};

const MAX_DEPTH: usize = 8;
const MAX_SCANNED: usize = 20_000;

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct FolderMedia {
    pub items: Vec<String>,
    pub truncated: bool,
}

/// Images, GIFs and videos in `folder`, sorted by path. Hidden entries and
/// symlinked directories are skipped so a scan cannot loop or wander.
pub fn list(folder: &Path, recursive: bool) -> Result<FolderMedia, CommandError> {
    if !folder.is_absolute() {
        return Err(CommandError::ui(
            "Choose an absolute folder path",
            UiMessage::new("gui.mediaFolder.relativePath"),
        ));
    }
    let mut found = Vec::new();
    let mut scanned = 0;
    let mut truncated = false;
    let mut pending = vec![(folder.to_path_buf(), 0)];
    while let Some((directory, depth)) = pending.pop() {
        let entries = match std::fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) if directory == folder => {
                return Err(CommandError::ui(
                    format!("Cannot read folder: {error}"),
                    UiMessage::new("gui.mediaFolder.readFailed").param("error", error),
                ))
            }
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            scanned += 1;
            if scanned > MAX_SCANNED {
                truncated = true;
                break;
            }
            if entry.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            let path = entry.path();
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_dir() {
                if recursive && depth < MAX_DEPTH {
                    pending.push((path, depth + 1));
                }
            } else if PlaylistItemKind::from_path(&path).is_some()
                && path.is_file()
                && path.to_str().is_some()
            {
                found.push(path);
            }
        }
    }
    found.sort_by_cached_key(|path: &PathBuf| path.to_string_lossy().to_lowercase());
    if found.len() > MAX_ITEMS {
        found.truncate(MAX_ITEMS);
        truncated = true;
    }
    Ok(FolderMedia {
        items: found
            .into_iter()
            .filter_map(|path| path.to_str().map(str::to_owned))
            .collect(),
        truncated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_supported_media_sorted_and_skips_hidden_or_linked_folders() {
        let root = tempfile::tempdir().unwrap();
        let base = root.path();
        for name in ["b.PNG", "a.mp4", "notes.txt", ".hidden.gif", "c.gif"] {
            std::fs::write(base.join(name), b"x").unwrap();
        }
        std::fs::create_dir(base.join("Nested")).unwrap();
        std::fs::write(base.join("Nested/d.jpg"), b"x").unwrap();
        std::fs::create_dir(base.join(".cache")).unwrap();
        std::fs::write(base.join(".cache/e.png"), b"x").unwrap();
        std::os::unix::fs::symlink(base, base.join("loop")).unwrap();

        let name = |path: &String| {
            path.strip_prefix(&format!("{}/", base.display()))
                .unwrap()
                .to_owned()
        };
        let flat = list(base, false).unwrap();
        assert_eq!(
            flat.items.iter().map(name).collect::<Vec<_>>(),
            ["a.mp4", "b.PNG", "c.gif"]
        );
        assert!(!flat.truncated);
        let deep = list(base, true).unwrap();
        assert_eq!(
            deep.items.iter().map(name).collect::<Vec<_>>(),
            ["a.mp4", "b.PNG", "c.gif", "Nested/d.jpg"]
        );
        assert!(list(Path::new("relative"), false).is_err());
        assert!(list(&base.join("missing"), false).is_err());
    }

    #[test]
    fn large_folders_are_capped_at_the_playlist_limit() {
        let root = tempfile::tempdir().unwrap();
        for index in 0..MAX_ITEMS + 3 {
            std::fs::write(root.path().join(format!("{index:04}.png")), b"x").unwrap();
        }
        let result = list(root.path(), false).unwrap();
        assert_eq!(result.items.len(), MAX_ITEMS);
        assert!(result.truncated);
        assert!(result.items[0].ends_with("0000.png"));
    }
}
