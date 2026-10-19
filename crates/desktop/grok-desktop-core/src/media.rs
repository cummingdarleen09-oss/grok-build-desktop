//! Project media the workbench can preview without embedding the classic TUI.

use std::path::{Path, PathBuf};

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct MediaFile {
    pub path: String,
    pub name: String,
    pub kind: String,
}

pub fn list_media(cwd: &str) -> Vec<MediaFile> {
    let root = PathBuf::from(cwd);
    if !root.is_dir() {
        return Vec::new();
    }
    let mut found = Vec::new();
    walk(&root, &root, 0, &mut found);
    found.sort_by(|left, right| left.path.cmp(&right.path));
    found.truncate(80);
    found
}

fn walk(root: &Path, dir: &Path, depth: usize, found: &mut Vec<MediaFile>) {
    if depth > 3 || found.len() >= 80 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        if found.len() >= 80 {
            return;
        }
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') || name == "node_modules" || name == "target" || name == "dist" {
            continue;
        }
        if path.is_dir() {
            walk(root, &path, depth + 1, found);
            continue;
        }
        let Some(kind) = kind_of(&name) else {
            continue;
        };
        found.push(MediaFile {
            path: path.display().to_string(),
            name,
            kind: kind.into(),
        });
    }
}

fn kind_of(name: &str) -> Option<&'static str> {
    let ext = name.rsplit('.').next()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" => "image",
        "mp4" | "webm" | "mov" => "video",
        "mp3" | "wav" | "ogg" | "m4a" => "audio",
        "pdf" => "pdf",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_images_and_skips_build_dirs() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("shot.png"), b"png").unwrap();
        std::fs::create_dir_all(dir.path().join("target")).unwrap();
        std::fs::write(dir.path().join("target").join("noise.png"), b"no").unwrap();
        let listed = list_media(dir.path().to_str().unwrap());
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].kind, "image");
        assert!(listed[0].name.ends_with("shot.png"));
    }
}
