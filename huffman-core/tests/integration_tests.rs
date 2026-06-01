use rand::Rng;
use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::Hasher;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

use huffman_core::{ArchiveEntry, Compressor, Decompressor};

/// 计算字节切片的 Hash 值以验证内容一致性
fn calculate_hash(data: &[u8]) -> u64 {
    let mut hasher = DefaultHasher::new();
    hasher.write(data);
    hasher.finish()
}

/// 递归获取目录下所有项的相对路径
fn get_all_relative_paths(dir: &Path, base: &Path) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            paths.push(path.strip_prefix(base).unwrap().to_path_buf());
            if path.is_dir() {
                paths.extend(get_all_relative_paths(&path, base));
            }
        }
    }
    paths
}

#[test]
fn test_complex_folder_compression_decompression() {
    let mut rng = rand::thread_rng();

    // 1. 创建高度复杂的临时测试源目录
    let temp_src = TempDir::new().unwrap();
    let src_dir = temp_src.path();

    // 创建子目录
    let sub_dir1 = src_dir.join("dir1");
    let sub_dir2 = src_dir.join("dir1/sub_dir2");
    let empty_dir = src_dir.join("dir1/empty_dir");
    fs::create_dir_all(&sub_dir1).unwrap();
    fs::create_dir_all(&sub_dir2).unwrap();
    fs::create_dir_all(&empty_dir).unwrap();

    // 写入随机文本文件 1
    let file1 = src_dir.join("file1.txt");
    let file1_len = rng.gen_range(50..200);
    let file1_data: Vec<u8> = (0..file1_len)
        .map(|_| rng.gen_range(32..127) as u8)
        .collect();
    fs::write(&file1, &file1_data).unwrap();
    let file1_hash = calculate_hash(&file1_data);

    // 写入随机文本文件 2
    let file2 = sub_dir1.join("file2.txt");
    let file2_len = rng.gen_range(50..200);
    let file2_data: Vec<u8> = (0..file2_len)
        .map(|_| rng.gen_range(32..127) as u8)
        .collect();
    fs::write(&file2, &file2_data).unwrap();
    let file2_hash = calculate_hash(&file2_data);

    // 写入一个大随机二进制数据文件 (5MB)
    let file3 = sub_dir2.join("large_binary.bin");
    let mut large_data = vec![0u8; 5 * 1024 * 1024];
    rng.fill(&mut large_data[..]);
    fs::write(&file3, &large_data).unwrap();
    let file3_hash = calculate_hash(&large_data);

    // 2. 确定输出归档文件和解压目标目录
    let temp_archive = TempDir::new().unwrap();
    let archive_path = temp_archive.path().join("test_archive.haf");

    let temp_dest = TempDir::new().unwrap();
    let dest_dir = temp_dest.path();

    // 3. 执行核心压缩
    let workspace = src_dir.parent().unwrap().to_path_buf();
    let folder_name = src_dir.file_name().unwrap().to_str().unwrap().to_string();
    let target_dir = archive_path.parent().unwrap().to_path_buf();
    let target_name = archive_path
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();

    let compressor = Compressor::new(
        workspace,
        huffman_core::CompressorEntries::Selected(vec![ArchiveEntry::Directory {
            relative_path: folder_name.clone(),
            recursive: true,
        }]),
        target_dir,
        target_name,
    );
    compressor.compress().unwrap();

    // 4. 执行核心解压
    let archive_dir = archive_path.parent().unwrap().to_path_buf();
    let archive_name = archive_path
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let decompressor = Decompressor::new(archive_dir, archive_name, dest_dir.to_path_buf());
    decompressor.decompress().unwrap();

    // 我们压缩的是 `src_dir` 这一文件夹，所以还原出来的结构根目录应该包含 `src_dir` 本身的文件夹名称。
    // 即解压后应该存在： `dest_dir / [src_dir_folder_name] / ...`
    let src_folder_name = src_dir.file_name().unwrap().to_str().unwrap();
    let restored_root = dest_dir.join(src_folder_name);

    // 5. 递归比对源文件夹与还原后的文件夹内容与元数据
    let src_paths = get_all_relative_paths(src_dir, src_dir);
    let dest_paths = get_all_relative_paths(&restored_root, &restored_root);

    assert_eq!(
        src_paths.len(),
        dest_paths.len(),
        "还原后的文件和目录总数不匹配"
    );

    for rel_path in &src_paths {
        let original_path = src_dir.join(rel_path);
        let restored_path = restored_root.join(rel_path);

        assert!(restored_path.exists(), "还原项不存在: {:?}", rel_path);

        if original_path.is_file() {
            assert!(restored_path.is_file(), "还原项应该是文件: {:?}", rel_path);
            let orig_data = fs::read(&original_path).unwrap();
            let dest_data = fs::read(&restored_path).unwrap();

            // 计算哈希并比对
            let orig_hash = calculate_hash(&orig_data);
            let dest_hash = calculate_hash(&dest_data);
            assert_eq!(
                orig_hash, dest_hash,
                "解压前后的文件 Hash 不匹配: {:?}",
                rel_path
            );

            // 额外与压缩前记录的 Hash 校验
            if rel_path == Path::new("file1.txt") {
                assert_eq!(dest_hash, file1_hash, "file1.txt 哈希与记录不一致");
            } else if rel_path == Path::new("dir1/file2.txt") {
                assert_eq!(dest_hash, file2_hash, "file2.txt 哈希与记录不一致");
            } else if rel_path == Path::new("dir1/sub_dir2/large_binary.bin") {
                assert_eq!(dest_hash, file3_hash, "large_binary.bin 哈希与记录不一致");
            }
        } else if original_path.is_dir() {
            assert!(restored_path.is_dir(), "还原项应该是目录: {:?}", rel_path);
        }
    }
}

