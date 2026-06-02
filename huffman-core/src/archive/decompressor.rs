use std::fs::{self, File};
use std::io::Read;
use std::num::NonZeroUsize;
use std::path::PathBuf;

use rayon::prelude::*;

use crate::bitstream::BitReader;
use crate::canonical::CanonicalTree;
use crate::error::HuffmanError;

pub struct HuffmanDecodeReader<R: Read> {
    bit_reader: BitReader<R>,
    decode_tree: crate::canonical::DecodeNode,
    payload_bits: u64,
    bits_consumed: u64,
}

impl<R: Read> HuffmanDecodeReader<R> {
    pub fn new(
        bit_reader: BitReader<R>,
        decode_tree: crate::canonical::DecodeNode,
        payload_bits: u64,
    ) -> Self {
        Self {
            bit_reader,
            decode_tree,
            payload_bits,
            bits_consumed: 0,
        }
    }
}

impl<R: Read> Read for HuffmanDecodeReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if buf.is_empty() || self.bits_consumed >= self.payload_bits {
            return Ok(0);
        }

        let mut bytes_read = 0;

        while bytes_read < buf.len() && self.bits_consumed < self.payload_bits {
            let mut curr = &self.decode_tree;
            loop {
                if let Some(sym) = curr.symbol {
                    buf[bytes_read] = sym;
                    bytes_read += 1;
                    break;
                }
                if self.bits_consumed >= self.payload_bits {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        "Unexpected end of bitstream",
                    ));
                }
                let bit = self.bit_reader.read_bit()?;
                self.bits_consumed += 1;
                if bit {
                    curr = curr.right.as_ref().ok_or_else(|| {
                        std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            "Bitstream points to an invalid tree path",
                        )
                    })?;
                } else {
                    curr = curr.left.as_ref().ok_or_else(|| {
                        std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            "Bitstream points to an invalid tree path",
                        )
                    })?;
                }
            }
        }

        Ok(bytes_read)
    }
}

pub struct Decompressor {
    pub archive_dir: PathBuf,
    pub archive_name: String,
    pub target_dir: PathBuf,
    pub password: Option<String>,
    pub parallelism: Option<NonZeroUsize>,
}

impl Decompressor {
    pub fn new<P1, P2>(archive_dir: P1, archive_name: String, target_dir: P2) -> Self
    where
        P1: Into<PathBuf>,
        P2: Into<PathBuf>,
    {
        Self {
            archive_dir: archive_dir.into(),
            archive_name,
            target_dir: target_dir.into(),
            password: None,
            parallelism: None,
        }
    }

    pub fn with_password(mut self, password: String) -> Self {
        self.password = Some(password);
        self
    }

    pub fn with_parallelism(mut self, parallelism: Option<NonZeroUsize>) -> Self {
        self.parallelism = parallelism;
        self
    }

