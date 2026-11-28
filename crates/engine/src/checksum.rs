//! File integrity: a downloaded file's checksum, worked out once it is
//! finished and compared with the one the user gave (from the site that
//! offers the file). The file is read in pieces on a worker thread, so a
//! large one neither freezes anything nor sits whole in memory.
//!
//! A failed comparison is never shown as fine: the download keeps its
//! finished file, and its checksum says "failed" with both sums.

use std::io::Read;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HashAlgo {
    Md5,
    Sha1,
    #[default]
    Sha256,
    Sha512,
}

impl HashAlgo {
    /// Hex digits in a sum of this kind.
    fn hex_len(self) -> usize {
        match self {
            HashAlgo::Md5 => 32,
            HashAlgo::Sha1 => 40,
            HashAlgo::Sha256 => 64,
            HashAlgo::Sha512 => 128,
        }
    }

    fn from_len(len: usize) -> Option<HashAlgo> {
        [HashAlgo::Md5, HashAlgo::Sha1, HashAlgo::Sha256, HashAlgo::Sha512].into_iter().find(|a| a.hex_len() == len)
    }

    fn from_name(name: &str) -> Option<HashAlgo> {
        match name.to_ascii_lowercase().replace(['-', '_', ' '], "").as_str() {
            "md5" => Some(HashAlgo::Md5),
            "sha1" => Some(HashAlgo::Sha1),
            "sha256" => Some(HashAlgo::Sha256),
            "sha512" => Some(HashAlgo::Sha512),
            _ => None,
        }
    }
}

/// Where a download's check stands.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckState {
    /// Not worked out yet, or worked out with nothing to compare it with.
    #[default]
    NotVerified,
    Calculating,
    /// It matches the sum given.
    Verified,
    /// It does not match: the file is not the one the sum belongs to.
    Failed,
    /// It could not be worked out (the file is gone, cannot be read...).
    Error,
}

/// A download's checksum: what was given, what the file has, and how the
/// two compare.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Checksum {
    pub algorithm: HashAlgo,
    /// The sum the user gave, lower-case hex.
    #[serde(default)]
    pub expected: Option<String>,
    /// The sum worked out from the file, lower-case hex.
    #[serde(default)]
    pub actual: Option<String>,
    #[serde(default)]
    pub state: CheckState,
    #[serde(default)]
    pub error: Option<String>,
    /// While it is worked out: how many bytes of the file have been read.
    #[serde(default)]
    pub done: u64,
}

impl Checksum {
    pub(crate) fn wanted(algorithm: HashAlgo, expected: Option<String>) -> Checksum {
        Checksum { algorithm, expected, ..Default::default() }
    }

    /// The outcome once the file's sum is known.
    pub(crate) fn settle(&mut self, actual: String) {
        self.state = match &self.expected {
            Some(expected) if *expected == actual => CheckState::Verified,
            Some(_) => CheckState::Failed,
            None => CheckState::NotVerified,
        };
        self.actual = Some(actual);
        self.error = None;
    }
}

/// Reads a sum as people paste it: "sha256:ab12…", "SHA-256 AB12…",
/// "ab12…  file.zip" (a line of a sums file), or the hex alone, whose kind
/// is told by its length. `algorithm` is used where the text names none
/// and the length allows it.
pub fn parse_expected(text: &str, algorithm: Option<HashAlgo>) -> Result<(HashAlgo, String), String> {
    let text = text.trim();
    let (named, rest) = match text.split_once([':', ' ', '=']) {
        Some((head, tail)) if HashAlgo::from_name(head).is_some() => (HashAlgo::from_name(head), tail.trim()),
        _ => (None, text),
    };
    // a line of a sums file: the hex, then the file's name
    let hex = rest.split_whitespace().next().unwrap_or_default().trim_start_matches('*').to_ascii_lowercase();
    if hex.is_empty() {
        return Err("Enter the checksum".into());
    }
    if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err("A checksum is made of the digits 0-9 and the letters a-f".into());
    }
    let by_len = HashAlgo::from_len(hex.len())
        .ok_or_else(|| format!("That checksum has {} digits; MD5 has 32, SHA-1 40, SHA-256 64 and SHA-512 128", hex.len()))?;
    let algorithm = named.or(algorithm.filter(|a| a.hex_len() == hex.len())).unwrap_or(by_len);
    if algorithm.hex_len() != hex.len() {
        return Err(format!("A {} checksum has {} digits, not {}", name(algorithm), algorithm.hex_len(), hex.len()));
    }
    Ok((algorithm, hex))
}

