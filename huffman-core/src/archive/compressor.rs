use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use crate::bitstream::BitWriter;
use crate::canonical::CanonicalTree;
use crate::error::HuffmanError;
use crate::tree::{build_tree, get_code_lengths};

use super::payload::{self, PayloadEntry};

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
        }
    }

    pub fn with_password(mut self, password: String) -> Self {
        self.password = Some(password);
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

        // 1. 计算字节频次
        let mut freqs = [0usize; 256];
        let mut stream = payload::PayloadStream::new(&entries);
        let mut buf = [0u8; 8192];
        loop {
            let n = stream.read(&mut buf)?;
            if n == 0 {
                break;
            }
            for &byte in &buf[..n] {
                freqs[byte as usize] += 1;
            }
        }

        // 2. 构建哈夫曼树
        let huffman_tree = build_tree(&freqs)
            .ok_or_else(|| HuffmanError::CorruptedArchive("Failed to build Huffman tree"))?;

        let lengths = get_code_lengths(&huffman_tree);

        // 3. 重构范式哈夫曼树并生成码表
        let canonical = CanonicalTree::new(lengths);
        let encoder_table = canonical.generate_encoder_table();

        // 4. 计算有效比特数
        let mut payload_bits = 0u64;
        let mut stream = payload::PayloadStream::new(&entries);
        loop {
            let n = stream.read(&mut buf)?;
            if n == 0 {
                break;
            }
            for &byte in &buf[..n] {
                if let Some(ref code) = encoder_table[byte as usize] {
                    payload_bits += code.len as u64;
                } else {
                    return Err(HuffmanError::CorruptedArchive(
                        "Unable to encode a byte, missing in tree",
                    ));
                }
            }
        }

        // 5. 写入物理 HAF 文件
        fs::create_dir_all(&self.target_dir)?;
        let out_archive_path = self.target_dir.join(&self.target_name);
        let mut out_file = File::create(out_archive_path)?;
        let tree_bytes = canonical.serialize();

        if let Some(ref pwd) = self.password {
            use rand::RngCore;
            let mut salt = [0u8; 16];
            let mut nonce = [0u8; 12];
            rand::thread_rng().fill_bytes(&mut salt);
            rand::thread_rng().fill_bytes(&mut nonce);

            let key = crate::crypto::derive_key(pwd, &salt);

            out_file.write_all(b"HAF\x02")?;
            out_file.write_all(&salt)?;
            out_file.write_all(&nonce)?;
            out_file.write_all(&(tree_bytes.len() as u16).to_be_bytes())?;
            out_file.write_all(&payload_bits.to_be_bytes())?;

            let mut enc_writer = crate::crypto::EncryptWriter::new(out_file, &key, &nonce);
            enc_writer.write_all(&tree_bytes)?;

            // 6. 使用 BitWriter 压缩数据
            let mut bit_writer = BitWriter::new(enc_writer);
            let mut stream = payload::PayloadStream::new(&entries);
            loop {
                let n = stream.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                for &byte in &buf[..n] {
                    if let Some(ref code) = encoder_table[byte as usize] {
                        bit_writer.write_bits(code.code, code.len as usize)?;
                    }
                }
            }

            bit_writer.into_inner()?;
        } else {
            out_file.write_all(b"HAF\x01")?;
            out_file.write_all(&(tree_bytes.len() as u16).to_be_bytes())?;
            out_file.write_all(&payload_bits.to_be_bytes())?;
            out_file.write_all(&tree_bytes)?;

            // 6. 使用 BitWriter 压缩数据
            let mut bit_writer = BitWriter::new(out_file);
            let mut stream = payload::PayloadStream::new(&entries);
            loop {
                let n = stream.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                for &byte in &buf[..n] {
                    if let Some(ref code) = encoder_table[byte as usize] {
                        bit_writer.write_bits(code.code, code.len as usize)?;
                    }
                }
            }

            bit_writer.into_inner()?;
        }
        Ok(())
    }
}
