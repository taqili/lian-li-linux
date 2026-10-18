use crate::command_error::CommandError;
use lianli_shared::playlist::{scan_folder, MAX_ITEMS};
use lianli_shared::ui_message::UiMessage;
use serde::Serialize;
use std::path::Path;

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct FolderMedia {
    pub items: Vec<String>,
    pub truncated: bool,
}

/// Images, GIFs and videos in `folder`, sorted by path, as the daemon will
/// see them when it rescans a live folder.
pub fn list(folder: &Path, recursive: bool) -> Result<FolderMedia, CommandError> {
    if !folder.is_absolute() {
        return Err(CommandError::ui(
            "Choose an absolute folder path",
            UiMessage::new("gui.mediaFolder.relativePath"),
        ));
    }
    let (found, truncated) = scan_folder(folder, recursive, MAX_ITEMS).map_err(|error| {
        CommandError::ui(
            format!("Cannot read folder: {error}"),
            UiMessage::new("gui.mediaFolder.readFailed").param("error", error),
        )
    })?;
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
    fn listing_reports_absolute_utf8_paths_and_rejects_relative_folders() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("b.png"), b"x").unwrap();
        std::fs::write(root.path().join("a.mp4"), b"x").unwrap();
        let listed = list(root.path(), false).unwrap();
        assert_eq!(
            listed.items,
            [
                root.path().join("a.mp4").to_str().unwrap(),
                root.path().join("b.png").to_str().unwrap()
            ]
        );
        assert!(!listed.truncated);
        assert!(list(Path::new("relative"), false).is_err());
        assert!(list(&root.path().join("missing"), false).is_err());
    }
}
