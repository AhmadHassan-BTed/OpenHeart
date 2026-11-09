use std::io::{self, Read, Write};

pub struct BinaryWriter<W: Write> {
    writer: W,
    written_bytes: usize,
}

impl<W: Write> BinaryWriter<W> {
    pub fn new(writer: W) -> Self {
        Self {
            writer,
            written_bytes: 0,
        }
    }

    pub fn written_bytes(&self) -> usize {
        self.written_bytes
    }

    pub fn write_u8(&mut self, val: u8) -> io::Result<()> {
        self.writer.write_all(&[val])?;
        self.written_bytes += 1;
        Ok(())
    }

    pub fn write_u16(&mut self, val: u16) -> io::Result<()> {
        let bytes = val.to_le_bytes();
        self.writer.write_all(&bytes)?;
        self.written_bytes += 2;
        Ok(())
    }

    pub fn write_u32(&mut self, val: u32) -> io::Result<()> {
        let bytes = val.to_le_bytes();
        self.writer.write_all(&bytes)?;
        self.written_bytes += 4;
        Ok(())
    }

    pub fn write_u64(&mut self, val: u64) -> io::Result<()> {
        let bytes = val.to_le_bytes();
        self.writer.write_all(&bytes)?;
        self.written_bytes += 8;
        Ok(())
    }

    pub fn write_bytes(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.writer.write_all(bytes)?;
        self.written_bytes += bytes.len();
        Ok(())
    }

    pub fn align_to(&mut self, alignment: usize) -> io::Result<()> {
        let remainder = self.written_bytes % alignment;
        if remainder != 0 {
            let padding = alignment - remainder;
            let zeros = vec![0u8; padding];
            self.write_bytes(&zeros)?;
        }
        Ok(())
    }
}

pub struct BinaryReader<R: Read> {
    reader: R,
    read_bytes: usize,
}

impl<R: Read> BinaryReader<R> {
    pub fn new(reader: R) -> Self {
        Self {
            reader,
            read_bytes: 0,
        }
    }

    pub fn read_bytes_count(&self) -> usize {
        self.read_bytes
    }

    pub fn read_u8(&mut self) -> io::Result<u8> {
        let mut buf = [0u8; 1];
        self.reader.read_exact(&mut buf)?;
        self.read_bytes += 1;
        Ok(buf[0])
    }

    pub fn read_u16(&mut self) -> io::Result<u16> {
        let mut buf = [0u8; 2];
        self.reader.read_exact(&mut buf)?;
        self.read_bytes += 2;
        Ok(u16::from_le_bytes(buf))
    }

    pub fn read_u32(&mut self) -> io::Result<u32> {
        let mut buf = [0u8; 4];
        self.reader.read_exact(&mut buf)?;
        self.read_bytes += 4;
        Ok(u32::from_le_bytes(buf))
    }

    pub fn read_u64(&mut self) -> io::Result<u64> {
        let mut buf = [0u8; 8];
        self.reader.read_exact(&mut buf)?;
        self.read_bytes += 8;
        Ok(u64::from_le_bytes(buf))
    }

    pub fn read_exact_bytes(&mut self, len: usize) -> io::Result<Vec<u8>> {
        let mut buf = vec![0u8; len];
        self.reader.read_exact(&mut buf)?;
        self.read_bytes += len;
        Ok(buf)
    }

    pub fn skip_alignment(&mut self, alignment: usize) -> io::Result<()> {
        let remainder = self.read_bytes % alignment;
        if remainder != 0 {
            let padding = alignment - remainder;
            let mut buf = vec![0u8; padding];
            self.reader.read_exact(&mut buf)?;
            self.read_bytes += padding;
        }
        Ok(())
    }
}

/// Builds an `InvalidData` error for malformed artifact bytes.
pub fn invalid_data<M: Into<String>>(msg: M) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.into())
}

