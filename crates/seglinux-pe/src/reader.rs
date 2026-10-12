//! Bounded random access reads over any `Read + Seek`.

use std::io::{self, Read, Seek, SeekFrom};

/// Wraps a reader and only ever hands out short, clamped slices of it.
pub(crate) struct Source<'a, R: Read + Seek> {
    inner: &'a mut R,
    len: u64,
}

impl<'a, R: Read + Seek> Source<'a, R> {
    pub(crate) fn new(inner: &'a mut R) -> io::Result<Self> {
        let len = inner.seek(SeekFrom::End(0))?;
        Ok(Self { inner, len })
    }

    pub(crate) fn len(&self) -> u64 {
        self.len
    }

    /// Read up to `len` bytes at `offset`. Returns fewer bytes when the file
    /// ends early and an empty buffer when `offset` is past the end.
    pub(crate) fn read_at(&mut self, offset: u64, len: usize) -> io::Result<Vec<u8>> {
        if offset >= self.len {
            return Ok(Vec::new());
        }
        let available = (self.len - offset).min(len as u64) as usize;
        let mut buf = vec![0u8; available];
        self.inner.seek(SeekFrom::Start(offset))?;
        self.inner.read_exact(&mut buf)?;
        Ok(buf)
    }

    /// Read exactly `len` bytes or return `None` if the file is too short.
    pub(crate) fn read_exact_at(&mut self, offset: u64, len: usize) -> io::Result<Option<Vec<u8>>> {
        let buf = self.read_at(offset, len)?;
        Ok((buf.len() == len).then_some(buf))
    }
}

pub(crate) fn u16_at(buf: &[u8], off: usize) -> Option<u16> {
    buf.get(off..off + 2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
}

pub(crate) fn u32_at(buf: &[u8], off: usize) -> Option<u32> {
    buf.get(off..off + 4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

pub(crate) fn u64_at(buf: &[u8], off: usize) -> Option<u64> {
    buf.get(off..off + 8).map(|b| {
        let mut a = [0u8; 8];
        a.copy_from_slice(b);
        u64::from_le_bytes(a)
    })
}
