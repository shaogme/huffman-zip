pub mod utils;

use std::os::raw::{c_char, c_int};
use std::panic;

/// 跨 ABI 的哈夫曼结果状态码定义
#[repr(C)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum HuffmanResult {
    Success = 0,
    InvalidPath = -1,
    IoError = -2,
    NullPointer = -3,
    InvalidArchive = -4,
    PasswordRequired = -5,
    PasswordMismatch = -6,
    AlreadyExists = -7,
    PanicTriggered = -99,
}

/// 兼容 C 的 ArchiveEntry 类型枚举
#[repr(C)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum CArchiveEntryType {
    File = 0,
    Directory = 1,
}

/// 兼容 C 的单个归档条目结构体
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct CArchiveEntry {
    pub entry_type: CArchiveEntryType,
    pub relative_path: *const c_char,
    /// 仅在 entry_type 为 File 时有效，表示文件名；若为 Directory 可传入空指针
    pub name: *const c_char,
    /// 仅在 entry_type 为 Directory 时有效，指示是否递归扫描
    pub recursive: bool,
}

/// 兼容 C 的 Compressor 配置结构体
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct CCompressorConfig {
    pub workspace: *const c_char,
    pub entries: *const CArchiveEntry,
    pub entries_count: usize,
    pub target_dir: *const c_char,
    pub target_name: *const c_char,
    pub password: *const c_char,
    pub overwrite: bool,
}

/// 兼容 C 的 Decompressor 配置结构体
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct CDecompressorConfig {
    pub archive_dir: *const c_char,
    pub archive_name: *const c_char,
    pub target_dir: *const c_char,
    pub password: *const c_char,
}

/// 安全地将 `CArchiveEntry` 转换为底层的 `huffman_core::ArchiveEntry`
unsafe fn convert_c_archive_entry(entry: &CArchiveEntry) -> Option<huffman_core::ArchiveEntry> {
    let rel_path = unsafe { utils::convert_c_str_to_string(entry.relative_path)? };
    match entry.entry_type {
        CArchiveEntryType::File => {
            let name = unsafe { utils::convert_c_str_to_string(entry.name)? };
            Some(huffman_core::ArchiveEntry::File {
                relative_path: rel_path,
                name,
            })
        }
        CArchiveEntryType::Directory => Some(huffman_core::ArchiveEntry::Directory {
            relative_path: rel_path,
            recursive: entry.recursive,
        }),
    }
}