/// Bounds-checked little-endian reader over an in-memory byte slice.
///
/// Every read verifies the remaining length first and returns `InvalidData` instead of
/// panicking, and element counts taken from a file are validated against the bytes that
/// remain before any allocation is sized from them.
#[derive(Debug, Clone)]
pub struct ByteCursor<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> ByteCursor<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    #[inline]
    pub fn position(&self) -> usize {
        self.pos
    }

    #[inline]
    pub fn remaining(&self) -> usize {
        self.buf.len() - self.pos
    }

    #[inline]
    pub fn is_at_end(&self) -> bool {
        self.pos == self.buf.len()
    }

    /// Moves to an absolute offset (which may equal the buffer length).
    pub fn seek(&mut self, pos: usize) -> io::Result<()> {
        if pos > self.buf.len() {
            return Err(invalid_data(format!(
                "seek to offset {} beyond end of {}-byte buffer",
                pos,
                self.buf.len()
            )));
        }
        self.pos = pos;
        Ok(())
    }

    /// Returns the next `len` bytes and advances past them.
    pub fn read_bytes(&mut self, len: usize) -> io::Result<&'a [u8]> {
        if len > self.remaining() {
            return Err(invalid_data(format!(
                "truncated data: need {} bytes at offset {}, only {} remain",
                len,
                self.pos,
                self.remaining()
            )));
        }
        let start = self.pos;
        self.pos += len;
        Ok(&self.buf[start..start + len])
    }

    pub fn skip(&mut self, len: usize) -> io::Result<()> {
        self.read_bytes(len).map(|_| ())
    }

    pub fn read_array<const N: usize>(&mut self) -> io::Result<[u8; N]> {
        let bytes = self.read_bytes(N)?;
        let mut out = [0u8; N];
        out.copy_from_slice(bytes);
        Ok(out)
    }

    pub fn read_u8(&mut self) -> io::Result<u8> {
        Ok(self.read_array::<1>()?[0])
    }

    pub fn read_u16(&mut self) -> io::Result<u16> {
        Ok(u16::from_le_bytes(self.read_array::<2>()?))
    }

    pub fn read_u32(&mut self) -> io::Result<u32> {
        Ok(u32::from_le_bytes(self.read_array::<4>()?))
    }

    pub fn read_u64(&mut self) -> io::Result<u64> {
        Ok(u64::from_le_bytes(self.read_array::<8>()?))
    }

    /// Skips zero padding up to the next multiple of `alignment` (absolute offset).
    pub fn align_to(&mut self, alignment: usize) -> io::Result<()> {
        if alignment == 0 {
            return Ok(());
        }
        let rem = self.pos % alignment;
        if rem != 0 {
            self.skip(alignment - rem)?;
        }
        Ok(())
    }

    /// Validates that `count` elements of `elem_size` bytes fit in the remaining input and
    /// returns `count` as `usize`. Call before `Vec::with_capacity(count)` on file-supplied
    /// counts so a crafted header cannot trigger a huge allocation.
    pub fn checked_count(&self, count: u64, elem_size: usize) -> io::Result<usize> {
        let count_usize = usize::try_from(count)
            .map_err(|_| invalid_data(format!("element count {} overflows usize", count)))?;
        let needed = count_usize.checked_mul(elem_size).ok_or_else(|| {
            invalid_data(format!(
                "element count {} x {} bytes overflows",
                count, elem_size
            ))
        })?;
        if needed > self.remaining() {
            return Err(invalid_data(format!(
                "element count {} x {} bytes at offset {} exceeds the {} bytes remaining",
                count,
                elem_size,
                self.pos,
                self.remaining()
            )));
        }
        Ok(count_usize)
    }

    /// Reads `count` little-endian u32 values after validating the length.
    pub fn read_u32_vec(&mut self, count: u64) -> io::Result<Vec<u32>> {
        let n = self.checked_count(count, 4)?;
        let bytes = self.read_bytes(n * 4)?;
        Ok(bytes
            .chunks_exact(4)
            .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect())
    }

    /// Reads `count` little-endian u64 values after validating the length.
    pub fn read_u64_vec(&mut self, count: u64) -> io::Result<Vec<u64>> {
        let n = self.checked_count(count, 8)?;
        let bytes = self.read_bytes(n * 8)?;
        Ok(bytes
            .chunks_exact(8)
            .map(|c| u64::from_le_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]]))
            .collect())
    }
}

/// Appends zero bytes to `buf` until its length is a multiple of `alignment`.
pub fn pad_to_alignment(buf: &mut Vec<u8>, alignment: usize) {
    if alignment == 0 {
        return;
    }
    let rem = buf.len() % alignment;
    if rem != 0 {
        buf.resize(buf.len() + (alignment - rem), 0);
    }
}
