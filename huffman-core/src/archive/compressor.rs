use std::fs::{self, File};
use std::io::{Read, Write};
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};

use rayon::prelude::*;

use crate::bitstream::BitWriter;
use crate::canonical::CanonicalTree;
use crate::error::HuffmanError;
use crate::tree::{build_tree, get_code_lengths};

/// 单个归档条目的内存表示
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PayloadEntry {
    File {
        relative_path: String,
        file_size: u64,
        source_path: Option<std::path::PathBuf>,
        raw_data: Option<Vec<u8>>,
    },
    Directory {
        relative_path: String,
    },
}

#[derive(Debug, Clone)]
pub enum ArchiveEntry {
    File {
        relative_path: String,
        name: String,
    },
    Directory {
        relative_path: String,
        recursive: bool,
    },
}

#[derive(Debug, Clone)]
pub enum CompressorEntries {
    All,
    Selected(Vec<ArchiveEntry>),
}

fn scan_dir_impl(
    dir_path: &Path,
    workspace: &Path,
    recursive: bool,
    entries: &mut Vec<PayloadEntry>,
) -> Result<(), HuffmanError> {
    for entry in fs::read_dir(dir_path)? {
        let entry = entry?;
        let child_path = entry.path();

        let rel_path = child_path
            .strip_prefix(workspace)
            .map_err(|_| HuffmanError::PathConversionError)?
            .to_str()
            .ok_or(HuffmanError::PathConversionError)?
            .replace('\\', "/");

        if child_path.is_file() {
            let metadata = fs::metadata(&child_path)?;
            let file_size = metadata.len();
            entries.push(PayloadEntry::File {
                relative_path: rel_path,
                file_size,
                source_path: Some(child_path),
                raw_data: None,
            });
        } else if child_path.is_dir() {
            entries.push(PayloadEntry::Directory {
                relative_path: rel_path.clone(),
            });
            if recursive {
                scan_dir_impl(&child_path, workspace, true, entries)?;
            }
        }
    }
    Ok(())
}

pub struct Compressor {
    pub workspace: PathBuf,
    pub entries: CompressorEntries,
    pub target_dir: PathBuf,
    pub target_name: String,
    pub password: Option<String>,
    pub overwrite: bool,
    pub parallelism: Option<NonZeroUsize>,
}

impl Compressor {
    pub fn new<P1, P2>(
        workspace: P1,
        entries: CompressorEntries,
        target_dir: P2,
        target_name: String,
    ) -> Self
    where
        P1: Into<PathBuf>,
        P2: Into<PathBuf>,
    {
        Self {
            workspace: workspace.into(),
            entries,
            target_dir: target_dir.into(),
            target_name,
            password: None,
            overwrite: true,
            parallelism: None,
        }
    }

    pub fn with_password(mut self, password: String) -> Self {
        self.password = Some(password);
        self
    }

    pub fn with_overwrite(mut self, overwrite: bool) -> Self {
        self.overwrite = overwrite;
        self
    }

    pub fn with_parallelism(mut self, parallelism: Option<NonZeroUsize>) -> Self {
        self.parallelism = parallelism;
        self
    }

