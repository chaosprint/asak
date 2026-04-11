use std::{fs, path::Path};

use crate::tui::{BrowserEntry, BrowserEntryKind, AUDIO_EXTENSIONS};

pub(crate) fn load_browser_entries(root: &Path) -> Vec<BrowserEntry> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };

    let mut directories = Vec::new();
    let mut audio_files = Vec::new();

    for path in entries.flatten().map(|entry| entry.path()) {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_string();

        if path.is_dir() {
            directories.push(BrowserEntry {
                name,
                path,
                kind: BrowserEntryKind::Directory,
            });
        } else if path.is_file() && is_supported_audio_file(&path) {
            audio_files.push(BrowserEntry {
                name,
                path,
                kind: BrowserEntryKind::AudioFile,
            });
        }
    }

    directories.sort_by(|left, right| left.name.to_lowercase().cmp(&right.name.to_lowercase()));
    audio_files.sort_by(|left, right| left.name.to_lowercase().cmp(&right.name.to_lowercase()));

    let mut all_entries = Vec::new();
    if let Some(parent) = root.parent() {
        all_entries.push(BrowserEntry {
            name: "..".to_string(),
            path: parent.to_path_buf(),
            kind: BrowserEntryKind::Parent,
        });
    }
    all_entries.extend(directories);
    all_entries.extend(audio_files);
    all_entries
}

fn is_supported_audio_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| AUDIO_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}
