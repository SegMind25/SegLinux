use super::*;

/// Builds small but structurally valid PE images for tests.
struct PeBuilder {
    machine: u16,
    pe32plus: bool,
    subsystem: u16,
    characteristics: u16,
    dotnet: bool,
    imports: Vec<&'static str>,
    delay_imports: Vec<&'static str>,
    section_name: &'static str,
    overlay: Vec<u8>,
}

impl Default for PeBuilder {
    fn default() -> Self {
        Self {
            machine: 0x8664,
            pe32plus: true,
            subsystem: 2,
            characteristics: 0x0022,
            dotnet: false,
            imports: vec!["KERNEL32.dll"],
            delay_imports: vec![],
            section_name: ".rdata",
            overlay: vec![],
        }
    }
}

const SECTION_RVA: u32 = 0x1000;
const SECTION_RAW: u32 = 0x200;
const SECTION_SIZE: u32 = 0x400;

impl PeBuilder {
    fn build(&self) -> Vec<u8> {
        let mut f = vec![0u8; (SECTION_RAW + SECTION_SIZE) as usize];
        let put16 =
            |f: &mut Vec<u8>, off: usize, v: u16| f[off..off + 2].copy_from_slice(&v.to_le_bytes());
        let put32 =
            |f: &mut Vec<u8>, off: usize, v: u32| f[off..off + 4].copy_from_slice(&v.to_le_bytes());

        // DOS header
        f[0..2].copy_from_slice(b"MZ");
        put32(&mut f, 0x3c, 0x40);

        // PE signature + COFF header
        let pe = 0x40;
        f[pe..pe + 4].copy_from_slice(b"PE\0\0");
        put16(&mut f, pe + 4, self.machine);
        put16(&mut f, pe + 6, 1);
        put32(&mut f, pe + 8, 0x5f00_0000);
        let (opt_size, dirs_off, count_off, magic) = if self.pe32plus {
            (112 + 128, 112, 108, 0x20b)
        } else {
            (96 + 128, 96, 92, 0x10b)
        };
        put16(&mut f, pe + 20, opt_size as u16);
        put16(&mut f, pe + 22, self.characteristics);

        // Optional header
        let opt = pe + 24;
        put16(&mut f, opt, magic);
        if self.pe32plus {
            f[opt + 24..opt + 32].copy_from_slice(&0x1_4000_0000u64.to_le_bytes());
        } else {
            put32(&mut f, opt + 28, 0x40_0000);
        }
        put32(&mut f, opt + 60, SECTION_RAW); // SizeOfHeaders
        put16(&mut f, opt + 68, self.subsystem);
        put32(&mut f, opt + count_off, 16);

        // Section data layout (RVAs):
        //   0x1000 import descriptors, 0x1100 delay descriptors,
        //   0x1200 CLR header, 0x1300.. DLL name strings
        let raw = |rva: u32| (rva - SECTION_RVA + SECTION_RAW) as usize;
        let mut name_rva = 0x1300u32;
        let mut write_name = |f: &mut Vec<u8>, name: &str| {
            let at = raw(name_rva);
            f[at..at + name.len()].copy_from_slice(name.as_bytes());
            let rva = name_rva;
            name_rva += 0x20;
            rva
        };

        let dir = |i: usize| opt + dirs_off + i * 8;
        if !self.imports.is_empty() {
            put32(&mut f, dir(1), 0x1000);
            put32(&mut f, dir(1) + 4, 20 * (self.imports.len() as u32 + 1));
            for (i, name) in self.imports.iter().enumerate() {
                let rva = write_name(&mut f, name);
                put32(&mut f, raw(0x1000) + i * 20 + 12, rva);
                put32(&mut f, raw(0x1000) + i * 20 + 16, 0x1280); // FirstThunk
            }
        }
        if !self.delay_imports.is_empty() {
            put32(&mut f, dir(13), 0x1100);
            put32(
                &mut f,
                dir(13) + 4,
                32 * (self.delay_imports.len() as u32 + 1),
            );
            for (i, name) in self.delay_imports.iter().enumerate() {
                let rva = write_name(&mut f, name);
                put32(&mut f, raw(0x1100) + i * 32, 1);
                put32(&mut f, raw(0x1100) + i * 32 + 4, rva);
            }
        }
        if self.dotnet {
            put32(&mut f, dir(14), 0x1200);
            put32(&mut f, dir(14) + 4, 72);
        }

        // Section header
        let sh = opt + opt_size;
        let mut name = [0u8; 8];
        name[..self.section_name.len()].copy_from_slice(self.section_name.as_bytes());
        f[sh..sh + 8].copy_from_slice(&name);
        put32(&mut f, sh + 8, SECTION_SIZE);
        put32(&mut f, sh + 12, SECTION_RVA);
        put32(&mut f, sh + 16, SECTION_SIZE);
        put32(&mut f, sh + 20, SECTION_RAW);

        f.extend_from_slice(&self.overlay);
        f
    }
}

