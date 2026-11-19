//! Errors produced by the document engine.

use std::fmt;

/// Everything that can go wrong when reading, editing or writing a document.
/// Messages are written for end users.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocxError {
    /// The bytes are not a ZIP archive, or the archive is damaged.
    InvalidArchive(String),
    /// A password-protected document or a legacy binary `.doc` file (both are
    /// OLE compound files, not ZIP archives).
    EncryptedOrLegacy,
    /// The archive is valid but is not a Word document.
    NotWordDocument(String),
    /// The archive exceeds a safety limit (size, entry count, expansion).
    TooLarge(String),
    /// A part contains XML that cannot be parsed safely.
    MalformedXml { part: String, message: String },
    /// An editing operation was invalid (e.g. an index out of range).
    InvalidEdit(String),
    /// Writing the archive failed.
    Write(String),
}

impl fmt::Display for DocxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidArchive(msg) => write!(f, "The file is not a valid .docx document: {msg}"),
            Self::EncryptedOrLegacy => f.write_str(
                "The file is password-protected or uses the old .doc format. Remove the password or save it as \
                 .docx in Word, then try again.",
            ),
            Self::NotWordDocument(msg) => write!(f, "The file is not a Word document: {msg}"),
            Self::TooLarge(msg) => write!(f, "The document is too large to open safely: {msg}"),
            Self::MalformedXml { part, message } => write!(f, "The document is damaged ({part}: {message})."),
            Self::InvalidEdit(msg) => write!(f, "Invalid edit: {msg}"),
            Self::Write(msg) => write!(f, "The document could not be saved: {msg}"),
        }
    }
}

impl std::error::Error for DocxError {}

pub type Result<T> = std::result::Result<T, DocxError>;
