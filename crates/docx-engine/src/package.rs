//! The OPC package: the ZIP archive that holds a document's parts.
//!
//! Every part is kept in memory with its original name, order, compression
//! method and timestamp, and parts that were not modified are written back
//! byte-for-byte. Nothing in the archive is ever extracted to disk.

use std::io::{Cursor, Read, Write};

use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, ZipArchive, ZipWriter};

use crate::error::{DocxError, Result};

/// Safety limits applied when reading an archive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Most entries an archive may contain.
    pub max_entries: usize,
    /// Largest uncompressed size of a single part.
    pub max_part_bytes: u64,
    /// Largest total uncompressed size of all parts.
    pub max_total_bytes: u64,
    /// Largest ratio between a part's uncompressed and compressed size,
    /// checked for parts above 1 MiB (ZIP bomb protection).
    pub max_compression_ratio: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self { max_entries: 10_000, max_part_bytes: 256 << 20, max_total_bytes: 512 << 20, max_compression_ratio: 200 }
    }
}

const RATIO_CHECK_THRESHOLD: u64 = 1 << 20;

/// How a part is stored in the archive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compression {
    Stored,
    Deflated,
}

/// One file inside the package.
#[derive(Debug, Clone)]
pub struct Part {
    /// Part name as stored in the archive, without a leading slash
    /// (e.g. `word/document.xml`).
    pub name: String,
    pub data: Vec<u8>,
    pub compression: Compression,
    modified: Option<DateTime>,
}

impl Part {
    pub fn new(name: impl Into<String>, data: Vec<u8>) -> Self {
        Self { name: name.into(), data, compression: Compression::Deflated, modified: None }
    }
}

/// An OPC package (a DOCX file's ZIP container).
#[derive(Debug, Clone, Default)]
pub struct Package {
    parts: Vec<Part>,
}

/// Signature of OLE compound files: encrypted OOXML and legacy `.doc`.
const OLE_SIGNATURE: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];

impl Package {
    /// Builds a package from parts (used for new documents and tests).
    pub fn from_parts(parts: Vec<Part>) -> Self {
        Self { parts }
    }

    /// Reads a package from the bytes of a ZIP archive.
    pub fn read(bytes: &[u8], limits: Limits) -> Result<Self> {
        if bytes.starts_with(&OLE_SIGNATURE) {
            return Err(DocxError::EncryptedOrLegacy);
        }
        if !bytes.starts_with(b"PK") {
            return Err(DocxError::InvalidArchive("it is not a ZIP archive".into()));
        }
        let mut archive = ZipArchive::new(Cursor::new(bytes))
            .map_err(|e| DocxError::InvalidArchive(format!("the ZIP structure is damaged ({e})")))?;
        if archive.len() > limits.max_entries {
            return Err(DocxError::TooLarge(format!("it contains more than {} files", limits.max_entries)));
        }

        let mut parts: Vec<Part> = Vec::with_capacity(archive.len());
        let mut total: u64 = 0;
        for index in 0..archive.len() {
            let mut entry = archive
                .by_index(index)
                .map_err(|e| DocxError::InvalidArchive(format!("entry {index} cannot be read ({e})")))?;
            if entry.is_dir() {
                continue;
            }
            let raw_name = entry
                .name()
                .map_err(|e| DocxError::InvalidArchive(format!("entry {index} has an unreadable name ({e})")))?
                .into_owned();
            let name = validate_part_name(&raw_name)?;
            if entry.encrypted() {
                return Err(DocxError::EncryptedOrLegacy);
            }
            if parts.iter().any(|p| p.name.eq_ignore_ascii_case(&name)) {
                return Err(DocxError::InvalidArchive(format!("the part '{name}' appears more than once")));
            }
            let compression = match entry.compression() {
                CompressionMethod::Stored => Compression::Stored,
                CompressionMethod::Deflated => Compression::Deflated,
                other => {
                    return Err(DocxError::InvalidArchive(format!(
                        "the part '{name}' uses an unsupported compression method ({other})"
                    )));
                }
            };

            // Declared sizes are only a first check; reads are limited regardless.
            let declared = entry.size();
            if declared > limits.max_part_bytes || total.saturating_add(declared) > limits.max_total_bytes {
                return Err(DocxError::TooLarge(format!("the part '{name}' is too large")));
            }
            let remaining = limits.max_total_bytes - total;
            let cap = limits.max_part_bytes.min(remaining);
            let compressed = entry.compressed_size().max(1);
            let modified = entry.last_modified();

            let mut data = Vec::with_capacity(declared.min(cap) as usize);
            (&mut entry)
                .take(cap + 1)
                .read_to_end(&mut data)
                .map_err(|e| DocxError::InvalidArchive(format!("the part '{name}' is damaged ({e})")))?;
            let size = data.len() as u64;
            if size > cap {
                return Err(DocxError::TooLarge(format!("the part '{name}' expands beyond the size limit")));
            }
            if size > RATIO_CHECK_THRESHOLD && size / compressed > limits.max_compression_ratio {
                return Err(DocxError::TooLarge(format!("the part '{name}' has a suspicious compression ratio")));
            }
            total += size;
            parts.push(Part { name, data, compression, modified });
        }
        Ok(Self { parts })
    }