/// 压缩接口
///
/// # 参数
/// * `config` - 指向 `CCompressorConfig` 配置结构体的指针
///
/// # 返回值
/// 成功返回 0，失败返回负数错误码
#[unsafe(no_mangle)]
pub unsafe extern "C" fn huffman_compress(config: *const CCompressorConfig) -> c_int {
    panic::catch_unwind(|| unsafe {
        if config.is_null() {
            return HuffmanResult::NullPointer as c_int;
        }
        let cfg = &*config;

        let Some(workspace) = utils::convert_c_str(cfg.workspace) else {
            return HuffmanResult::NullPointer as c_int;
        };

        let Some(target_dir) = utils::convert_c_str(cfg.target_dir) else {
            return HuffmanResult::NullPointer as c_int;
        };

        let Some(target_name) = utils::convert_c_str_to_string(cfg.target_name) else {
            return HuffmanResult::NullPointer as c_int;
        };

        if cfg.entries.is_null() && cfg.entries_count > 0 {
            return HuffmanResult::NullPointer as c_int;
        }

        let compressor_entries = if cfg.entries.is_null() || cfg.entries_count == 0 {
            huffman_core::CompressorEntries::All
        } else {
            let mut entries = Vec::with_capacity(cfg.entries_count);
            for i in 0..cfg.entries_count {
                let c_entry = &*cfg.entries.add(i);
                let Some(entry) = convert_c_archive_entry(c_entry) else {
                    return HuffmanResult::InvalidPath as c_int;
                };
                entries.push(entry);
            }
            huffman_core::CompressorEntries::Selected(entries)
        };

        let mut compressor =
            huffman_core::Compressor::new(workspace, compressor_entries, target_dir, target_name)
                .with_overwrite(cfg.overwrite);
        if !cfg.password.is_null() {
            if let Some(pwd) = utils::convert_c_str_to_string(cfg.password) {
                compressor = compressor.with_password(pwd);
            }
        }

        match compressor.compress() {
            Ok(()) => HuffmanResult::Success as c_int,
            Err(huffman_core::error::HuffmanError::Io(_)) => HuffmanResult::IoError as c_int,
            Err(huffman_core::error::HuffmanError::PathConversionError) => {
                HuffmanResult::InvalidPath as c_int
            }
            Err(huffman_core::error::HuffmanError::InvalidParameters) => {
                HuffmanResult::InvalidPath as c_int
            }
            Err(huffman_core::error::HuffmanError::CorruptedArchive(_)) => {
                HuffmanResult::InvalidArchive as c_int
            }
            Err(huffman_core::error::HuffmanError::PasswordRequired) => {
                HuffmanResult::PasswordRequired as c_int
            }
            Err(huffman_core::error::HuffmanError::PasswordMismatch) => {
                HuffmanResult::PasswordMismatch as c_int
            }
            Err(huffman_core::error::HuffmanError::AlreadyExists(_)) => {
                HuffmanResult::AlreadyExists as c_int
            }
        }
    })
    .unwrap_or(HuffmanResult::PanicTriggered as c_int)
}

