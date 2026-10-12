//! Portable Executable header, section table and import table parsing.
//!
//! Reference: "PE Format", Microsoft Learn (windows/win32/debug/pe-format).

use std::collections::BTreeSet;
use std::fmt;
use std::io::{self, Read, Seek};

use crate::reader::{Source, u16_at, u32_at, u64_at};
use crate::sig;

const IMAGE_FILE_LARGE_ADDRESS_AWARE: u16 = 0x0020;
const IMAGE_FILE_DLL: u16 = 0x2000;

const DIR_IMPORT: usize = 1;
const DIR_RESOURCE: usize = 2;
const DIR_DELAY_IMPORT: usize = 13;
const DIR_CLR: usize = 14;

/// Upper bounds that keep hostile files from making us allocate or loop.
const MAX_SECTIONS: usize = 96;
const MAX_IMPORT_DESCRIPTORS: usize = 4096;
const MAX_NAME_LEN: usize = 256;
const SCAN_WINDOW: usize = 4 << 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Machine {
    I386,
    Amd64,
    Arm,
    Arm64,
    Other(u16),
}

impl Machine {
    fn from_raw(v: u16) -> Self {
        match v {
            0x014c => Machine::I386,
            0x8664 => Machine::Amd64,
            0x01c0 | 0x01c4 => Machine::Arm,
            0xaa64 => Machine::Arm64,
            other => Machine::Other(other),
        }
    }

    /// Bitness as Windows reports it ("32-bit"/"64-bit").
    pub fn bits(&self) -> Option<u8> {
        match self {
            Machine::I386 | Machine::Arm => Some(32),
            Machine::Amd64 | Machine::Arm64 => Some(64),
            Machine::Other(_) => None,
        }
    }
}

impl fmt::Display for Machine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Machine::I386 => f.write_str("x86"),
            Machine::Amd64 => f.write_str("x64"),
            Machine::Arm => f.write_str("ARM"),
            Machine::Arm64 => f.write_str("ARM64"),
            Machine::Other(v) => write!(f, "machine 0x{v:04x}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeFormat {
    Pe32,
    Pe32Plus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Subsystem {
    Native,
    WindowsGui,
    WindowsCui,
    EfiApplication,
    EfiDriver,
    Other(u16),
}

impl Subsystem {
    fn from_raw(v: u16) -> Self {
        match v {
            1 => Subsystem::Native,
            2 => Subsystem::WindowsGui,
            3 => Subsystem::WindowsCui,
            10 => Subsystem::EfiApplication,
            11 | 12 => Subsystem::EfiDriver,
            other => Subsystem::Other(other),
        }
    }
}

impl fmt::Display for Subsystem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Subsystem::Native => f.write_str("native (driver)"),
            Subsystem::WindowsGui => f.write_str("GUI"),
            Subsystem::WindowsCui => f.write_str("console"),
            Subsystem::EfiApplication => f.write_str("EFI application"),
            Subsystem::EfiDriver => f.write_str("EFI driver"),
            Subsystem::Other(v) => write!(f, "subsystem {v}"),
        }
    }
}

/// Installer frameworks that can be recognised from the file contents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallerKind {
    InnoSetup,
    Nsis,
    InstallShield,
    WixBurn,
    Squirrel,
    WiseInstaller,
    SevenZipSfx,
}

impl InstallerKind {
    /// Command line switches that make the installer run without questions,
    /// used by `segrun install --silent`.
    pub fn silent_args(&self) -> &'static [&'static str] {
        match self {
            InstallerKind::InnoSetup => &["/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART"],
            InstallerKind::Nsis => &["/S"],
            InstallerKind::InstallShield => &["/s", "/v/qn"],
            InstallerKind::WixBurn => &["/quiet", "/norestart"],
            InstallerKind::Squirrel => &["--silent"],
            InstallerKind::WiseInstaller => &["/s"],
            InstallerKind::SevenZipSfx => &["-y"],
        }
    }
}