#[test]
fn test_encrypted_compression_decompression() {
    let temp_src = TempDir::new().unwrap();
    let src_dir = temp_src.path();

    let file_path = src_dir.join("secret.txt");
    let test_data = b"This is a top secret message encrypted with ChaCha20!";
    fs::write(&file_path, test_data).unwrap();

    let temp_archive = TempDir::new().unwrap();
    let archive_path = temp_archive.path().join("encrypted.haf");

    let temp_dest = TempDir::new().unwrap();
    let dest_dir = temp_dest.path();

    let workspace = src_dir.parent().unwrap().to_path_buf();
    let folder_name = src_dir.file_name().unwrap().to_str().unwrap().to_string();
    let target_dir = archive_path.parent().unwrap().to_path_buf();
    let target_name = archive_path
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();

    // 1. 带密码压缩
    let password = "MySecurePassword123".to_string();
    let compressor = Compressor::new(
        workspace,
        huffman_core::CompressorEntries::Selected(vec![ArchiveEntry::Directory {
            relative_path: folder_name.clone(),
            recursive: true,
        }]),
        target_dir.clone(),
        target_name.clone(),
    )
    .with_password(password.clone());

    compressor.compress().unwrap();

    // 2. 无密码解压（应该失败并返回 PasswordRequired）
    let decompressor_no_pwd = Decompressor::new(
        target_dir.clone(),
        target_name.clone(),
        dest_dir.to_path_buf(),
    );
    let err_no_pwd = decompressor_no_pwd.decompress().unwrap_err();
    assert!(
        matches!(
            err_no_pwd,
            huffman_core::error::HuffmanError::PasswordRequired
        ),
        "Expected PasswordRequired, got: {:?}",
        err_no_pwd
    );

    // 3. 错误密码解压（应该失败并返回 PasswordMismatch）
    let decompressor_wrong_pwd = Decompressor::new(
        target_dir.clone(),
        target_name.clone(),
        dest_dir.to_path_buf(),
    )
    .with_password("wrong_password".to_string());
    let err_wrong_pwd = decompressor_wrong_pwd.decompress().unwrap_err();
    assert!(
        matches!(
            err_wrong_pwd,
            huffman_core::error::HuffmanError::PasswordMismatch
        ),
        "Expected PasswordMismatch, got: {:?}",
        err_wrong_pwd
    );

    // 4. 正确密码解压（应该成功）
    let decompressor_correct = Decompressor::new(
        target_dir.clone(),
        target_name.clone(),
        dest_dir.to_path_buf(),
    )
    .with_password(password);
    decompressor_correct.decompress().unwrap();

    // 5. 验证解压出来的文件数据一致
    let restored_file = dest_dir.join(&folder_name).join("secret.txt");
    assert!(restored_file.exists());
    let restored_data = fs::read(restored_file).unwrap();
    assert_eq!(restored_data, test_data);
}

#[test]
fn test_workspace_all_compression_decompression() {
    let temp_src = TempDir::new().unwrap();
    let src_dir = temp_src.path();

    // 写入几个测试文件和子目录
    let file1 = src_dir.join("file1.txt");
    fs::write(&file1, b"hello from file1").unwrap();

    let sub_dir = src_dir.join("dir1");
    fs::create_dir_all(&sub_dir).unwrap();
    let file2 = sub_dir.join("file2.txt");
    fs::write(&file2, b"hello from file2 inside dir1").unwrap();

    // 输出归档文件和解压目标目录
    let temp_archive = TempDir::new().unwrap();
    let archive_path = temp_archive.path().join("workspace_all.haf");

    let temp_dest = TempDir::new().unwrap();
    let dest_dir = temp_dest.path();

    // 执行核心压缩，将 src_dir 作为 workspace，entries 为 CompressorEntries::All
    let workspace = src_dir.to_path_buf();
    let target_dir = archive_path.parent().unwrap().to_path_buf();
    let target_name = archive_path
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();

    let compressor = Compressor::new(
        workspace,
        huffman_core::CompressorEntries::All,
        target_dir,
        target_name,
    );
    compressor.compress().unwrap();

    // 执行核心解压
    let archive_dir = archive_path.parent().unwrap().to_path_buf();
    let archive_name = archive_path
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let decompressor = Decompressor::new(archive_dir, archive_name, dest_dir.to_path_buf());
    decompressor.decompress().unwrap();

    // 验证解压出来的文件数据一致
    let restored_file1 = dest_dir.join("file1.txt");
    assert!(restored_file1.exists());
    assert_eq!(
        fs::read_to_string(restored_file1).unwrap(),
        "hello from file1"
    );

    let restored_file2 = dest_dir.join("dir1/file2.txt");
    assert!(restored_file2.exists());
    assert_eq!(
        fs::read_to_string(restored_file2).unwrap(),
        "hello from file2 inside dir1"
    );
}
