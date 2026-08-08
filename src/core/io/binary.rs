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
