//! Magic numbers and installer fingerprints.

use crate::pe::InstallerKind;

/// OLE2 compound file header, used by MSI/MSP packages.
pub(crate) const OLE_MAGIC: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];

/// `HeaderSize` field of a shell link: always 0x0000004C.
pub(crate) const LNK_HEADER_SIZE: [u8; 4] = [0x4C, 0x00, 0x00, 0x00];

/// `LinkCLSID` of a shell link: {00021401-0000-0000-C000-000000000046}.
pub(crate) const LNK_CLSID: [u8; 16] = [
    0x01, 0x14, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46,
];

/// 7-Zip archive signature, found in the overlay of 7z SFX installers.
const SEVEN_ZIP: &[u8] = &[b'7', b'z', 0xBC, 0xAF, 0x27, 0x1C];

/// Fingerprints are checked in order; the first match wins.
const FINGERPRINTS: &[(InstallerKind, &[&str])] = &[
    (
        InstallerKind::InnoSetup,
        &["Inno Setup Setup Data", "built with Inno Setup", "rDlPtS"],
    ),
    (
        InstallerKind::Nsis,
        &["NullsoftInst", "Nullsoft Install System", "Nullsoft.NSIS"],
    ),
    (InstallerKind::InstallShield, &["InstallShield"]),
    (
        InstallerKind::Squirrel,
        &["SquirrelSetup", "Squirrel.Windows"],
    ),
    (InstallerKind::WiseInstaller, &["Wise Installation Wizard"]),
];

/// Look for installer fingerprints in a chunk of the file. Strings are
/// matched both as ASCII and as UTF-16LE, because version resources store
/// text as UTF-16.
pub(crate) fn scan_installer(chunk: &[u8]) -> Option<InstallerKind> {
    for (kind, needles) in FINGERPRINTS {
        for needle in *needles {
            if contains(chunk, needle.as_bytes()) || contains(chunk, &utf16le(needle)) {
                return Some(*kind);
            }
        }
    }
    if contains(chunk, SEVEN_ZIP) {
        return Some(InstallerKind::SevenZipSfx);
    }
    None
}

fn utf16le(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

pub(crate) fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() || needle.len() > haystack.len() {
        return needle.is_empty();
    }
    let first = needle[0];
    let last_start = haystack.len() - needle.len();
    let mut i = 0;
    while i <= last_start {
        match haystack[i..=last_start].iter().position(|&b| b == first) {
            Some(p) => {
                let start = i + p;
                if &haystack[start..start + needle.len()] == needle {
                    return true;
                }
                i = start + 1;
            }
            None => return false,
        }
    }
    false
}
