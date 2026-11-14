//! ELF64 parsing for the kernel image: static x86_64 executables only.

pub const PF_X: u32 = 1;
pub const PF_W: u32 = 2;

const PT_LOAD: u32 = 1;
const ET_EXEC: u16 = 2;
const ET_DYN: u16 = 3;
const EM_X86_64: u16 = 62;

/// Lowest address the kernel may be linked at: the top 2 GiB.
const KERNEL_SPACE: u64 = 0xffff_ffff_8000_0000;

#[derive(Clone, Copy)]
pub struct Segment {
    pub vaddr: u64,
    pub offset: u64,
    pub filesz: u64,
    pub memsz: u64,
    pub flags: u32,
}

pub struct Elf<'a> {
    data: &'a [u8],
    pub entry: u64,
    phoff: usize,
    phnum: usize,
}

fn u16_at(d: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([d[o], d[o + 1]])
}
fn u32_at(d: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(d[o..o + 4].try_into().unwrap())
}
fn u64_at(d: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(d[o..o + 8].try_into().unwrap())
}

impl<'a> Elf<'a> {
    pub fn parse(data: &'a [u8]) -> Result<Self, &'static str> {
        if data.len() < 64 || data[0..4] != *b"\x7fELF" {
            return Err("not an ELF file (bad magic)");
        }
        if data[4] != 2 {
            return Err("not a 64-bit ELF file");
        }
        if data[5] != 1 {
            return Err("not a little-endian ELF file");
        }
        if data[6] != 1 {
            return Err("unknown ELF version");
        }
        match u16_at(data, 16) {
            ET_EXEC => {}
            ET_DYN => return Err("kernel is position independent (PIE); link it static"),
            _ => return Err("ELF file is not an executable"),
        }
        if u16_at(data, 18) != EM_X86_64 {
            return Err("ELF file is not for x86_64");
        }
        let entry = u64_at(data, 24);
        let phoff = u64_at(data, 32);
        let phentsize = u16_at(data, 54) as u64;
        let phnum = u16_at(data, 56) as u64;
        if phentsize != 56 {
            return Err("bad ELF program header size");
        }
        if phnum == 0 {
            return Err("ELF file has no program headers");
        }
        match phoff.checked_add(phnum * 56) {
            Some(end) if end <= data.len() as u64 => {}
            _ => return Err("ELF program headers lie outside the file"),
        }
        let elf = Elf {
            data,
            entry,
            phoff: phoff as usize,
            phnum: phnum as usize,
        };
        let mut any = false;
        let mut entry_ok = false;
        for s in elf.segments() {
            any = true;
            if s.filesz > s.memsz {
                return Err("ELF segment file size exceeds its memory size");
            }
            match s.offset.checked_add(s.filesz) {
                Some(end) if end <= data.len() as u64 => {}
                _ => return Err("ELF segment data lies outside the file"),
            }
            match s.vaddr.checked_add(s.memsz) {
                Some(_) if s.vaddr >= KERNEL_SPACE => {}
                _ => return Err("ELF segment not in the top 2 GiB (link at 0xffffffff80000000)"),
            }
            if s.memsz > 1 << 30 {
                return Err("ELF segment larger than 1 GiB");
            }
            if s.flags & PF_X != 0 && entry >= s.vaddr && entry < s.vaddr + s.memsz {
                entry_ok = true;
            }
        }
        if !any {
            return Err("ELF file has no loadable segments");
        }
        if !entry_ok {
            return Err("ELF entry point is not inside an executable segment");
        }
        Ok(elf)
    }

    /// The PT_LOAD segments (empty ones skipped).
    pub fn segments(&self) -> impl Iterator<Item = Segment> + '_ {
        (0..self.phnum).filter_map(move |i| {
            let p = &self.data[self.phoff + i * 56..self.phoff + (i + 1) * 56];
            if u32_at(p, 0) != PT_LOAD || u64_at(p, 40) == 0 {
                return None;
            }
            Some(Segment {
                flags: u32_at(p, 4),
                offset: u64_at(p, 8),
                vaddr: u64_at(p, 16),
                filesz: u64_at(p, 32),
                memsz: u64_at(p, 40),
            })
        })
    }

    pub fn bytes(&self, s: &Segment) -> &'a [u8] {
        &self.data[s.offset as usize..(s.offset + s.filesz) as usize]
    }
}