    pub fn decompress(&self) -> Result<(), HuffmanError> {
        let archive_path = self.archive_dir.join(&self.archive_name);
        let mut file = File::open(archive_path)?;
        let file_metadata = file.metadata()?;
        let file_len = file_metadata.len();

        if file_len < 4 {
            return Err(HuffmanError::CorruptedArchive("Archive file is too small"));
        }

        // 1. 读取并校验 Magic
        let mut magic = [0u8; 4];
        file.read_exact(&mut magic)?;

        let version = match &magic {
            b"HAF\x03" => 3,
            b"HAF\x04" => 4,
            _ => return Err(HuffmanError::CorruptedArchive("Invalid magic number")),
        };

        let is_encrypted = version == 4;

        let mut salt = [0u8; 16];
        let mut nonce = [0u8; 12];
        if is_encrypted {
            if file_len < 34 { // 4 + 16 + 12 + 2 = 34
                return Err(HuffmanError::CorruptedArchive(
                    "Encrypted archive file is too small",
                ));
            }
            file.read_exact(&mut salt)?;
            file.read_exact(&mut nonce)?;
        }

        // 2. 读取 Tree Size
        let mut tree_size_buf = [0u8; 2];
        file.read_exact(&mut tree_size_buf)?;
        let tree_size = u16::from_be_bytes(tree_size_buf) as usize;

        // 确保目标解压目录存在
        fs::create_dir_all(&self.target_dir)?;

        // 定义要加载的数据结构（完全不带未使用字段与 allow）
        struct IndexEntry {
            entry_type: u8,
            relative_path: String,
            compressed_offset: u64,
            compressed_bits_len: u64,
            compressed_bytes_len: usize,
        }

        let run = || -> Result<(), HuffmanError> {
            let mut index_entries = Vec::new();
            let decode_tree;
            let mut compressed_data_buffer = Vec::new();

            if is_encrypted {
                let Some(ref pwd) = self.password else {
                    return Err(HuffmanError::PasswordRequired);
                };

                let key = crate::crypto::derive_key(pwd, &salt);
                let mut dec_reader = crate::crypto::DecryptReader::new(file, &key, &nonce);

                // 3. 读取 Compressed Tree
                let mut tree_buf = vec![0u8; tree_size];
                dec_reader
                    .read_exact(&mut tree_buf)
                    .map_err(|_| HuffmanError::PasswordMismatch)?;

                let canonical_tree = CanonicalTree::deserialize(&tree_buf)
                    .map_err(|_| HuffmanError::PasswordMismatch)?;
                decode_tree = canonical_tree.build_decode_tree();

                // 4. 读取索引区
                let mut entry_count_buf = [0u8; 4];
                dec_reader
                    .read_exact(&mut entry_count_buf)
                    .map_err(|_| HuffmanError::PasswordMismatch)?;
                let entry_count = u32::from_be_bytes(entry_count_buf);

                for _ in 0..entry_count {
                    let mut entry_type_buf = [0u8; 1];
                    dec_reader.read_exact(&mut entry_type_buf).map_err(|_| HuffmanError::PasswordMismatch)?;
                    let entry_type = entry_type_buf[0];

                    let mut path_len_buf = [0u8; 4];
                    dec_reader.read_exact(&mut path_len_buf).map_err(|_| HuffmanError::PasswordMismatch)?;
                    let path_len = u32::from_be_bytes(path_len_buf) as usize;

                    let mut path_bytes = vec![0u8; path_len];
                    dec_reader.read_exact(&mut path_bytes).map_err(|_| HuffmanError::PasswordMismatch)?;
                    let relative_path = std::str::from_utf8(&path_bytes)
                        .map_err(|_| HuffmanError::PasswordMismatch)?
                        .to_string();

                    // 安全校验：防范路径穿越
                    if relative_path.contains("..")
                        || relative_path.starts_with('/')
                        || relative_path.starts_with('\\')
                    {
                        return Err(HuffmanError::CorruptedArchive(
                            "Path traversal attempt detected",
                        ));
                    }

                    let mut file_size_buf = [0u8; 8];
                    dec_reader.read_exact(&mut file_size_buf).map_err(|_| HuffmanError::PasswordMismatch)?;
                    let _file_size = u64::from_be_bytes(file_size_buf);

                    let mut comp_offset_buf = [0u8; 8];
                    dec_reader.read_exact(&mut comp_offset_buf).map_err(|_| HuffmanError::PasswordMismatch)?;
                    let compressed_offset = u64::from_be_bytes(comp_offset_buf);

                    let mut comp_bits_len_buf = [0u8; 8];
                    dec_reader.read_exact(&mut comp_bits_len_buf).map_err(|_| HuffmanError::PasswordMismatch)?;
                    let compressed_bits_len = u64::from_be_bytes(comp_bits_len_buf);

                    index_entries.push(IndexEntry {
                        entry_type,
                        relative_path,
                        compressed_offset,
                        compressed_bits_len,
                        compressed_bytes_len: 0,
                    });
                }

                // 5. 顺序一次性读入并解密剩余所有的压缩数据块
                dec_reader.read_to_end(&mut compressed_data_buffer)?;
            } else {
                // 3. 读取 Compressed Tree
                let mut tree_buf = vec![0u8; tree_size];
                file.read_exact(&mut tree_buf)?;

                let canonical_tree = CanonicalTree::deserialize(&tree_buf)?;
                decode_tree = canonical_tree.build_decode_tree();

                // 4. 读取索引区
                let mut entry_count_buf = [0u8; 4];
                file.read_exact(&mut entry_count_buf)?;
                let entry_count = u32::from_be_bytes(entry_count_buf);

                for _ in 0..entry_count {
                    let mut entry_type_buf = [0u8; 1];
                    file.read_exact(&mut entry_type_buf)?;
                    let entry_type = entry_type_buf[0];

                    let mut path_len_buf = [0u8; 4];
                    file.read_exact(&mut path_len_buf)?;
                    let path_len = u32::from_be_bytes(path_len_buf) as usize;

                    let mut path_bytes = vec![0u8; path_len];
                    file.read_exact(&mut path_bytes)?;
                    let relative_path = std::str::from_utf8(&path_bytes)
                        .map_err(|_| HuffmanError::PathConversionError)?
                        .to_string();

                    // 安全校验：防范路径穿越
                    if relative_path.contains("..")
                        || relative_path.starts_with('/')
                        || relative_path.starts_with('\\')
                    {
                        return Err(HuffmanError::CorruptedArchive(
                            "Path traversal attempt detected",
                        ));
                    }

                    let mut file_size_buf = [0u8; 8];
                    file.read_exact(&mut file_size_buf)?;
                    let _file_size = u64::from_be_bytes(file_size_buf);

                    let mut comp_offset_buf = [0u8; 8];
                    file.read_exact(&mut comp_offset_buf)?;
                    let compressed_offset = u64::from_be_bytes(comp_offset_buf);

                    let mut comp_bits_len_buf = [0u8; 8];
                    file.read_exact(&mut comp_bits_len_buf)?;
                    let compressed_bits_len = u64::from_be_bytes(comp_bits_len_buf);

                    index_entries.push(IndexEntry {
                        entry_type,
                        relative_path,
                        compressed_offset,
                        compressed_bits_len,
                        compressed_bytes_len: 0,
                    });
                }

                // 5. 读取剩余的全部压缩字节
                file.read_to_end(&mut compressed_data_buffer)?;
            }

            // 6. 顺次计算每个条目各自的压缩字节长度
            for i in 0..index_entries.len() {
                let next_offset = if i + 1 < index_entries.len() {
                    index_entries[i + 1].compressed_offset as usize
                } else {
                    compressed_data_buffer.len()
                };
                index_entries[i].compressed_bytes_len = next_offset - (index_entries[i].compressed_offset as usize);
            }

            // 7. 并行解压和写磁盘
            index_entries.par_iter().try_for_each(|entry| -> Result<(), HuffmanError> {
                let target_path = self.target_dir.join(&entry.relative_path);

                if target_path.is_file() {
                    return Err(HuffmanError::AlreadyExists(target_path));
                }

                if entry.entry_type == 0x01 {
                    if let Some(parent) = target_path.parent() {
                        fs::create_dir_all(parent)?;
                    }

                    let slice_start = entry.compressed_offset as usize;
                    let slice_end = slice_start + entry.compressed_bytes_len;

                    if slice_end > compressed_data_buffer.len() {
                        return Err(HuffmanError::CorruptedArchive("Compressed data index out of range"));
                    }

                    let compressed_slice = &compressed_data_buffer[slice_start..slice_end];
                    let bit_reader = BitReader::new(compressed_slice);
                    let mut decode_reader = HuffmanDecodeReader::new(bit_reader, decode_tree.clone(), entry.compressed_bits_len);

                    let mut target_file = File::create(&target_path)?;
                    std::io::copy(&mut decode_reader, &mut target_file)?;
                } else if entry.entry_type == 0x02 {
                    fs::create_dir_all(&target_path)?;
                } else {
                    return Err(HuffmanError::CorruptedArchive(
                        "Invalid entry type in archive",
                    ));
                }
                Ok(())
            })?;

            Ok(())
        };

        if let Some(n) = self.parallelism {
            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(n.get())
                .build()
                .map_err(|e| HuffmanError::Io(std::io::Error::new(std::io::ErrorKind::Other, e.to_string())))?;
            pool.install(run)
        } else {
            run()
        }
    }
}
