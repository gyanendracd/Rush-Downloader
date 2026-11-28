//! Reading files of links for a batch download (any text: a list, JSON,
//! CSV, HTML, a playlist...) and saving a list of links back out. Whether a
//! file is text is decided by what is in it, not by its name; pictures,
//! archives, programs and other binary files are turned away.

use std::path::Path;

/// Larger files are not read: no list of links is this big.
const MAX_BYTES: u64 = 64 * 1024 * 1024;

/// A text file that was read.
#[derive(Debug, serde::Serialize)]
pub struct TextFile {
    /// The file's name, without its folder.
    pub name: String,
    pub text: String,
    pub bytes: u64,
}

/// Formats that are known not to be text, by how their files begin.
const BINARY_STARTS: &[&[u8]] = &[
    b"%PDF",
    b"PK\x03\x04",
    b"PK\x05\x06",
    b"\x89PNG",
    b"GIF8",
    b"\xFF\xD8\xFF",
    b"Rar!",
    b"7z\xBC\xAF",
    b"\x1F\x8B",
    b"MZ",
    b"\x7FELF",
    b"ID3",
    b"OggS",
    b"fLaC",
    b"RIFF",
    b"\x1A\x45\xDF\xA3",
    b"SQLite format 3",
    b"\xD0\xCF\x11\xE0",
    b"d8:announce",
];

/// The text in `bytes`, or why it is not text.
pub fn decode(bytes: &[u8]) -> Result<String, &'static str> {
    if let Some(rest) = bytes.strip_prefix(b"\xEF\xBB\xBF") {
        return Ok(String::from_utf8_lossy(rest).into_owned());
    }
    if let Some(rest) = bytes.strip_prefix(b"\xFF\xFE") {
        return Ok(utf16(rest, false));
    }
    if let Some(rest) = bytes.strip_prefix(b"\xFE\xFF") {
        return Ok(utf16(rest, true));
    }
    if BINARY_STARTS.iter().any(|start| bytes.starts_with(start)) {
        return Err("Not a text file");
    }
    let sample = &bytes[..bytes.len().min(64 * 1024)];
    if sample.is_empty() {
        return Ok(String::new());
    }
    let zeros = sample.iter().filter(|b| **b == 0).count();
    if zeros > 0 {
        // Text written as UTF-16 without a mark: every other byte is zero.
        let odd_zeros = sample.iter().skip(1).step_by(2).filter(|b| **b == 0).count();
        let even_zeros = sample.iter().step_by(2).filter(|b| **b == 0).count();
        let half = sample.len() / 2;
        if half > 0 && odd_zeros * 10 >= half * 9 && even_zeros * 10 < half {
            return Ok(utf16(bytes, false));
        }
        if half > 0 && even_zeros * 10 >= half * 9 && odd_zeros * 10 < half {
            return Ok(utf16(bytes, true));
        }
        return Err("Not a text file");
    }
    // Text has almost no control characters besides tabs and line ends.
    let controls = sample
        .iter()
        .filter(|b| **b < 0x20 && !matches!(**b, b'\t' | b'\n' | b'\r' | 0x0C | 0x1B))
        .count();
    if controls * 100 > sample.len() {
        return Err("Not a text file");
    }
    Ok(String::from_utf8_lossy(bytes).into_owned())
}

fn utf16(bytes: &[u8], big_endian: bool) -> String {
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|p| if big_endian { u16::from_be_bytes([p[0], p[1]]) } else { u16::from_le_bytes([p[0], p[1]]) })
        .collect();
    String::from_utf16_lossy(&units)
}

/// Reads a file of links as text.
pub fn read(path: &Path) -> Result<TextFile, String> {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let meta = std::fs::metadata(path).map_err(|e| format!("{name}: could not open it ({e})"))?;
    if meta.is_dir() {
        return Err(format!("{name}: is a folder, not a file"));
    }
    if meta.len() > MAX_BYTES {
        return Err(format!("{name}: too large to be a list of links"));
    }
    let bytes = std::fs::read(path).map_err(|e| format!("{name}: could not read it ({e})"))?;
    let text = decode(&bytes).map_err(|why| format!("{name}: {why}"))?;
    Ok(TextFile { name, text, bytes: meta.len() })
}

/// Saves `contents` to `path` (a place the user picked to save to).
pub fn write(path: &Path, contents: &str) -> Result<(), String> {
    std::fs::write(path, contents).map_err(|e| format!("Could not save {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_in_any_encoding_is_read_and_binary_files_are_not() {
        let line = "see https://example.com/a.zip?x=1 and more";
        assert_eq!(decode(line.as_bytes()).unwrap(), line);
        let with_bom = [b"\xEF\xBB\xBF".as_slice(), line.as_bytes()].concat();
        assert_eq!(decode(&with_bom).unwrap(), line);
        let le: Vec<u8> = line.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
        assert_eq!(decode(&[b"\xFF\xFE".as_slice(), &le].concat()).unwrap(), line);
        assert_eq!(decode(&le).unwrap(), line, "UTF-16 without its mark");
        let be: Vec<u8> = line.encode_utf16().flat_map(|u| u.to_be_bytes()).collect();
        assert_eq!(decode(&[b"\xFE\xFF".as_slice(), &be].concat()).unwrap(), line);
        // a file named .txt that is really a picture or an archive
        assert!(decode(b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR").is_err());
        assert!(decode(b"PK\x03\x04\x14\0\0\0").is_err());
        assert!(decode(&[0u8, 1, 2, 3, 200, 7, 0, 9, 1, 1, 1]).is_err());
        assert_eq!(decode(b"").unwrap(), "");
        // RTF, JSON and HTML are text
        assert!(decode(br"{\rtf1\ansi {\field{\*\fldinst HYPERLINK https://a.test/x}}}").is_ok());
    }

    #[test]
    fn a_list_is_saved_and_read_back() {
        let dir = std::env::temp_dir().join(format!("rush-textfile-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("extracted-urls.txt");
        write(&file, "https://a.test/1\nhttps://a.test/2\n").unwrap();
        let read_back = read(&file).unwrap();
        assert_eq!(read_back.name, "extracted-urls.txt");
        assert_eq!(read_back.text, "https://a.test/1\nhttps://a.test/2\n");
        assert!(read(&dir).is_err(), "a folder is not a file");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
