//! Sorting finished downloads into folders by the kind of file they are
//! (videos, music, documents...), and moving a finished download to another
//! folder. A file is never written over: when the name is taken, a number
//! is added ("name (1).ext"), as everywhere else in the app.
//!
//! Only single files are sorted, and only those saved to the download
//! folder itself: a download given a folder of its own (by the user, by a
//! schedule with a folder, or as part of a batch) stays where it was put,
//! and profiles, playlists, torrents and file sets keep their own folders.

use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::probe;

/// One kind of file and the folder it goes to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileCategory {
    pub name: String,
    /// A folder inside the download folder ("Videos"), or a full path.
    pub folder: String,
    /// File endings, lower case, without the dot ("mp4").
    pub extensions: Vec<String>,
}

/// How finished downloads are sorted into folders.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Organize {
    pub enabled: bool,
    /// Looked through in order; the first that lists a file's ending wins.
    pub categories: Vec<FileCategory>,
    /// Where every other kind of file goes; empty leaves it in the download folder.
    #[serde(default)]
    pub other: String,
}

fn category(name: &str, exts: &[&str]) -> FileCategory {
    FileCategory {
        name: name.into(),
        folder: name.into(),
        extensions: exts.iter().map(|e| e.to_string()).collect(),
    }
}

impl Default for Organize {
    fn default() -> Self {
        Organize {
            enabled: true,
            categories: vec![
                category("Videos", &["mp4", "mkv", "avi", "mov", "webm"]),
                category("Music", &["mp3", "wav", "flac", "m4a", "aac"]),
                category("Documents", &["pdf", "docx", "doc", "xlsx", "pptx", "txt"]),
                category("Images", &["jpg", "jpeg", "png", "gif", "webp", "svg"]),
                category("Archives", &["zip", "rar", "7z", "tar", "gz"]),
                category("Software", &["exe", "msi", "dmg", "deb", "rpm", "apk"]),
            ],
            other: "Other".into(),
        }
    }
}

/// A file ending as kept: lower case, no dot, no spaces.
fn clean_ext(ext: &str) -> String {
    ext.trim().trim_start_matches('.').to_ascii_lowercase()
}

impl Organize {
    /// Tidied as typed: names and folders trimmed, endings lower case and
    /// each listed once (in the first category that has it).
    pub(crate) fn normalized(mut self) -> Self {
        let mut seen = std::collections::HashSet::new();
        for c in self.categories.iter_mut() {
            c.name = c.name.trim().to_string();
            c.folder = c.folder.trim().to_string();
            c.extensions = c
                .extensions
                .iter()
                .map(|e| clean_ext(e))
                .filter(|e| !e.is_empty() && !e.contains(['/', '\\', ' ']) && seen.insert(e.clone()))
                .collect();
        }
        self.other = self.other.trim().to_string();
        self
    }

    /// The folder (as set) a file of this name goes to.
    pub fn folder_for(&self, filename: &str) -> &str {
        let ext = Path::new(filename).extension().map(|e| clean_ext(&e.to_string_lossy())).unwrap_or_default();
        self.categories
            .iter()
            .find(|c| !ext.is_empty() && c.extensions.iter().any(|e| *e == ext))
            .map_or(self.other.as_str(), |c| c.folder.as_str())
    }
}

/// A folder as set: a full path as it is, anything else inside `base`.
pub(crate) fn resolve(base: &Path, folder: &str) -> PathBuf {
    let folder = folder.trim();
    if folder.is_empty() {
        return base.to_path_buf();
    }
    let path = Path::new(folder);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
}

/// Moves a file or folder into `dir`, under a name not taken there; returns
/// where it is now. Across drives a file is copied, then the original
/// removed; a folder can only be moved on the same drive.
pub(crate) fn move_into(src: &Path, dir: &Path) -> io::Result<PathBuf> {
    let name = src
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "nothing to move"))?;
    if src.parent() == Some(dir) {
        return Ok(src.to_path_buf());
    }
    if src.is_dir() && dir.starts_with(src) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "a folder cannot go inside itself"));
    }
    std::fs::create_dir_all(dir)?;
    let free = |n: u32| {
        let candidate = if src.is_dir() {
            if n == 0 { name.clone() } else { format!("{name} ({n})") }
        } else {
            probe::candidate(&name, n)
        };
        dir.join(candidate)
    };
    let dest = (0..10_000)
        .map(free)
        .find(|p| !p.exists())
        .ok_or_else(|| io::Error::new(io::ErrorKind::AlreadyExists, "no free name"))?;
    match std::fs::rename(src, &dest) {
        Ok(()) => Ok(dest),
        Err(e) if src.is_file() => {
            // Another drive: copy, and only then remove the original.
            std::fs::copy(src, &dest).map_err(|copy| {
                let _ = std::fs::remove_file(&dest);
                io::Error::new(copy.kind(), format!("{copy} (after: {e})"))
            })?;
            std::fs::remove_file(src)?;
            Ok(dest)
        }
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_go_to_the_folder_of_their_kind() {
        let o = Organize::default();
        assert_eq!(o.folder_for("talk.MP4"), "Videos");
        assert_eq!(o.folder_for("song.flac"), "Music");
        assert_eq!(o.folder_for("report.pdf"), "Documents");
        assert_eq!(o.folder_for("photo.JPEG"), "Images");
        assert_eq!(o.folder_for("backup.tar.gz"), "Archives");
        assert_eq!(o.folder_for("setup.exe"), "Software");
        assert_eq!(o.folder_for("data.bin"), "Other");
        assert_eq!(o.folder_for("README"), "Other");
    }

    #[test]
    fn custom_categories_and_endings_are_tidied() {
        let mut o = Organize::default();
        o.categories.insert(0, FileCategory { name: " Books ".into(), folder: " Books ".into(), extensions: vec![".EPUB".into(), "pdf".into(), " ".into()] });
        let o = o.normalized();
        assert_eq!(o.categories[0].name, "Books");
        assert_eq!(o.categories[0].extensions, vec!["epub", "pdf"]);
        // pdf now belongs to Books, the first to list it
        assert_eq!(o.folder_for("novel.pdf"), "Books");
        assert!(!o.categories.iter().find(|c| c.name == "Documents").unwrap().extensions.contains(&"pdf".to_string()));
    }

    #[test]
    fn moving_never_writes_over_a_file() {
        let base = std::env::temp_dir().join(format!("rush-organize-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let videos = resolve(&base, "Videos");
        assert_eq!(videos, base.join("Videos"));
        std::fs::create_dir_all(&videos).unwrap();
        std::fs::write(videos.join("clip.mp4"), b"old").unwrap();
        std::fs::write(base.join("clip.mp4"), b"new").unwrap();
        let moved = move_into(&base.join("clip.mp4"), &videos).unwrap();
        assert_eq!(moved, videos.join("clip (1).mp4"));
        assert_eq!(std::fs::read(videos.join("clip.mp4")).unwrap(), b"old");
        assert_eq!(std::fs::read(&moved).unwrap(), b"new");
        assert!(!base.join("clip.mp4").exists());
        // a folder moves whole, and never into itself
        std::fs::create_dir_all(base.join("set/inner")).unwrap();
        assert!(move_into(&base.join("set"), &base.join("set/inner")).is_err());
        let moved = move_into(&base.join("set"), &base.join("Elsewhere")).unwrap();
        assert!(moved.join("inner").is_dir());
        let _ = std::fs::remove_dir_all(&base);
    }
}