    /// Writes the package as a ZIP archive, keeping part order, compression
    /// methods and timestamps.
    pub fn write(&self) -> Result<Vec<u8>> {
        self.write_with(&[])
    }

    /// Writes the package with the contents of some parts replaced, without
    /// modifying (or copying) the package itself.
    pub fn write_with(&self, replacements: &[(&str, &[u8])]) -> Result<Vec<u8>> {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        for part in &self.parts {
            let method = match part.compression {
                Compression::Stored => CompressionMethod::Stored,
                Compression::Deflated => CompressionMethod::Deflated,
            };
            let mut options = SimpleFileOptions::default().compression_method(method).large_file(false);
            if let Some(modified) = part.modified {
                options = options.last_modified_time(modified);
            }
            let data = replacements
                .iter()
                .find(|(name, _)| name.trim_start_matches('/').eq_ignore_ascii_case(&part.name))
                .map_or(part.data.as_slice(), |(_, data)| data);
            writer
                .start_file(part.name.as_str(), options)
                .map_err(|e| DocxError::Write(format!("{}: {e}", part.name)))?;
            writer.write_all(data).map_err(|e| DocxError::Write(format!("{}: {e}", part.name)))?;
        }
        let cursor = writer.finish().map_err(|e| DocxError::Write(e.to_string()))?;
        Ok(cursor.into_inner())
    }

    /// All parts, in archive order.
    pub fn parts(&self) -> &[Part] {
        &self.parts
    }

    /// Finds a part by name (case-insensitive, as OPC part names are).
    pub fn part(&self, name: &str) -> Option<&Part> {
        let name = name.trim_start_matches('/');
        self.parts.iter().find(|p| p.name.eq_ignore_ascii_case(name))
    }

    /// Replaces the contents of an existing part, or appends a new part.
    pub fn set_part(&mut self, name: &str, data: Vec<u8>) {
        let name = name.trim_start_matches('/');
        match self.parts.iter_mut().find(|p| p.name.eq_ignore_ascii_case(name)) {
            Some(part) => part.data = data,
            None => self.parts.push(Part::new(name, data)),
        }
    }
}

/// Checks that an archive entry name is a safe, relative part name.
fn validate_part_name(raw: &str) -> Result<String> {
    let name = raw.trim_start_matches('/');
    let unsafe_name = name.is_empty()
        || name.contains('\\')
        || name.contains('\0')
        || name.split('/').any(|seg| seg.is_empty() || seg == "." || seg == "..")
        || name.chars().any(char::is_control)
        || name.len() > 1024;
    if unsafe_name {
        return Err(DocxError::InvalidArchive(format!("it contains an invalid part name ({:?})", truncate(raw, 80))));
    }
    Ok(name.to_string())
}