pub fn name(algorithm: HashAlgo) -> &'static str {
    match algorithm {
        HashAlgo::Md5 => "MD5",
        HashAlgo::Sha1 => "SHA-1",
        HashAlgo::Sha256 => "SHA-256",
        HashAlgo::Sha512 => "SHA-512",
    }
}

enum Hasher {
    Md5(md5::Md5),
    Ring(ring::digest::Context),
}

impl Hasher {
    fn new(algorithm: HashAlgo) -> Hasher {
        match algorithm {
            HashAlgo::Md5 => Hasher::Md5(<md5::Md5 as md5::Digest>::new()),
            HashAlgo::Sha1 => Hasher::Ring(ring::digest::Context::new(&ring::digest::SHA1_FOR_LEGACY_USE_ONLY)),
            HashAlgo::Sha256 => Hasher::Ring(ring::digest::Context::new(&ring::digest::SHA256)),
            HashAlgo::Sha512 => Hasher::Ring(ring::digest::Context::new(&ring::digest::SHA512)),
        }
    }

    fn update(&mut self, data: &[u8]) {
        match self {
            Hasher::Md5(h) => md5::Digest::update(h, data),
            Hasher::Ring(h) => h.update(data),
        }
    }

    fn finish(self) -> String {
        let bytes: Vec<u8> = match self {
            Hasher::Md5(h) => md5::Digest::finalize(h).to_vec(),
            Hasher::Ring(h) => h.finish().as_ref().to_vec(),
        };
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }
}

/// Works out a file's sum, reading it a piece at a time. `progress` hears
/// how far it is now and then; `stop` ends it early (`Ok(None)`).
pub(crate) fn hash_file(path: &Path, algorithm: HashAlgo, stop: &AtomicBool, mut progress: impl FnMut(u64)) -> Result<Option<String>, String> {
    let mut file = std::fs::File::open(path).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => "The file is no longer where it was saved".to_string(),
        std::io::ErrorKind::PermissionDenied => "The file is in use or may not be read; try again in a moment".to_string(),
        _ => format!("The file could not be read: {e}"),
    })?;
    let mut hasher = Hasher::new(algorithm);
    let mut buf = vec![0u8; 1 << 20];
    let mut done = 0u64;
    let mut told = 0u64;
    loop {
        if stop.load(Relaxed) {
            return Ok(None);
        }
        let n = file.read(&mut buf).map_err(|e| format!("The file could not be read: {e}"))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        done += n as u64;
        if done - told >= 32 << 20 {
            told = done;
            progress(done);
        }
    }
    progress(done);
    Ok(Some(hasher.finish()))
}