impl fmt::Display for InstallerKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            InstallerKind::InnoSetup => "Inno Setup",
            InstallerKind::Nsis => "NSIS (Nullsoft)",
            InstallerKind::InstallShield => "InstallShield",
            InstallerKind::WixBurn => "WiX Burn bundle",
            InstallerKind::Squirrel => "Squirrel",
            InstallerKind::WiseInstaller => "Wise",
            InstallerKind::SevenZipSfx => "7-Zip self-extractor",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    pub name: String,
    pub virtual_address: u32,
    pub virtual_size: u32,
    pub raw_offset: u32,
    pub raw_size: u32,
}

impl Section {
    fn contains_rva(&self, rva: u32) -> bool {
        let span = self.virtual_size.max(self.raw_size);
        rva >= self.virtual_address && (rva - self.virtual_address) < span
    }
}

/// Everything SegLinux needs to know about a PE image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeInfo {
    pub machine: Machine,
    pub format: PeFormat,
    pub subsystem: Subsystem,
    pub is_dll: bool,
    pub large_address_aware: bool,
    pub timestamp: u32,
    /// The image has a CLR header: it is a .NET assembly.
    pub dotnet: bool,
    /// Imported DLL names (lower case, sorted, de-duplicated, including
    /// delay-loaded imports).
    pub imports: Vec<String>,
    pub sections: Vec<Section>,
    /// Bytes appended after the last section (installer payloads live here).
    pub overlay_size: u64,
    pub installer: Option<InstallerKind>,
}

impl PeInfo {
    pub fn describe(&self) -> String {
        let bits = self
            .machine
            .bits()
            .map(|b| format!("{b}-bit "))
            .unwrap_or_default();
        let what = if self.is_dll { "library" } else { "program" };
        let mut s = format!(
            "{bits}{} {} {what} ({})",
            self.machine,
            self.subsystem,
            match self.format {
                PeFormat::Pe32 => "PE32",
                PeFormat::Pe32Plus => "PE32+",
            }
        );
        if self.dotnet {
            s.push_str(", .NET assembly");
        }
        if let Some(kind) = self.installer {
            s.push_str(&format!(", {kind} installer"));
        }
        s
    }

    /// DLLs, drivers and EFI binaries cannot be started directly.
    pub fn is_launchable(&self) -> bool {
        !self.is_dll
            && matches!(
                self.subsystem,
                Subsystem::WindowsGui | Subsystem::WindowsCui
            )
    }

    pub fn imports_dll(&self, name: &str) -> bool {
        let name = name.to_ascii_lowercase();
        self.imports.contains(&name)
    }

    pub fn imports_matching(&self, pred: impl Fn(&str) -> bool) -> bool {
        self.imports.iter().any(|i| pred(i))
    }
}

struct Headers {
    machine: Machine,
    format: PeFormat,
    subsystem: Subsystem,
    characteristics: u16,
    timestamp: u32,
    image_base: u64,
    size_of_headers: u32,
    dirs: Vec<(u32, u32)>,
    sections: Vec<Section>,
}

/// Parse a file that starts with `MZ`. Returns `Ok(None)` when it is a DOS
/// executable without a (valid) PE header.
pub(crate) fn parse<R: Read + Seek>(src: &mut Source<'_, R>) -> io::Result<Option<PeInfo>> {
    let Some(h) = parse_headers(src)? else {
        return Ok(None);
    };

    let imports = read_imports(src, &h)?;
    let dotnet = h
        .dirs
        .get(DIR_CLR)
        .is_some_and(|&(rva, size)| rva != 0 && size != 0);

    let end_of_image = h
        .sections
        .iter()
        .map(|s| u64::from(s.raw_offset) + u64::from(s.raw_size))
        .max()
        .unwrap_or(u64::from(h.size_of_headers));
    let overlay_size = src.len().saturating_sub(end_of_image);

    let installer = detect_installer(src, &h, end_of_image, overlay_size)?;

    Ok(Some(PeInfo {
        machine: h.machine,
        format: h.format,
        subsystem: h.subsystem,
        is_dll: h.characteristics & IMAGE_FILE_DLL != 0,
        large_address_aware: h.characteristics & IMAGE_FILE_LARGE_ADDRESS_AWARE != 0,
        timestamp: h.timestamp,
        dotnet,
        imports,
        sections: h.sections,
        overlay_size,
        installer,
    }))
}