fn pe(kind: FileKind) -> PeInfo {
    match kind {
        FileKind::Pe(info) => *info,
        other => panic!("expected a PE image, got {other:?}"),
    }
}

#[test]
fn x64_gui_program_with_imports() {
    let bytes = PeBuilder {
        imports: vec!["KERNEL32.dll", "d3d11.dll", "USER32.dll"],
        delay_imports: vec!["XINPUT1_3.dll"],
        ..Default::default()
    }
    .build();
    let info = pe(analyze_bytes(&bytes, Some("exe")));
    assert_eq!(info.machine, Machine::Amd64);
    assert_eq!(info.format, PeFormat::Pe32Plus);
    assert_eq!(info.subsystem, Subsystem::WindowsGui);
    assert!(!info.is_dll);
    assert!(info.large_address_aware);
    assert!(!info.dotnet);
    assert_eq!(
        info.imports,
        vec!["d3d11.dll", "kernel32.dll", "user32.dll", "xinput1_3.dll"]
    );
    assert!(info.imports_dll("D3D11.DLL"));
    assert_eq!(info.overlay_size, 0);
    assert_eq!(info.installer, None);
    assert!(info.is_launchable());
    assert_eq!(info.describe(), "64-bit x64 GUI program (PE32+)");
}

#[test]
fn x86_console_program() {
    let bytes = PeBuilder {
        machine: 0x014c,
        pe32plus: false,
        subsystem: 3,
        characteristics: 0x0102,
        imports: vec!["msvcrt.dll"],
        ..Default::default()
    }
    .build();
    let info = pe(analyze_bytes(&bytes, None));
    assert_eq!(info.machine, Machine::I386);
    assert_eq!(info.machine.bits(), Some(32));
    assert_eq!(info.format, PeFormat::Pe32);
    assert_eq!(info.subsystem, Subsystem::WindowsCui);
    assert!(!info.large_address_aware);
    assert_eq!(info.imports, vec!["msvcrt.dll"]);
}

#[test]
fn dotnet_assembly() {
    let bytes = PeBuilder {
        machine: 0x014c,
        pe32plus: false,
        dotnet: true,
        imports: vec!["mscoree.dll"],
        ..Default::default()
    }
    .build();
    let info = pe(analyze_bytes(&bytes, None));
    assert!(info.dotnet);
    assert!(info.describe().contains(".NET"));
}

#[test]
fn dll_is_not_launchable() {
    let bytes = PeBuilder {
        characteristics: 0x2022,
        ..Default::default()
    }
    .build();
    let kind = analyze_bytes(&bytes, Some("dll"));
    assert!(!kind.is_launchable());
    assert!(pe(kind).is_dll);
}

#[test]
fn driver_is_not_launchable() {
    let bytes = PeBuilder {
        subsystem: 1,
        imports: vec!["ntoskrnl.exe"],
        ..Default::default()
    }
    .build();
    let info = pe(analyze_bytes(&bytes, Some("sys")));
    assert_eq!(info.subsystem, Subsystem::Native);
    assert!(!info.is_launchable());
}

#[test]
fn arm64_program() {
    let bytes = PeBuilder {
        machine: 0xaa64,
        ..Default::default()
    }
    .build();
    assert_eq!(pe(analyze_bytes(&bytes, None)).machine, Machine::Arm64);
}