/// The sum a sums file ("<hex>  <name>" lines) gives for `file_name`.
pub(crate) fn sum_for(sums: &str, file_name: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let (hex, name) = line.trim().split_once(char::is_whitespace)?;
        let name = name.trim().trim_start_matches('*');
        let name = name.rsplit(['/', '\\']).next().unwrap_or(name);
        (name.eq_ignore_ascii_case(file_name) && HashAlgo::from_len(hex.len()).is_some() && hex.chars().all(|c| c.is_ascii_hexdigit()))
            .then(|| hex.to_ascii_lowercase())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const ABC_MD5: &str = "900150983cd24fb0d6963f7d28e17f72";
    const ABC_SHA1: &str = "a9993e364706816aba3e25717850c26c9cd0d89d";
    const ABC_SHA256: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
    const ABC_SHA512: &str = "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f";

    #[test]
    fn every_kind_gives_the_known_sum() {
        let dir = std::env::temp_dir().join(format!("rd-hash-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("abc.txt");
        std::fs::write(&file, b"abc").unwrap();
        let stop = AtomicBool::new(false);
        for (algo, sum) in [(HashAlgo::Md5, ABC_MD5), (HashAlgo::Sha1, ABC_SHA1), (HashAlgo::Sha256, ABC_SHA256), (HashAlgo::Sha512, ABC_SHA512)] {
            assert_eq!(hash_file(&file, algo, &stop, |_| {}).unwrap().as_deref(), Some(sum), "{algo:?}");
        }
        stop.store(true, Relaxed);
        assert_eq!(hash_file(&file, HashAlgo::Sha256, &stop, |_| {}).unwrap(), None, "stopped");
        assert!(hash_file(&dir.join("gone.txt"), HashAlgo::Sha256, &AtomicBool::new(false), |_| {}).unwrap_err().contains("no longer"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sums_are_read_as_people_paste_them() {
        assert_eq!(parse_expected(ABC_SHA256, None).unwrap(), (HashAlgo::Sha256, ABC_SHA256.into()));
        assert_eq!(parse_expected(&ABC_SHA256.to_uppercase(), None).unwrap().1, ABC_SHA256);
        assert_eq!(parse_expected(&format!("sha256:{ABC_SHA256}"), None).unwrap().0, HashAlgo::Sha256);
        assert_eq!(parse_expected(&format!("SHA-1 {ABC_SHA1}"), None).unwrap(), (HashAlgo::Sha1, ABC_SHA1.into()));
        assert_eq!(parse_expected(&format!("  {ABC_MD5}  setup.exe "), None).unwrap(), (HashAlgo::Md5, ABC_MD5.into()));
        assert_eq!(parse_expected(&format!("{ABC_SHA512} *file.iso"), None).unwrap().0, HashAlgo::Sha512);
        assert!(parse_expected("", None).is_err());
        assert!(parse_expected("xyz", None).unwrap_err().contains("0-9"));
        assert!(parse_expected("abcd", None).unwrap_err().contains("4 digits"));
        assert!(parse_expected(&format!("md5:{ABC_SHA256}"), None).unwrap_err().contains("MD5"));
        // a chosen kind that does not fit the length gives way to the length
        assert_eq!(parse_expected(ABC_MD5, Some(HashAlgo::Sha256)).unwrap().0, HashAlgo::Md5);
    }

    #[test]
    fn a_wrong_sum_is_never_shown_as_fine() {
        let mut c = Checksum::wanted(HashAlgo::Sha256, Some(ABC_SHA256.into()));
        c.settle(ABC_SHA256.into());
        assert_eq!(c.state, CheckState::Verified);
        let mut c = Checksum::wanted(HashAlgo::Sha256, Some(ABC_SHA256.into()));
        c.settle("0".repeat(64));
        assert_eq!(c.state, CheckState::Failed);
        let mut c = Checksum::wanted(HashAlgo::Sha256, None);
        c.settle(ABC_SHA256.into());
        assert_eq!(c.state, CheckState::NotVerified, "worked out, with nothing to compare it with");
        assert_eq!(c.actual.as_deref(), Some(ABC_SHA256));
    }

    #[test]
    fn a_sums_file_gives_each_files_sum() {
        let sums = format!("{ABC_SHA256}  dist/app-1.0.zip\n{ABC_MD5} *Setup.EXE\nnot a line\n");
        assert_eq!(sum_for(&sums, "app-1.0.zip").as_deref(), Some(ABC_SHA256));
        assert_eq!(sum_for(&sums, "setup.exe").as_deref(), Some(ABC_MD5));
        assert_eq!(sum_for(&sums, "other.zip"), None);
    }
}