fn parse_headers<R: Read + Seek>(src: &mut Source<'_, R>) -> io::Result<Option<Headers>> {
    let Some(dos) = src.read_exact_at(0, 64)? else {
        return Ok(None);
    };
    let Some(e_lfanew) = u32_at(&dos, 0x3c) else {
        return Ok(None);
    };
    let pe_off = u64::from(e_lfanew);
    // signature (4) + COFF header (20)
    let Some(coff) = src.read_exact_at(pe_off, 24)? else {
        return Ok(None);
    };
    if &coff[..4] != b"PE\0\0" {
        return Ok(None);
    }
    let machine = Machine::from_raw(u16_at(&coff, 4).unwrap_or(0));
    let n_sections = usize::from(u16_at(&coff, 6).unwrap_or(0)).min(MAX_SECTIONS);
    let timestamp = u32_at(&coff, 8).unwrap_or(0);
    let opt_size = usize::from(u16_at(&coff, 20).unwrap_or(0));
    let characteristics = u16_at(&coff, 22).unwrap_or(0);

    let opt_off = pe_off + 24;
    let opt = src.read_at(opt_off, opt_size)?;
    let (format, dir_count_off, dirs_off, image_base) = match u16_at(&opt, 0) {
        Some(0x10b) => (
            PeFormat::Pe32,
            92,
            96,
            u64::from(u32_at(&opt, 28).unwrap_or(0)),
        ),
        Some(0x20b) => (PeFormat::Pe32Plus, 108, 112, u64_at(&opt, 24).unwrap_or(0)),
        _ => return Ok(None),
    };
    let subsystem = Subsystem::from_raw(u16_at(&opt, 68).unwrap_or(0));
    let size_of_headers = u32_at(&opt, 60).unwrap_or(0);
    let dir_count = u32_at(&opt, dir_count_off).unwrap_or(0).min(16) as usize;
    let dirs = (0..dir_count)
        .map_while(|i| {
            let off = dirs_off + i * 8;
            Some((u32_at(&opt, off)?, u32_at(&opt, off + 4)?))
        })
        .collect();

    let sec_table = src.read_at(opt_off + opt_size as u64, n_sections * 40)?;
    let (headers, _) = sec_table.as_chunks::<40>();
    let sections = headers
        .iter()
        .map(|s| Section {
            name: String::from_utf8_lossy(&s[..8])
                .trim_end_matches('\0')
                .to_string(),
            virtual_size: u32_at(s, 8).unwrap_or(0),
            virtual_address: u32_at(s, 12).unwrap_or(0),
            raw_size: u32_at(s, 16).unwrap_or(0),
            raw_offset: u32_at(s, 20).unwrap_or(0),
        })
        .collect();

    Ok(Some(Headers {
        machine,
        format,
        subsystem,
        characteristics,
        timestamp,
        image_base,
        size_of_headers,
        dirs,
        sections,
    }))
}

impl Headers {
    /// Translate a relative virtual address to a file offset.
    fn rva_to_offset(&self, rva: u32) -> Option<u64> {
        if rva < self.size_of_headers {
            return Some(u64::from(rva));
        }
        let s = self.sections.iter().find(|s| s.contains_rva(rva))?;
        let delta = rva - s.virtual_address;
        // Bytes beyond the raw data are zero-filled at load time and do not
        // exist in the file.
        (delta < s.raw_size).then(|| u64::from(s.raw_offset) + u64::from(delta))
    }