#[test]
fn nsis_installer_from_overlay() {
    let mut overlay = vec![0u8; 16];
    overlay.extend_from_slice(b"\xef\xbe\xad\xdeNullsoftInst");
    let bytes = PeBuilder {
        overlay,
        ..Default::default()
    }
    .build();
    let info = pe(analyze_bytes(&bytes, None));
    assert_eq!(info.installer, Some(InstallerKind::Nsis));
    assert_eq!(info.overlay_size, 32);
    assert_eq!(InstallerKind::Nsis.silent_args(), &["/S"]);
}

#[test]
fn inno_setup_from_utf16_overlay_text() {
    let text: Vec<u8> = "This installation was built with Inno Setup."
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    let bytes = PeBuilder {
        overlay: text,
        ..Default::default()
    }
    .build();
    assert_eq!(
        pe(analyze_bytes(&bytes, None)).installer,
        Some(InstallerKind::InnoSetup)
    );
}

#[test]
fn wix_burn_bundle_from_section_name() {
    let bytes = PeBuilder {
        section_name: ".wixburn",
        imports: vec![],
        ..Default::default()
    }
    .build();
    assert_eq!(
        pe(analyze_bytes(&bytes, None)).installer,
        Some(InstallerKind::WixBurn)
    );
}

#[test]
fn seven_zip_sfx() {
    let mut overlay = b"7z".to_vec();
    overlay.extend_from_slice(&[0xBC, 0xAF, 0x27, 0x1C, 0, 4]);
    let bytes = PeBuilder {
        overlay,
        ..Default::default()
    }
    .build();
    assert_eq!(
        pe(analyze_bytes(&bytes, None)).installer,
        Some(InstallerKind::SevenZipSfx)
    );
}

#[test]
fn dos_program_without_pe_header() {
    let mut bytes = vec![0u8; 128];
    bytes[..2].copy_from_slice(b"MZ");
    bytes[0x3c] = 0x40;
    assert_eq!(analyze_bytes(&bytes, Some("exe")), FileKind::Dos);
}

#[test]
fn truncated_and_hostile_headers_do_not_panic() {
    let good = PeBuilder::default().build();
    for len in 0..good.len() {
        let _ = analyze_bytes(&good[..len], Some("exe"));
    }
    // e_lfanew pointing far past the end of the file.
    let mut bytes = good.clone();
    bytes[0x3c..0x40].copy_from_slice(&u32::MAX.to_le_bytes());
    assert_eq!(analyze_bytes(&bytes, None), FileKind::Dos);
    // Import directory pointing outside every section.
    let mut bytes = good.clone();
    let dir1 = 0x40 + 24 + 112 + 8;
    bytes[dir1..dir1 + 4].copy_from_slice(&0x7fff_0000u32.to_le_bytes());
    assert!(pe(analyze_bytes(&bytes, None)).imports.is_empty());
}

#[test]
fn msi_and_ole_documents() {
    let mut bytes = sig::OLE_MAGIC.to_vec();
    bytes.resize(512, 0);
    assert_eq!(analyze_bytes(&bytes, Some("msi")), FileKind::Msi);
    assert_eq!(analyze_bytes(&bytes, Some(".MSP")), FileKind::Msi);
    assert_eq!(analyze_bytes(&bytes, Some("doc")), FileKind::OleCompound);
    assert!(!FileKind::OleCompound.is_launchable());
}

#[test]
fn shell_link() {
    let mut bytes = sig::LNK_HEADER_SIZE.to_vec();
    bytes.extend_from_slice(&sig::LNK_CLSID);
    bytes.resize(76, 0);
    assert_eq!(analyze_bytes(&bytes, Some("lnk")), FileKind::Shortcut);
}

#[test]
fn batch_scripts_by_extension() {
    assert_eq!(
        analyze_bytes(b"@echo off\r\n", Some("bat")),
        FileKind::Batch
    );
    assert_eq!(
        analyze_bytes(b"@echo off\r\n", Some("CMD")),
        FileKind::Batch
    );
    assert_eq!(
        analyze_bytes(b"@echo off\r\n", Some("txt")),
        FileKind::Unknown
    );
}

#[test]
fn substring_search() {
    assert!(sig::contains(b"abcNullsoftInstxyz", b"NullsoftInst"));
    assert!(sig::contains(b"aaab", b"aab"));
    assert!(!sig::contains(b"Nullsoft", b"NullsoftInst"));
    assert!(sig::contains(b"", b""));
}