/// 解压接口
///
/// # 参数
/// * `config` - 指向 `CDecompressorConfig` 配置结构体的指针
///
/// # 返回值
/// 成功返回 0，失败返回负数错误码
#[unsafe(no_mangle)]
pub unsafe extern "C" fn huffman_decompress(config: *const CDecompressorConfig) -> c_int {
    panic::catch_unwind(|| unsafe {
        if config.is_null() {
            return HuffmanResult::NullPointer as c_int;
        }
        let cfg = &*config;

        let Some(archive_dir) = utils::convert_c_str(cfg.archive_dir) else {
            return HuffmanResult::NullPointer as c_int;
        };

        let Some(archive_name) = utils::convert_c_str_to_string(cfg.archive_name) else {
            return HuffmanResult::NullPointer as c_int;
        };

        let Some(target_dir) = utils::convert_c_str(cfg.target_dir) else {
            return HuffmanResult::NullPointer as c_int;
        };

        let mut decompressor =
            huffman_core::Decompressor::new(archive_dir, archive_name, target_dir);
        if !cfg.password.is_null() {
            if let Some(pwd) = utils::convert_c_str_to_string(cfg.password) {
                decompressor = decompressor.with_password(pwd);
            }
        }

        match decompressor.decompress() {
            Ok(()) => HuffmanResult::Success as c_int,
            Err(huffman_core::error::HuffmanError::Io(_)) => HuffmanResult::IoError as c_int,
            Err(huffman_core::error::HuffmanError::PathConversionError) => {
                HuffmanResult::InvalidPath as c_int
            }
            Err(huffman_core::error::HuffmanError::InvalidParameters) => {
                HuffmanResult::InvalidPath as c_int
            }
            Err(huffman_core::error::HuffmanError::CorruptedArchive(_)) => {
                HuffmanResult::InvalidArchive as c_int
            }
            Err(huffman_core::error::HuffmanError::PasswordRequired) => {
                HuffmanResult::PasswordRequired as c_int
            }
            Err(huffman_core::error::HuffmanError::PasswordMismatch) => {
                HuffmanResult::PasswordMismatch as c_int
            }
            Err(huffman_core::error::HuffmanError::AlreadyExists(_)) => {
                HuffmanResult::AlreadyExists as c_int
            }
        }
    })
    .unwrap_or(HuffmanResult::PanicTriggered as c_int)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;
    use std::fs;
    use std::ptr;

    #[test]
    fn test_ffi_null_pointers() {
        unsafe {
            let res = huffman_compress(ptr::null());
            assert_eq!(res, HuffmanResult::NullPointer as c_int);

            let res = huffman_decompress(ptr::null());
            assert_eq!(res, HuffmanResult::NullPointer as c_int);
        }
    }

    #[test]
    fn test_ffi_panic_boundary() {
        let result = panic::catch_unwind(|| {
            panic!("Mock panic!");
        });
        assert!(result.is_err());
    }

    #[test]
    fn test_ffi_compress_decompress_roundtrip() {
        // 创建临时目录和测试文件进行端到端集成测试
        let temp_dir = std::env::temp_dir().join("huffman_dll_test");
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();

        let workspace_dir = temp_dir.join("workspace");
        fs::create_dir_all(&workspace_dir).unwrap();

        // 写入待压缩测试文件
        let test_file = workspace_dir.join("hello.txt");
        fs::write(&test_file, b"hello, huffman compression world!").unwrap();

        let target_dir = temp_dir.join("output");
        let target_name = "archive.haf";

        // C 兼容结构体参数配置
        let workspace_c = CString::new(workspace_dir.to_str().unwrap()).unwrap();
        let relative_path_c = CString::new("hello.txt").unwrap();
        let name_c = CString::new("hello.txt").unwrap();
        let target_dir_c = CString::new(target_dir.to_str().unwrap()).unwrap();
        let target_name_c = CString::new(target_name).unwrap();

        let entry = CArchiveEntry {
            entry_type: CArchiveEntryType::File,
            relative_path: relative_path_c.as_ptr(),
            name: name_c.as_ptr(),
            recursive: false,
        };

        let config = CCompressorConfig {
            workspace: workspace_c.as_ptr(),
            entries: &entry,
            entries_count: 1,
            target_dir: target_dir_c.as_ptr(),
            target_name: target_name_c.as_ptr(),
            password: ptr::null(),
            overwrite: true,
        };

        unsafe {
            // 测试压缩
            let res = huffman_compress(&config);
            assert_eq!(res, HuffmanResult::Success as c_int);

            // 验证生成的压缩包存在
            let archive_file = target_dir.join(target_name);
            assert!(archive_file.exists());

            // 测试解压缩
            let decomp_dir = temp_dir.join("decompressed");
            let decomp_dir_c = CString::new(decomp_dir.to_str().unwrap()).unwrap();

            let decomp_config = CDecompressorConfig {
                archive_dir: target_dir_c.as_ptr(),
                archive_name: target_name_c.as_ptr(),
                target_dir: decomp_dir_c.as_ptr(),
                password: ptr::null(),
            };

            let res = huffman_decompress(&decomp_config);
            assert_eq!(res, HuffmanResult::Success as c_int);

            // 验证解压出的内容一致
            let restored_file = decomp_dir.join("hello.txt");
            assert!(restored_file.exists());
            let content = fs::read_to_string(restored_file).unwrap();
            assert_eq!(content, "hello, huffman compression world!");
        }

        // 清理临时文件
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_ffi_encrypted_compress_decompress() {
        let temp_dir = std::env::temp_dir().join("huffman_dll_test_enc");
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();

        let workspace_dir = temp_dir.join("workspace");
        fs::create_dir_all(&workspace_dir).unwrap();

        let test_file = workspace_dir.join("secret.txt");
        fs::write(&test_file, b"highly sensitive data").unwrap();

        let target_dir = temp_dir.join("output");
        let target_name = "secret_archive.haf";

        let workspace_c = CString::new(workspace_dir.to_str().unwrap()).unwrap();
        let relative_path_c = CString::new("secret.txt").unwrap();
        let name_c = CString::new("secret.txt").unwrap();
        let target_dir_c = CString::new(target_dir.to_str().unwrap()).unwrap();
        let target_name_c = CString::new(target_name).unwrap();
        let password_c = CString::new("top_secret_pass").unwrap();

        let entry = CArchiveEntry {
            entry_type: CArchiveEntryType::File,
            relative_path: relative_path_c.as_ptr(),
            name: name_c.as_ptr(),
            recursive: false,
        };

        // 1. 加密压缩
        let config = CCompressorConfig {
            workspace: workspace_c.as_ptr(),
            entries: &entry,
            entries_count: 1,
            target_dir: target_dir_c.as_ptr(),
            target_name: target_name_c.as_ptr(),
            password: password_c.as_ptr(),
            overwrite: true,
        };

        unsafe {
            let res = huffman_compress(&config);
            assert_eq!(res, HuffmanResult::Success as c_int);

            let decomp_dir = temp_dir.join("decompressed");
            let decomp_dir_c = CString::new(decomp_dir.to_str().unwrap()).unwrap();

            // 2. 无密码解压缩（应失败）
            let decomp_config_no_pwd = CDecompressorConfig {
                archive_dir: target_dir_c.as_ptr(),
                archive_name: target_name_c.as_ptr(),
                target_dir: decomp_dir_c.as_ptr(),
                password: ptr::null(),
            };
            let res = huffman_decompress(&decomp_config_no_pwd);
            assert_eq!(res, HuffmanResult::PasswordRequired as c_int);

            // 3. 错误密码解压缩（应失败）
            let wrong_password_c = CString::new("wrong_password").unwrap();
            let decomp_config_wrong_pwd = CDecompressorConfig {
                archive_dir: target_dir_c.as_ptr(),
                archive_name: target_name_c.as_ptr(),
                target_dir: decomp_dir_c.as_ptr(),
                password: wrong_password_c.as_ptr(),
            };
            let res = huffman_decompress(&decomp_config_wrong_pwd);
            assert_eq!(res, HuffmanResult::PasswordMismatch as c_int);

            // 4. 正确密码解压缩（应成功）
            let decomp_config_correct = CDecompressorConfig {
                archive_dir: target_dir_c.as_ptr(),
                archive_name: target_name_c.as_ptr(),
                target_dir: decomp_dir_c.as_ptr(),
                password: password_c.as_ptr(),
            };
            let res = huffman_decompress(&decomp_config_correct);
            assert_eq!(res, HuffmanResult::Success as c_int);

            let restored_file = decomp_dir.join("secret.txt");
            assert!(restored_file.exists());
            let content = fs::read_to_string(restored_file).unwrap();
            assert_eq!(content, "highly sensitive data");
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_ffi_workspace_all_compression_decompression() {
        let temp_dir = std::env::temp_dir().join("huffman_dll_test_all");
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();

        let workspace_dir = temp_dir.join("workspace");
        fs::create_dir_all(&workspace_dir).unwrap();

        // 写入待压缩测试文件
        let test_file = workspace_dir.join("hello.txt");
        fs::write(&test_file, b"hello from DLL workspace all!").unwrap();

        let target_dir = temp_dir.join("output");
        let target_name = "archive_all.haf";

        let workspace_c = CString::new(workspace_dir.to_str().unwrap()).unwrap();
        let target_dir_c = CString::new(target_dir.to_str().unwrap()).unwrap();
        let target_name_c = CString::new(target_name).unwrap();

        let config = CCompressorConfig {
            workspace: workspace_c.as_ptr(),
            entries: ptr::null(),
            entries_count: 0,
            target_dir: target_dir_c.as_ptr(),
            target_name: target_name_c.as_ptr(),
            password: ptr::null(),
            overwrite: true,
        };

        unsafe {
            let res = huffman_compress(&config);
            assert_eq!(res, HuffmanResult::Success as c_int);

            let decomp_dir = temp_dir.join("decompressed");
            let decomp_dir_c = CString::new(decomp_dir.to_str().unwrap()).unwrap();

            let decomp_config = CDecompressorConfig {
                archive_dir: target_dir_c.as_ptr(),
                archive_name: target_name_c.as_ptr(),
                target_dir: decomp_dir_c.as_ptr(),
                password: ptr::null(),
            };

            let res = huffman_decompress(&decomp_config);
            assert_eq!(res, HuffmanResult::Success as c_int);

            let restored_file = decomp_dir.join("hello.txt");
            assert!(restored_file.exists());
            let content = fs::read_to_string(restored_file).unwrap();
            assert_eq!(content, "hello from DLL workspace all!");
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_ffi_overwrite_and_already_exists() {
        let temp_dir = std::env::temp_dir().join("huffman_dll_test_overwrite");
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();

        let workspace_dir = temp_dir.join("workspace");
        fs::create_dir_all(&workspace_dir).unwrap();

        let test_file = workspace_dir.join("hello.txt");
        fs::write(&test_file, b"hello world").unwrap();

        let target_dir = temp_dir.join("output");
        fs::create_dir_all(&target_dir).unwrap();
        let target_name = "archive.haf";

        // 先创建一个同名文件模拟冲突
        let target_archive_path = target_dir.join(target_name);
        fs::write(&target_archive_path, b"already exists archive data").unwrap();

        let workspace_c = CString::new(workspace_dir.to_str().unwrap()).unwrap();
        let relative_path_c = CString::new("hello.txt").unwrap();
        let name_c = CString::new("hello.txt").unwrap();
        let target_dir_c = CString::new(target_dir.to_str().unwrap()).unwrap();
        let target_name_c = CString::new(target_name).unwrap();

        let entry = CArchiveEntry {
            entry_type: CArchiveEntryType::File,
            relative_path: relative_path_c.as_ptr(),
            name: name_c.as_ptr(),
            recursive: false,
        };

        // 1. 测试当 overwrite 为 false 时压缩，应该报错 AlreadyExists (-7)
        let config_no_overwrite = CCompressorConfig {
            workspace: workspace_c.as_ptr(),
            entries: &entry,
            entries_count: 1,
            target_dir: target_dir_c.as_ptr(),
            target_name: target_name_c.as_ptr(),
            password: ptr::null(),
            overwrite: false,
        };

        unsafe {
            let res = huffman_compress(&config_no_overwrite);
            assert_eq!(res, HuffmanResult::AlreadyExists as c_int);
        }

        // 2. 测试当 overwrite 为 true 时压缩，应该成功
        let config_overwrite = CCompressorConfig {
            workspace: workspace_c.as_ptr(),
            entries: &entry,
            entries_count: 1,
            target_dir: target_dir_c.as_ptr(),
            target_name: target_name_c.as_ptr(),
            password: ptr::null(),
            overwrite: true,
        };

        unsafe {
            let res = huffman_compress(&config_overwrite);
            assert_eq!(res, HuffmanResult::Success as c_int);
        }

        // 3. 测试解压时目标路径存在同名文件，应该报错 AlreadyExists (-7)
        let decomp_dir = temp_dir.join("decompressed");
        fs::create_dir_all(&decomp_dir).unwrap();
        // 创建一个同名文件以阻碍解压
        let restored_file_conflict = decomp_dir.join("hello.txt");
        fs::write(&restored_file_conflict, b"pre-existing file").unwrap();

        let decomp_dir_c = CString::new(decomp_dir.to_str().unwrap()).unwrap();
        let decomp_config = CDecompressorConfig {
            archive_dir: target_dir_c.as_ptr(),
            archive_name: target_name_c.as_ptr(),
            target_dir: decomp_dir_c.as_ptr(),
            password: ptr::null(),
        };

        unsafe {
            let res = huffman_decompress(&decomp_config);
            assert_eq!(res, HuffmanResult::AlreadyExists as c_int);
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
