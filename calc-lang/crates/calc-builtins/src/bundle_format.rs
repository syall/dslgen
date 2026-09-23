//! The format IPC bundles travel in inside a compiled program (session A12): `calcc
//! build` packs the bundles a program uses into one blob, the link driver embeds it
//! as data, and the IPC shim in `calc-runtime` reads it back and unpacks it. Defined
//! once, here, so the packing and unpacking sides can't disagree.
//!
//! Layout (all integers little-endian; a "string" is a `u32` byte length followed by
//! UTF-8 bytes):
//!
//! ```text
//! b"CALCBND1"                          magic and version
//! u32 bundle count
//! per bundle:
//!     string name
//!     u64    hash                      FNV-1a 64 of the bundle's file records
//!     u32    file count
//!     per file:
//!         string path                  relative, `/`-separated
//!         u32    mode                  Unix permission bits (0o755 keeps a helper executable)
//!         u64    length, then that many bytes
//! ```
//!
//! The hash only names the directory a bundle is unpacked into (so a changed bundle
//! never reuses a stale copy); it's not a security check.

/// Identifies a packed blob, and its format version.
pub const MAGIC: &[u8; 8] = b"CALCBND1";

/// One file inside a bundle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundleFile {
    /// Relative to the bundle's root, `/`-separated, no `..`.
    pub path: String,
    pub mode: u32,
    pub bytes: Vec<u8>,
}

/// A named bundle, as read back from a blob.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bundle {
    pub name: String,
    pub hash: u64,
    pub files: Vec<BundleFile>,
}

impl Bundle {
    /// Total size of the bundle's file contents, in bytes.
    pub fn size(&self) -> u64 {
        self.files.iter().map(|f| f.bytes.len() as u64).sum()
    }
}

/// Packs `bundles` (name, files) into one blob, computing each bundle's hash.
pub fn pack(bundles: &[(&str, Vec<BundleFile>)]) -> Vec<u8> {
    let mut out = MAGIC.to_vec();
    out.extend((bundles.len() as u32).to_le_bytes());
    for (name, files) in bundles {
        let mut records = Vec::new();
        records.extend((files.len() as u32).to_le_bytes());
        for file in files {
            put_str(&mut records, &file.path);
            records.extend(file.mode.to_le_bytes());
            records.extend((file.bytes.len() as u64).to_le_bytes());
            records.extend(&file.bytes);
        }
        put_str(&mut out, name);
        out.extend(fnv1a64(&records).to_le_bytes());
        out.extend(records);
    }
    out
}

/// Reads a blob written by [`pack`]. An empty slice means "no bundles" (what a
/// program linked without any gets); anything malformed, or a path that could
/// escape the bundle's directory, is `None`.
pub fn parse(blob: &[u8]) -> Option<Vec<Bundle>> {
    if blob.is_empty() {
        return Some(Vec::new());
    }
    let mut r = Reader(blob);
    if r.take(MAGIC.len())? != MAGIC {
        return None;
    }
    let count = r.u32()?;
    let mut bundles = Vec::new();
    for _ in 0..count {
        let name = r.string()?;
        let hash = r.u64()?;
        let file_count = r.u32()?;
        let mut files = Vec::new();
        for _ in 0..file_count {
            let path = r.string()?;
            if !is_safe_relative_path(&path) {
                return None;
            }
            let mode = r.u32()?;
            let len = usize::try_from(r.u64()?).ok()?;
            files.push(BundleFile {
                path,
                mode,
                bytes: r.take(len)?.to_vec(),
            });
        }
        bundles.push(Bundle { name, hash, files });
    }
    r.0.is_empty().then_some(bundles)
}

/// Relative, `/`-separated, and without `..`, `.` or empty components — so joining
/// it onto a directory can't land outside that directory.
fn is_safe_relative_path(path: &str) -> bool {
    !path.is_empty()
        && !path.contains('\\')
        && !path.contains(':')
        && path
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

/// FNV-1a, 64-bit: a tiny, stable, non-cryptographic hash.
pub fn fnv1a64(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, &b| {
        (hash ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

fn put_str(out: &mut Vec<u8>, s: &str) {
    out.extend((s.len() as u32).to_le_bytes());
    out.extend(s.as_bytes());
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        if self.0.len() < n {
            return None;
        }
        let (head, rest) = self.0.split_at(n);
        self.0 = rest;
        Some(head)
    }

    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }

    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }

    fn string(&mut self) -> Option<String> {
        let len = self.u32()? as usize;
        String::from_utf8(self.take(len)?.to_vec()).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str, mode: u32, bytes: &[u8]) -> BundleFile {
        BundleFile {
            path: path.into(),
            mode,
            bytes: bytes.into(),
        }
    }

    #[test]
    fn round_trips_bundles_with_paths_and_modes() {
        let files = vec![
            file("__main__.py", 0o644, b"import formatting"),
            file("bin/helper", 0o755, &[0, 1, 2]),
        ];
        let blob = pack(&[("print", files.clone())]);
        let bundles = parse(&blob).unwrap();
        assert_eq!(bundles.len(), 1);
        assert_eq!(bundles[0].name, "print");
        assert_eq!(bundles[0].files, files);
        assert_eq!(bundles[0].size(), 20);
    }

    #[test]
    fn no_bundles_round_trips_and_an_empty_slice_means_none() {
        assert_eq!(parse(&pack(&[])), Some(Vec::new()));
        assert_eq!(parse(&[]), Some(Vec::new()));
    }

    /// The hash names the unpack directory, so any content change must change it.
    #[test]
    fn the_hash_changes_with_the_content() {
        let a = parse(&pack(&[("p", vec![file("x", 0o644, b"1")])])).unwrap();
        let b = parse(&pack(&[("p", vec![file("x", 0o644, b"2")])])).unwrap();
        assert_ne!(a[0].hash, b[0].hash);
    }

    #[test]
    fn rejects_malformed_blobs_and_escaping_paths() {
        assert_eq!(parse(b"not a bundle blob"), None);
        let mut truncated = pack(&[("p", vec![file("x", 0o644, b"123")])]);
        truncated.pop();
        assert_eq!(parse(&truncated), None);
        for bad in ["../x", "/etc/passwd", "a/../../b", "C:/x", "a\\b", "a//b"] {
            assert_eq!(
                parse(&pack(&[("p", vec![file(bad, 0o644, b"")])])),
                None,
                "{bad}"
            );
        }
    }
}
