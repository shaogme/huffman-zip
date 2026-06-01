use std::io::{self, Read, Write};

/// 位流写入器，支持向 Write 写入变长的比特
pub struct BitWriter<W: Write> {
    inner: Option<W>,
    byte_buf: u8,
    bit_count: u8,
}

impl<W: Write> BitWriter<W> {
    pub fn new(inner: W) -> Self {
        Self {
            inner: Some(inner),
            byte_buf: 0,
            bit_count: 0,
        }
    }

    /// 写入单个比特
    pub fn write_bit(&mut self, bit: bool) -> io::Result<()> {
        if bit {
            self.byte_buf |= 1 << (7 - self.bit_count);
        }
        self.bit_count += 1;
        if self.bit_count == 8 {
            if let Some(ref mut inner) = self.inner {
                inner.write_all(&[self.byte_buf])?;
            }
            self.byte_buf = 0;
            self.bit_count = 0;
        }
        Ok(())
    }

    /// 写入指定长度的比特值 (采用 u128 支持深层哈夫曼树)
    pub fn write_bits(&mut self, value: u128, bit_len: usize) -> io::Result<()> {
        for i in (0..bit_len).rev() {
            let bit = ((value >> i) & 1) != 0;
            self.write_bit(bit)?;
        }
        Ok(())
    }

    /// 强制将不足 1 字节的部分补 0 刷出
    pub fn flush_bits(&mut self) -> io::Result<()> {
        if self.bit_count > 0 {
            if let Some(ref mut inner) = self.inner {
                inner.write_all(&[self.byte_buf])?;
            }
            self.byte_buf = 0;
            self.bit_count = 0;
        }
        if let Some(ref mut inner) = self.inner {
            inner.flush()?;
        }
        Ok(())
    }

    /// 消费掉 BitWriter，返回底层的 Write，并保证刷出所有比特
    pub fn into_inner(mut self) -> io::Result<W> {
        self.flush_bits()?;
        Ok(self.inner.take().unwrap())
    }
}

impl<W: Write> Drop for BitWriter<W> {
    fn drop(&mut self) {
        if self.inner.is_some() {
            let _ = self.flush_bits();
        }
    }
}

/// 位流读取器，支持从 Read 读取变长的比特
pub struct BitReader<R: Read> {
    inner: R,
    byte_buf: u8,
    bit_count: u8,
}

impl<R: Read> BitReader<R> {
    pub fn new(inner: R) -> Self {
        Self {
            inner,
            byte_buf: 0,
            bit_count: 0,
        }
    }

    /// 读取单个比特
    pub fn read_bit(&mut self) -> io::Result<bool> {
        if self.bit_count == 0 {
            let mut buf = [0u8; 1];
            self.inner.read_exact(&mut buf)?;
            self.byte_buf = buf[0];
            self.bit_count = 8;
        }
        let bit = ((self.byte_buf >> (self.bit_count - 1)) & 1) != 0;
        self.bit_count -= 1;
        Ok(bit)
    }

    /// 读取指定长度的比特值，并返回 u128
    pub fn read_bits(&mut self, bit_len: usize) -> io::Result<u128> {
        let mut value = 0u128;
        for _ in 0..bit_len {
            let bit = self.read_bit()?;
            value = (value << 1) | (if bit { 1 } else { 0 });
        }
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bit_writer_reader() {
        let mut buffer = Vec::new();
        {
            let mut writer = BitWriter::new(&mut buffer);
            writer.write_bit(true).unwrap();
            writer.write_bit(false).unwrap();
            writer.write_bits(5, 3).unwrap(); // 5 = 二进制 101, 长度 3
            writer.write_bits(3, 2).unwrap(); // 3 = 二进制 11, 长度 2
            writer.write_bit(true).unwrap(); // 共计 1 + 1 + 3 + 2 + 1 = 8 位，刚好一个字节

            // 写入第二个字节的部分内容
            writer.write_bit(false).unwrap();
            writer.write_bits(1, 1).unwrap();
        } // drop 自动 flush 第二个字节（余下6位补0）

        assert_eq!(buffer.len(), 2);

        let mut reader = BitReader::new(&buffer[..]);
        assert_eq!(reader.read_bit().unwrap(), true);
        assert_eq!(reader.read_bit().unwrap(), false);
        assert_eq!(reader.read_bits(3).unwrap(), 5);
        assert_eq!(reader.read_bits(2).unwrap(), 3);
        assert_eq!(reader.read_bit().unwrap(), true);

        assert_eq!(reader.read_bit().unwrap(), false);
        assert_eq!(reader.read_bit().unwrap(), true);
    }
}