    pub fn compress(&self) -> Result<(), HuffmanError> {
        let abs_workspace = self.workspace.canonicalize().map_err(HuffmanError::Io)?;
        let mut entries = Vec::new();

        match &self.entries {
            CompressorEntries::All => {
                scan_dir_impl(&abs_workspace, &abs_workspace, true, &mut entries)?;
            }
            CompressorEntries::Selected(selected) => {
                if selected.is_empty() {
                    return Err(HuffmanError::InvalidParameters);
                }

                for entry in selected {
                    match entry {
                        ArchiveEntry::File { relative_path, .. } => {
                            let full_path = self.workspace.join(relative_path);
                            let abs_path = full_path.canonicalize().map_err(HuffmanError::Io)?;

                            if !abs_path.starts_with(&abs_workspace) {
                                return Err(HuffmanError::PathConversionError);
                            }

                            let rel_path = abs_path
                                .strip_prefix(&abs_workspace)
                                .map_err(|_| HuffmanError::PathConversionError)?
                                .to_str()
                                .ok_or(HuffmanError::PathConversionError)?
                                .replace('\\', "/");

                            let metadata = fs::metadata(&abs_path)?;
                            let file_size = metadata.len();
                            entries.push(PayloadEntry::File {
                                relative_path: rel_path,
                                file_size,
                                source_path: Some(abs_path),
                                raw_data: None,
                            });
                        }
                        ArchiveEntry::Directory {
                            relative_path,
                            recursive,
                        } => {
                            let full_path = self.workspace.join(relative_path);
                            let abs_path = full_path.canonicalize().map_err(HuffmanError::Io)?;

                            if !abs_path.starts_with(&abs_workspace) {
                                return Err(HuffmanError::PathConversionError);
                            }

                            let rel_path = abs_path
                                .strip_prefix(&abs_workspace)
                                .map_err(|_| HuffmanError::PathConversionError)?
                                .to_str()
                                .ok_or(HuffmanError::PathConversionError)?
                                .replace('\\', "/");

                            entries.push(PayloadEntry::Directory {
                                relative_path: rel_path,
                            });

                            scan_dir_impl(&abs_path, &abs_workspace, *recursive, &mut entries)?;
                        }
                    }
                }
            }
        }

        let run = || -> Result<(), HuffmanError> {
            // 1. 并行计算字节频次
            let freqs = entries.par_iter().map(|entry| {
                let mut local_freqs = [0usize; 256];
                match entry {
                    PayloadEntry::File { source_path, raw_data, .. } => {
                        if let Some(path) = source_path {
                            if let Ok(mut f) = fs::File::open(path) {
                                let mut buf = [0u8; 8192];
                                while let Ok(n) = f.read(&mut buf) {
                                    if n == 0 {
                                        break;
                                    }
                                    for &byte in &buf[..n] {
                                        local_freqs[byte as usize] += 1;
                                    }
                                }
                            }
                        } else if let Some(data) = raw_data {
                            for &byte in data {
                                local_freqs[byte as usize] += 1;
                            }
                        }
                    }
                    PayloadEntry::Directory { .. } => {}
                }
                local_freqs
            }).reduce(|| [0usize; 256], |mut a, b| {
                for i in 0..256 {
                    a[i] += b[i];
                }
                a
            });

            // 2. 构建哈夫曼树
            let huffman_tree = build_tree(&freqs)
                .ok_or_else(|| HuffmanError::CorruptedArchive("Failed to build Huffman tree"))?;

            let lengths = get_code_lengths(&huffman_tree);

            // 3. 重构范式哈夫曼树并生成码表
            let canonical = CanonicalTree::new(lengths);
            let encoder_table = canonical.generate_encoder_table();

            // 4. 并行对各个条目进行哈夫曼编码，在内存中生成临时的压缩比特数组
            struct EncodedEntry {
                compressed_data: Vec<u8>,
                compressed_bits_len: u64,
            }

            let encoded_results: Result<Vec<EncodedEntry>, HuffmanError> = entries.par_iter().map(|entry| {
                match entry {
                    PayloadEntry::File { source_path, raw_data, file_size, .. } => {
                        let mut buffer = Vec::with_capacity((*file_size as usize) / 2);
                        let mut bit_writer = BitWriter::new(&mut buffer);
                        let mut bits_written = 0u64;

                        let mut process_byte = |byte: u8| -> Result<(), HuffmanError> {
                            if let Some(ref code) = encoder_table[byte as usize] {
                                bit_writer.write_bits(code.code, code.len as usize)?;
                                bits_written += code.len as u64;
                                Ok(())
                            } else {
                                Err(HuffmanError::CorruptedArchive(
                                    "Unable to encode a byte, missing in tree",
                                ))
                            }
                        };

                        if let Some(path) = source_path {
                            let mut f = fs::File::open(path)?;
                            let mut read_buf = [0u8; 8192];
                            while let Ok(n) = f.read(&mut read_buf) {
                                if n == 0 {
                                    break;
                                }
                                for &b in &read_buf[..n] {
                                    process_byte(b)?;
                                }
                            }
                        } else if let Some(data) = raw_data {
                            for &b in data {
                                process_byte(b)?;
                            }
                        }
                        bit_writer.into_inner()?;
                        Ok(EncodedEntry {
                            compressed_data: buffer,
                            compressed_bits_len: bits_written,
                        })
                    }
                    PayloadEntry::Directory { .. } => {
                        Ok(EncodedEntry {
                            compressed_data: Vec::new(),
                            compressed_bits_len: 0,
                        })
                    }
                }
            }).collect();

            let encoded_results = encoded_results?;

            // 5. 计算绝对偏移量并构造索引数据
            let mut current_offset = 0u64;
            let mut index_bytes = Vec::new();
            index_bytes.write_all(&(entries.len() as u32).to_be_bytes())?;

            for (entry, encoded) in entries.iter().zip(encoded_results.iter()) {
                let (entry_type, relative_path, file_size) = match entry {
                    PayloadEntry::File { relative_path, file_size, .. } => (0x01, relative_path, *file_size),
                    PayloadEntry::Directory { relative_path } => (0x02, relative_path, 0),
                };
                index_bytes.write_all(&[entry_type])?;
                let path_bytes = relative_path.as_bytes();
                index_bytes.write_all(&(path_bytes.len() as u32).to_be_bytes())?;
                index_bytes.write_all(path_bytes)?;
                index_bytes.write_all(&file_size.to_be_bytes())?;
                index_bytes.write_all(&current_offset.to_be_bytes())?;
                index_bytes.write_all(&encoded.compressed_bits_len.to_be_bytes())?;

                current_offset += encoded.compressed_data.len() as u64;
            }

            // 6. 写入物理 HAF 文件
            fs::create_dir_all(&self.target_dir)?;
            let out_archive_path = self.target_dir.join(&self.target_name);
            if !self.overwrite && out_archive_path.exists() {
                return Err(HuffmanError::AlreadyExists(out_archive_path));
            }
            let mut out_file = File::create(&out_archive_path)?;
            let tree_bytes = canonical.serialize();

            if let Some(ref pwd) = self.password {
                use rand::RngCore;
                let mut salt = [0u8; 16];
                let mut nonce = [0u8; 12];
                rand::thread_rng().fill_bytes(&mut salt);
                rand::thread_rng().fill_bytes(&mut nonce);

                let key = crate::crypto::derive_key(pwd, &salt);

                out_file.write_all(b"HAF\x04")?;
                out_file.write_all(&salt)?;
                out_file.write_all(&nonce)?;
                out_file.write_all(&(tree_bytes.len() as u16).to_be_bytes())?;

                let mut enc_writer = crate::crypto::EncryptWriter::new(out_file, &key, &nonce);
                enc_writer.write_all(&tree_bytes)?;
                enc_writer.write_all(&index_bytes)?;
                for encoded in &encoded_results {
                    enc_writer.write_all(&encoded.compressed_data)?;
                }
            } else {
                out_file.write_all(b"HAF\x03")?;
                out_file.write_all(&(tree_bytes.len() as u16).to_be_bytes())?;
                out_file.write_all(&tree_bytes)?;
                out_file.write_all(&index_bytes)?;
                for encoded in &encoded_results {
                    out_file.write_all(&encoded.compressed_data)?;
                }
            }
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