    fn dir(&self, index: usize) -> Option<(u32, u32)> {
        self.dirs
            .get(index)
            .copied()
            .filter(|&(rva, size)| rva != 0 && size != 0)
    }
}

fn read_cstr<R: Read + Seek>(
    src: &mut Source<'_, R>,
    h: &Headers,
    rva: u32,
) -> io::Result<Option<String>> {
    let Some(off) = h.rva_to_offset(rva) else {
        return Ok(None);
    };
    let buf = src.read_at(off, MAX_NAME_LEN)?;
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    let name = &buf[..end];
    if name.is_empty() || !name.iter().all(|b| b.is_ascii_graphic()) {
        return Ok(None);
    }
    Ok(Some(String::from_utf8_lossy(name).to_ascii_lowercase()))
}

fn read_imports<R: Read + Seek>(src: &mut Source<'_, R>, h: &Headers) -> io::Result<Vec<String>> {
    let mut names = BTreeSet::new();
    // IMAGE_IMPORT_DESCRIPTOR: 20 bytes, DLL name RVA at offset 12.
    if let Some((rva, _)) = h.dir(DIR_IMPORT) {
        read_descriptor_table(src, h, rva, 20, 12, &mut names)?;
    }
    // IMAGE_DELAYLOAD_DESCRIPTOR: 32 bytes, DLL name RVA at offset 4.
    if let Some((rva, _)) = h.dir(DIR_DELAY_IMPORT) {
        read_descriptor_table(src, h, rva, 32, 4, &mut names)?;
    }
    Ok(names.into_iter().collect())
}

/// Walk a null-terminated table of fixed-size descriptors and collect the
/// DLL name each one points to.
fn read_descriptor_table<R: Read + Seek>(
    src: &mut Source<'_, R>,
    h: &Headers,
    table_rva: u32,
    entry_size: usize,
    name_field: usize,
    out: &mut BTreeSet<String>,
) -> io::Result<()> {
    let Some(table_off) = h.rva_to_offset(table_rva) else {
        return Ok(());
    };
    for i in 0..MAX_IMPORT_DESCRIPTORS {
        let Some(entry) = src.read_exact_at(table_off + (i * entry_size) as u64, entry_size)?
        else {
            break;
        };
        if entry.iter().all(|&b| b == 0) {
            break;
        }
        let Some(name_rva) = u32_at(&entry, name_field) else {
            break;
        };
        let name_rva = rebase(name_rva, h);
        if let Some(name) = read_cstr(src, h, name_rva)? {
            out.insert(name);
        }
    }
    Ok(())
}

/// Very old linkers stored virtual addresses where an RVA was expected
/// (delay-load descriptors without the RVA attribute); turn those back into
/// RVAs.
fn rebase(value: u32, h: &Headers) -> u32 {
    let base = h.image_base;
    if base != 0 && u64::from(value) >= base && h.rva_to_offset(value).is_none() {
        (u64::from(value) - base) as u32
    } else {
        value
    }
}

fn detect_installer<R: Read + Seek>(
    src: &mut Source<'_, R>,
    h: &Headers,
    end_of_image: u64,
    overlay_size: u64,
) -> io::Result<Option<InstallerKind>> {
    if h.sections.iter().any(|s| s.name == ".wixburn") {
        return Ok(Some(InstallerKind::WixBurn));
    }
    // Installer payloads are appended after the image...
    if overlay_size > 0 {
        let chunk = src.read_at(end_of_image, SCAN_WINDOW)?;
        if let Some(kind) = sig::scan_installer(&chunk) {
            return Ok(Some(kind));
        }
    }
    // ...and their version resources / manifests name the framework.
    if let Some((rva, size)) = h.dir(DIR_RESOURCE)
        && let Some(off) = h.rva_to_offset(rva)
    {
        let chunk = src.read_at(off, (size as usize).min(SCAN_WINDOW))?;
        if let Some(kind) = sig::scan_installer(&chunk) {
            return Ok(Some(kind));
        }
    }
    Ok(None)
}
