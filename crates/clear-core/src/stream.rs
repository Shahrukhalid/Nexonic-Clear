//! Random-access sources and output plans, so audio and video can be cleaned
//! from a file without loading the media data into memory.
//!
//! A cleaner produces a [`Plan`]: a list of new bytes and ranges copied from
//! the source. The same plan writes to memory or streams to a file.

use crate::{malformed, Result};
use std::io::{Read, Seek, SeekFrom, Write};
use std::sync::Mutex;

/// Largest single structure (box, chunk or tag) read into memory.
pub(crate) const MAX_STRUCT: u64 = 64 * 1024 * 1024;

pub(crate) trait Src: Sync {
    fn len(&self) -> u64;
    fn read_at(&self, off: u64, buf: &mut [u8]) -> Result<()>;

    fn bytes(&self, off: u64, len: u64) -> Result<Vec<u8>> {
        if len > MAX_STRUCT {
            return Err(malformed("a metadata structure is too large"));
        }
        let end = off.checked_add(len).filter(|&e| e <= self.len());
        if end.is_none() {
            return Err(malformed("structure runs past the end of the file"));
        }
        let mut v = vec![0; len as usize];
        self.read_at(off, &mut v)?;
        Ok(v)
    }
}

/// An in-memory source.
pub(crate) struct Mem<'a>(pub &'a [u8]);

impl Src for Mem<'_> {
    fn len(&self) -> u64 {
        self.0.len() as u64
    }
    fn read_at(&self, off: u64, buf: &mut [u8]) -> Result<()> {
        let s = usize::try_from(off).map_err(|_| malformed("offset out of range"))?;
        let e = s
            .checked_add(buf.len())
            .filter(|&e| e <= self.0.len())
            .ok_or_else(|| malformed("structure runs past the end of the file"))?;
        buf.copy_from_slice(&self.0[s..e]);
        Ok(())
    }
}

/// A file read with positioned reads.
pub(crate) struct FileSrc {
    file: Mutex<std::fs::File>,
    len: u64,
}

impl FileSrc {
    pub fn open(path: &std::path::Path) -> std::io::Result<Self> {
        let file = std::fs::File::open(path)?;
        let len = file.metadata()?.len();
        Ok(Self {
            file: Mutex::new(file),
            len,
        })
    }
}

impl Src for FileSrc {
    fn len(&self) -> u64 {
        self.len
    }
    fn read_at(&self, off: u64, buf: &mut [u8]) -> Result<()> {
        if off
            .checked_add(buf.len() as u64)
            .is_none_or(|e| e > self.len)
        {
            return Err(malformed("structure runs past the end of the file"));
        }
        let mut f = self.file.lock().unwrap_or_else(|e| e.into_inner());
        f.seek(SeekFrom::Start(off))
            .and_then(|_| f.read_exact(buf))
            .map_err(|e| crate::Error::Malformed(format!("read failed: {e}")))
    }
}

#[derive(Debug, Clone)]
pub(crate) enum Piece {
    Bytes(Vec<u8>),
    /// Copy `len` bytes starting at `off` in the source.
    Copy(u64, u64),
}

#[derive(Debug, Default, Clone)]
pub(crate) struct Plan(pub Vec<Piece>);

impl Plan {
    pub fn bytes(&mut self, b: Vec<u8>) {
        if !b.is_empty() {
            self.0.push(Piece::Bytes(b));
        }
    }
    pub fn copy(&mut self, off: u64, len: u64) {
        if len == 0 {
            return;
        }
        if let Some(Piece::Copy(o, l)) = self.0.last_mut() {
            if *o + *l == off {
                *l += len;
                return;
            }
        }
        self.0.push(Piece::Copy(off, len));
    }
    pub fn append(&mut self, other: Plan) {
        for p in other.0 {
            match p {
                Piece::Bytes(b) => self.bytes(b),
                Piece::Copy(o, l) => self.copy(o, l),
            }
        }
    }
    pub fn len(&self) -> u64 {
        self.0
            .iter()
            .map(|p| match p {
                Piece::Bytes(b) => b.len() as u64,
                Piece::Copy(_, l) => *l,
            })
            .sum()
    }
    pub fn write(&self, src: &dyn Src, w: &mut dyn Write) -> Result<()> {
        let io = |e: std::io::Error| crate::Error::Malformed(format!("write failed: {e}"));
        let mut buf = vec![0u8; 1 << 20];
        for p in &self.0 {
            match p {
                Piece::Bytes(b) => w.write_all(b).map_err(io)?,
                Piece::Copy(o, l) => {
                    let mut done = 0;
                    while done < *l {
                        let n = (*l - done).min(buf.len() as u64) as usize;
                        src.read_at(o + done, &mut buf[..n])?;
                        w.write_all(&buf[..n]).map_err(io)?;
                        done += n as u64;
                    }
                }
            }
        }
        Ok(())
    }
    pub fn to_vec(&self, src: &dyn Src) -> Result<Vec<u8>> {
        let mut v = Vec::with_capacity(self.len() as usize);
        self.write(src, &mut v)?;
        Ok(v)
    }
}

/// Adds a source range to a digest as one length-prefixed part, streaming.
pub(crate) fn digest_range(d: &mut crate::Digest, src: &dyn Src, off: u64, len: u64) -> Result<()> {
    use sha2::Digest as _;
    d.0.update(len.to_le_bytes());
    let mut buf = vec![0u8; (len.min(1 << 20)) as usize];
    let mut done = 0;
    while done < len {
        let n = (len - done).min(buf.len() as u64) as usize;
        src.read_at(off + done, &mut buf[..n])?;
        d.0.update(&buf[..n]);
        done += n as u64;
    }
    Ok(())
}