fn truncate(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zip_of(entries: &[(&str, &[u8], CompressionMethod)]) -> Vec<u8> {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        for (name, data, method) in entries {
            writer.start_file(*name, SimpleFileOptions::default().compression_method(*method)).unwrap();
            writer.write_all(data).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    #[test]
    fn reads_and_writes_parts_in_order_with_compression() {
        let bytes = zip_of(&[
            ("[Content_Types].xml", b"<Types/>", CompressionMethod::Deflated),
            ("word/media/image1.png", b"\x89PNG data", CompressionMethod::Stored),
            ("word/document.xml", b"<w:document/>", CompressionMethod::Deflated),
        ]);
        let package = Package::read(&bytes, Limits::default()).unwrap();
        let names: Vec<_> = package.parts().iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["[Content_Types].xml", "word/media/image1.png", "word/document.xml"]);
        assert_eq!(package.part("word/media/image1.png").unwrap().compression, Compression::Stored);
        assert_eq!(package.part("/WORD/document.xml").unwrap().data, b"<w:document/>");

        let reread = Package::read(&package.write().unwrap(), Limits::default()).unwrap();
        for (a, b) in package.parts().iter().zip(reread.parts()) {
            assert_eq!(a.name, b.name);
            assert_eq!(a.data, b.data);
            assert_eq!(a.compression, b.compression);
        }
    }

    #[test]
    fn set_part_replaces_or_appends() {
        let mut package = Package::from_parts(vec![Part::new("a.xml", b"1".to_vec())]);
        package.set_part("/a.xml", b"2".to_vec());
        package.set_part("b.xml", b"3".to_vec());
        assert_eq!(package.part("a.xml").unwrap().data, b"2");
        assert_eq!(package.parts().len(), 2);
    }

    #[test]
    fn rejects_non_zip_and_ole_files() {
        assert!(matches!(Package::read(b"hello", Limits::default()), Err(DocxError::InvalidArchive(_))));
        assert!(matches!(Package::read(b"", Limits::default()), Err(DocxError::InvalidArchive(_))));
        let mut ole = OLE_SIGNATURE.to_vec();
        ole.extend_from_slice(&[0; 64]);
        assert_eq!(Package::read(&ole, Limits::default()).unwrap_err(), DocxError::EncryptedOrLegacy);
        let truncated = &zip_of(&[("a.xml", b"<a/>", CompressionMethod::Deflated)])[..20];
        assert!(matches!(Package::read(truncated, Limits::default()), Err(DocxError::InvalidArchive(_))));
    }

    #[test]
    fn rejects_unsafe_and_duplicate_names() {
        for name in ["../evil.xml", "word/../../evil.xml", "word//document.xml", "word\\document.xml", "./a.xml"] {
            let bytes = zip_of(&[(name, b"x", CompressionMethod::Stored)]);
            let err = Package::read(&bytes, Limits::default()).unwrap_err();
            assert!(matches!(err, DocxError::InvalidArchive(_)), "{name}: {err:?}");
        }
        let bytes =
            zip_of(&[("word/a.xml", b"1", CompressionMethod::Stored), ("WORD/A.xml", b"2", CompressionMethod::Stored)]);
        assert!(
            matches!(Package::read(&bytes, Limits::default()), Err(DocxError::InvalidArchive(m)) if m.contains("more than once"))
        );
    }

    #[test]
    fn enforces_limits() {
        let bytes = zip_of(&[("a.xml", b"1", CompressionMethod::Stored), ("b.xml", b"2", CompressionMethod::Stored)]);
        let few = Limits { max_entries: 1, ..Limits::default() };
        assert!(matches!(Package::read(&bytes, few), Err(DocxError::TooLarge(_))));

        let big = vec![b'a'; 4 << 20];
        let bomb = zip_of(&[("word/document.xml", &big, CompressionMethod::Deflated)]);
        assert!(bomb.len() < 64 << 10, "test archive should compress well");
        let err = Package::read(&bomb, Limits::default()).unwrap_err();
        assert!(matches!(&err, DocxError::TooLarge(m) if m.contains("compression ratio")), "{err:?}");

        let small_parts = Limits { max_part_bytes: 1024, ..Limits::default() };
        assert!(matches!(Package::read(&bomb, small_parts), Err(DocxError::TooLarge(_))));
        let small_total = Limits { max_total_bytes: 1024, ..Limits::default() };
        assert!(matches!(Package::read(&bomb, small_total), Err(DocxError::TooLarge(_))));
    }
}
