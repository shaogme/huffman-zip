use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

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
        }
    }

    pub fn with_password(mut self, password: String) -> Self {
        self.password = Some(password);
        self
    }

    pub fn decompress(&self) -> Result<(), HuffmanError> {
        let archive_path = self.archive_dir.join(&self.archive_name);
        let mut file = File::open(archive_path)?;
        let file_metadata = file.metadata()?;
        let file_len = file_metadata.len();

        if file_len < 14 {
            return Err(HuffmanError::CorruptedArchive("Archive file is too small"));
        }

        // 1. 读取并校验 Magic
        let mut magic = [0u8; 4];
        file.read_exact(&mut magic)?;
        let is_encrypted = if &magic == b"HAF\x02" {
            true
        } else if &magic == b"HAF\x01" {
            false
        } else {
            return Err(HuffmanError::CorruptedArchive("Invalid magic number"));
        };

        let mut salt = [0u8; 16];
        let mut nonce = [0u8; 12];
        if is_encrypted {
            if file_len < 42 {
                return Err(HuffmanError::CorruptedArchive(
                    "Encrypted archive file is too small",
                ));
            }
            file.read_exact(&mut salt)?;
            file.read_exact(&mut nonce)?;
        }

        // 2. 读取 Tree Size 和 Payload Bits
        let mut tree_size_buf = [0u8; 2];
        file.read_exact(&mut tree_size_buf)?;
        let tree_size = u16::from_be_bytes(tree_size_buf) as usize;

        let mut payload_bits_buf = [0u8; 8];
        file.read_exact(&mut payload_bits_buf)?;
        let payload_bits = u64::from_be_bytes(payload_bits_buf);

        let min_required_len = if is_encrypted {
            42 + tree_size as u64
        } else {
            14 + tree_size as u64
        };

        if file_len < min_required_len {
            return Err(HuffmanError::CorruptedArchive(
                "Archive header size mismatched",
            ));
        }

        // 确保目标解压目录存在
        fs::create_dir_all(&self.target_dir)?;

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
            let decode_tree = canonical_tree.build_decode_tree();

            // 4. 比特流还原并流式解压
            let bit_reader = BitReader::new(dec_reader);
            let mut decode_reader = HuffmanDecodeReader::new(bit_reader, decode_tree, payload_bits);

            // 5. 流式解析并还原文件和目录
            Self::decompress_entries(&mut decode_reader, &self.target_dir, true)?;
        } else {
            // 3. 读取 Compressed Tree
            let mut tree_buf = vec![0u8; tree_size];
            file.read_exact(&mut tree_buf)?;

            let canonical_tree = CanonicalTree::deserialize(&tree_buf)?;
            let decode_tree = canonical_tree.build_decode_tree();

            // 4. 比特流还原并流式解压
            let bit_reader = BitReader::new(file);
            let mut decode_reader = HuffmanDecodeReader::new(bit_reader, decode_tree, payload_bits);

            // 5. 流式解析并还原文件和目录
            Self::decompress_entries(&mut decode_reader, &self.target_dir, false)?;
        }

        Ok(())
    }

    fn decompress_entries<R: Read>(
        decode_reader: &mut HuffmanDecodeReader<R>,
        target_dir: &Path,
        is_encrypted: bool,
    ) -> Result<(), HuffmanError> {
        let mut entry_count_buf = [0u8; 4];
        decode_reader
            .read_exact(&mut entry_count_buf)
            .map_err(|e| {
                if is_encrypted {
                    HuffmanError::PasswordMismatch
                } else {
                    HuffmanError::Io(e)
                }
            })?;
        let entry_count = u32::from_be_bytes(entry_count_buf);

        for _ in 0..entry_count {
            let mut entry_type_buf = [0u8; 1];
            decode_reader.read_exact(&mut entry_type_buf).map_err(|e| {
                if is_encrypted {
                    HuffmanError::PasswordMismatch
                } else {
                    HuffmanError::Io(e)
                }
            })?;
            let entry_type = entry_type_buf[0];

            let mut path_len_buf = [0u8; 4];
            decode_reader.read_exact(&mut path_len_buf).map_err(|e| {
                if is_encrypted {
                    HuffmanError::PasswordMismatch
                } else {
                    HuffmanError::Io(e)
                }
            })?;
            let path_len = u32::from_be_bytes(path_len_buf) as usize;

            let mut path_bytes = vec![0u8; path_len];
            decode_reader.read_exact(&mut path_bytes).map_err(|e| {
                if is_encrypted {
                    HuffmanError::PasswordMismatch
                } else {
                    HuffmanError::Io(e)
                }
            })?;
            let relative_path = std::str::from_utf8(&path_bytes)
                .map_err(|_| {
                    if is_encrypted {
                        HuffmanError::PasswordMismatch
                    } else {
                        HuffmanError::PathConversionError
                    }
                })?
                .to_string();

            // 安全校验：防范路径穿越（Path Traversal）漏洞
            if relative_path.contains("..")
                || relative_path.starts_with('/')
                || relative_path.starts_with('\\')
            {
                return Err(HuffmanError::CorruptedArchive(
                    "Path traversal attempt detected",
                ));
            }

            let target_path = target_dir.join(&relative_path);

            if entry_type == 0x01 {
                let mut file_size_buf = [0u8; 8];
                decode_reader.read_exact(&mut file_size_buf).map_err(|e| {
                    if is_encrypted {
                        HuffmanError::PasswordMismatch
                    } else {
                        HuffmanError::Io(e)
                    }
                })?;
                let file_size = u64::from_be_bytes(file_size_buf);

                if let Some(parent) = target_path.parent() {
                    fs::create_dir_all(parent)?;
                }
                let mut target_file = File::create(&target_path)?;
                let mut file_reader = decode_reader.take(file_size);
                std::io::copy(&mut file_reader, &mut target_file).map_err(|e| {
                    if is_encrypted {
                        HuffmanError::PasswordMismatch
                    } else {
                        HuffmanError::Io(e)
                    }
                })?;
            } else if entry_type == 0x02 {
                fs::create_dir_all(&target_path)?;
            } else {
                return Err(HuffmanError::CorruptedArchive(
                    "Invalid entry type in archive",
                ));
            }
        }

        Ok(())
    }
}
