//! Identify Windows programs the way the Windows loader and Explorer would.
//!
//! SegLinux uses this crate to decide *how* to start a file the user
//! double-clicked: a 64-bit GUI program, a .NET assembly, an MSI package, an
//! NSIS/Inno Setup installer, a `.lnk` shortcut or a batch script all need a
//! slightly different Wine invocation and prefix setup.
//!
//! The parser has no dependencies, only performs bounded reads and never
//! trusts offsets found in the file, so it is safe to run on arbitrary
//! downloads.

mod pe;
mod reader;
mod sig;

use std::fs::File;
use std::io::{self, Read, Seek};
use std::path::Path;

pub use pe::{InstallerKind, Machine, PeFormat, PeInfo, Section, Subsystem};

/// What kind of Windows file something is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileKind {
    /// A Portable Executable: `.exe`, `.dll`, `.scr`, `.sys`, ...
    Pe(Box<PeInfo>),
    /// `MZ` header without a valid PE header (16-bit DOS, NE or LE program).
    Dos,
    /// Windows Installer package (`.msi` / `.msp`).
    Msi,
    /// OLE compound document that is not an installer package (e.g. `.doc`).
    OleCompound,
    /// Shell link (`.lnk`).
    Shortcut,
    /// `cmd.exe` batch script (`.bat` / `.cmd`).
    Batch,
    /// Not something SegLinux knows how to run.
    Unknown,
}

impl FileKind {
    /// Short human readable description, as shown by `segrun info`.
    pub fn describe(&self) -> String {
        match self {
            FileKind::Pe(info) => info.describe(),
            FileKind::Dos => "16-bit DOS/Windows program".to_string(),
            FileKind::Msi => "Windows Installer package (MSI)".to_string(),
            FileKind::OleCompound => "OLE compound document".to_string(),
            FileKind::Shortcut => "Windows shortcut (.lnk)".to_string(),
            FileKind::Batch => "Batch script".to_string(),
            FileKind::Unknown => "Unknown file type".to_string(),
        }
    }

    /// Whether SegLinux can try to start this file with Wine at all.
    pub fn is_launchable(&self) -> bool {
        match self {
            FileKind::Pe(info) => info.is_launchable(),
            FileKind::Dos | FileKind::Msi | FileKind::Shortcut | FileKind::Batch => true,
            FileKind::OleCompound | FileKind::Unknown => false,
        }
    }
}

/// Analyse a file on disk. The extension is used to tell batch scripts and
/// MSI packages apart from other files with the same magic bytes.
pub fn analyze_path(path: &Path) -> io::Result<FileKind> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase());
    let mut file = File::open(path)?;
    analyze_reader(&mut file, ext.as_deref())
}

/// Analyse an in-memory buffer.
pub fn analyze_bytes(data: &[u8], ext: Option<&str>) -> FileKind {
    let mut cursor = io::Cursor::new(data);
    // Reading from a cursor cannot fail.
    analyze_reader(&mut cursor, ext).unwrap_or(FileKind::Unknown)
}

/// Analyse anything that can be read and seeked.
pub fn analyze_reader<R: Read + Seek>(r: &mut R, ext: Option<&str>) -> io::Result<FileKind> {
    let ext = ext.map(|e| e.trim_start_matches('.').to_ascii_lowercase());
    let mut src = reader::Source::new(r)?;
    let head = src.read_at(0, 64)?;

    if head.starts_with(b"MZ") {
        return Ok(match pe::parse(&mut src)? {
            Some(info) => FileKind::Pe(Box::new(info)),
            None => FileKind::Dos,
        });
    }
    if head.starts_with(&sig::OLE_MAGIC) {
        return Ok(match ext.as_deref() {
            Some("msi" | "msp") => FileKind::Msi,
            _ => FileKind::OleCompound,
        });
    }
    if head.len() >= 20 && head[..4] == sig::LNK_HEADER_SIZE && head[4..20] == sig::LNK_CLSID {
        return Ok(FileKind::Shortcut);
    }
    if matches!(ext.as_deref(), Some("bat" | "cmd")) {
        return Ok(FileKind::Batch);
    }
    Ok(FileKind::Unknown)
}

#[cfg(test)]
mod tests;
