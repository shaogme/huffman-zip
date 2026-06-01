# Huffman DLL - 跨语言哈夫曼压缩/解压缩动态链接库

本项目为 `huffman-core` 库的 C 兼容 FFI（外部函数接口）封装，旨在将哈夫曼压缩（`compress`）与解压缩（`decompress`）功能打包为动态链接库（DLL），以便于 C、C++、C#、Python、Go 等多语言进行无缝调用。

---

## 目录
- [1. 如何构建 DLL](#1-如何构建-dll)
- [2. 配置结构体详解](#2-配置结构体详解)
  - [2.1 CArchiveEntryType (条目类型)](#21-carchiveentrytype-条目类型)
  - [2.2 CArchiveEntry (单个归档条目)](#22-carchiveentry-单个归档条目)
  - [2.3 CCompressorConfig (压缩配置)](#23-ccompressorconfig-压缩配置)
  - [2.4 CDecompressorConfig (解压配置)](#24-cdecompressorconfig-解压配置)
- [3. FFI 接口说明](#3-ffi-接口说明)
  - [3.1 HuffmanResult (返回值状态码)](#31-huffmanresult-返回值状态码)
  - [3.2 huffman_compress (压缩接口)](#32-huffman_compress-压缩接口)
  - [3.3 huffman_decompress (解压接口)](#33-huffman_decompress-解压接口)
- [4. C/C++ 完整调用示例](#4-cc-完整调用示例)

---

## 1. 如何构建 DLL

动态链接库的构建依赖于 Rust 工具链。请确保系统已安装 [Rust 和 Cargo](https://rustup.rs/)。

### 构建命令
在项目根目录下或 `huffman-dll` 子目录下执行以下命令：

```bash
# 构建 Release 版本动态链接库
cargo build --release -p huffman-dll
```

### 构建输出位置
根据编译的目标操作系统，生成的动态链接库文件将输出在以下位置：
- **Windows**: `target/release/huffman_dll.dll` (及对应的导入库 `huffman_dll.dll.lib`，取决于编译器工具链如 MSVC)
- **Linux**: `target/release/libhuffman_dll.so`
- **macOS**: `target/release/libhuffman_dll.dylib`

> [!NOTE]
> 在其他语言中加载并调用该 DLL 时，请务必保证 DLL 文件所在的路径在系统的环境变量中，或者通过绝对路径加载该库。

---

## 2. 配置结构体详解

DLL 接口中使用的全部数据结构均符合 C ABI（使用 `#[repr(C)]` 导出），其具体定义及字段含义如下：

### 2.1 CArchiveEntryType (条目类型)
一个枚举值，用于区分归档项目是文件还是目录。

```rust
#[repr(C)]
pub enum CArchiveEntryType {
    File = 0,       // 文件条目
    Directory = 1,  // 目录条目
}
```

### 2.2 CArchiveEntry (单个归档条目)
表示待归档的单个目标。

| 字段名 | 类型 | 说明 |
| :--- | :--- | :--- |
| `entry_type` | `CArchiveEntryType` | 条目类型，指明是 `File` 还是 `Directory`。 |
| `relative_path` | `*const c_char` | 目标条目在工作区目录（`workspace`）中的相对路径（以 `\0` 结尾的 C 字符串，建议使用 `/` 分割）。 |
| `name` | `*const c_char` | 归档条目在压缩包内的文件名或文件夹名。仅在 `entry_type` 为 `File` 时有效；若为 `Directory`，则该字段传入空指针即可。 |
| `recursive` | `bool` | 是否递归扫描。仅在 `entry_type` 为 `Directory` 时有效，指示是否递归扫描该目录下的所有子文件和文件夹。 |

### 2.3 CCompressorConfig (压缩配置)
控制压缩流程的配置结构体。

| 字段名 | 类型 | 说明 |
| :--- | :--- | :--- |
| `workspace` | `*const c_char` | 待压缩的工作区根目录路径（以 `\0` 结尾的 C 字符串）。 |
| `entries` | `*const CArchiveEntry` | 指向包含 `CArchiveEntry` 数组首元素的指针。 |
| `entries_count` | `usize` | `entries` 数组中元素的个数。若此字段为 `0` 且 `entries` 指针为空，则系统默认**全量压缩**整个 `workspace` 目录。 |
| `target_dir` | `*const c_char` | 生成的压缩包所保存的目标输出目录路径（以 `\0` 结尾的 C 字符串）。 |
| `target_name` | `*const c_char` | 压缩包的文件名（含后缀，如 `archive.haf`，以 `\0` 结尾的 C 字符串）。 |
| `password` | `*const c_char` | 压缩加密密码（以 `\0` 结尾的 C 字符串）。若无需加密，请传入**空指针（NULL/nullptr）**。 |
| `overwrite` | `bool` | 若目标同名压缩包已存在，指示是否覆盖该文件。`true` 表示直接覆盖，`false` 表示不覆盖并返回 `AlreadyExists` (-7) 错误。 |

### 2.4 CDecompressorConfig (解压配置)
控制解压缩流程的配置结构体。

| 字段名 | 类型 | 说明 |
| :--- | :--- | :--- |
| `archive_dir` | `*const c_char` | 待解压的 `HAF` 压缩包文件所在的目录路径（以 `\0` 结尾的 C 字符串）。 |
| `archive_name` | `*const c_char` | 压缩包的文件名（以 `\0` 结尾的 C 字符串）。 |
| `target_dir` | `*const c_char` | 解压文件及文件夹所放置的目标输出目录路径（以 `\0` 结尾的 C 字符串）。 |
| `password` | `*const c_char` | 解密密码（以 `\0` 结尾的 C 字符串）。若压缩包未加密，请传入**空指针（NULL/nullptr）**。 |

---

## 3. FFI 接口说明

### 3.1 HuffmanResult (返回值状态码)
调用 API 时，所有接口均会返回 `c_int`。你可以将其强制转换为以下状态码：

| 状态码名称 | 整数值 | 说明 |
| :--- | :--- | :--- |
| `Success` | `0` | 操作成功完成。 |
| `InvalidPath` | `-1` | 传入的路径不合法或在转换时失败。 |
| `IoError` | `-2` | 系统底层发生 I/O 错误（如磁盘读写失败，目录不存在等）。 |
| `NullPointer` | `-3` | 必填的参数传入了空指针（如配置指针或必须存在的 C 字符串指针为 NULL）。 |
| `InvalidArchive` | `-4` | 非法的 HAF 归档文件（可能文件损坏，魔数不对）。 |
| `PasswordRequired` | `-5` | 该压缩包已加密，必须提供解密密码。 |
| `PasswordMismatch` | `-6` | 解密密码错误。 |
| `AlreadyExists` | `-7` | 目标项（已解压的目标文件或压缩包目标路径）已存在，且配置为不进行覆盖。 |
| `PanicTriggered` | `-99` | 内部 Rust 代码触发了异常（Panic），但已被安全捕获，未导致宿主进程崩溃。 |

---

### 3.2 huffman_compress (压缩接口)

```rust
#[unsafe(no_mangle)]
pub unsafe extern "C" fn huffman_compress(config: *const CCompressorConfig) -> c_int
```

- **功能说明**: 根据传入的 `CCompressorConfig` 结构体进行哈夫曼压缩。
- **线程安全性**: 线程安全。
- **注意事项**: 传入的指针及结构体内所有字符指针生命周期必须覆盖函数调用期，该函数在返回前不会保留对指针的引用。

### 3.3 huffman_decompress (解压接口)

```rust
#[unsafe(no_mangle)]
pub unsafe extern "C" fn huffman_decompress(config: *const CDecompressorConfig) -> c_int
```

- **功能说明**: 根据传入的 `CDecompressorConfig` 结构体进行哈夫曼解压缩。
- **线程安全性**: 线程安全。
- **安全特性**: 内置**路径穿越（Path Traversal）**安全校验。若归档文件中包含类似 `../` 的越权解压路径，接口将返回 `-4 (InvalidArchive)` 错误并阻止解压。

---

## 4. C/C++ 完整调用示例

以下是使用 C 语言调用本动态链接库的完整代码示例：

```c
#include <stdio.h>
#include <stdbool.h>

// 1. 定义状态码
typedef enum {
    HUFFMAN_SUCCESS = 0,
    HUFFMAN_INVALID_PATH = -1,
    HUFFMAN_IO_ERROR = -2,
    HUFFMAN_NULL_POINTER = -3,
    HUFFMAN_INVALID_ARCHIVE = -4,
    HUFFMAN_PASSWORD_REQUIRED = -5,
    HUFFMAN_PASSWORD_MISMATCH = -6,
    HUFFMAN_ALREADY_EXISTS = -7,
    HUFFMAN_PANIC_TRIGGERED = -99
} HuffmanResult;

// 2. 声明 CArchiveEntry 及其关联项
typedef enum {
    ENTRY_FILE = 0,
    ENTRY_DIRECTORY = 1
} CArchiveEntryType;

typedef struct {
    CArchiveEntryType entry_type;
    const char* relative_path;
    const char* name;
    bool recursive;
} CArchiveEntry;

// 3. 声明配置结构体
typedef struct {
    const char* workspace;
    const CArchiveEntry* entries;
    size_t entries_count;
    const char* target_dir;
    const char* target_name;
    const char* password;
    bool overwrite;
} CCompressorConfig;

typedef struct {
    const char* archive_dir;
    const char* archive_name;
    const char* target_dir;
    const char* password;
} CDecompressorConfig;

// 4. 声明 FFI 函数（由 DLL 导出）
#ifdef __cplusplus
extern "C" {
#endif

int huffman_compress(const CCompressorConfig* config);
int huffman_decompress(const CDecompressorConfig* config);

#ifdef __cplusplus
}
#endif

int main() {
    // ----------------------------------------------------
    // 示例一：选定文件进行加密压缩
    // ----------------------------------------------------
    CArchiveEntry entry;
    entry.entry_type = ENTRY_FILE;
    entry.relative_path = "hello.txt"; // workspace 中的相对路径
    entry.name = "hello.txt";
    entry.recursive = false;

    CCompressorConfig comp_config;
    comp_config.workspace = "C:\\path\\to\\workspace";
    comp_config.entries = &entry;
    comp_config.entries_count = 1;
    comp_config.target_dir = "C:\\path\\to\\output";
    comp_config.target_name = "my_archive.haf";
    comp_config.password = "my_secure_password"; // 传 NULL 则为不加密
    comp_config.overwrite = true; // 是否覆盖已有同名压缩包

    printf("开始加密压缩...\n");
    int comp_res = huffman_compress(&comp_config);
    if (comp_res == HUFFMAN_SUCCESS) {
        printf("压缩成功！生成文件：C:\\path\\to\\output\\my_archive.haf\n");
    } else {
        printf("压缩失败，错误码为: %d\n", comp_res);
        return comp_res;
    }

    // ----------------------------------------------------
    // 示例二：对生成的压缩包进行解密与解压缩
    // ----------------------------------------------------
    CDecompressorConfig decomp_config;
    decomp_config.archive_dir = "C:\\path\\to\\output";
    decomp_config.archive_name = "my_archive.haf";
    decomp_config.target_dir = "C:\\path\\to\\decompressed";
    decomp_config.password = "my_secure_password"; // 解密密码必须匹配

    printf("开始解密解压缩...\n");
    int decomp_res = huffman_decompress(&decomp_config);
    if (decomp_res == HUFFMAN_SUCCESS) {
        printf("解压成功！文件已还原至：C:\\path\\to\\decompressed\n");
    } else if (decomp_res == HUFFMAN_PASSWORD_MISMATCH) {
        printf("解压失败：密码不匹配！\n");
    } else {
        printf("解压失败，错误码为: %d\n", decomp_res);
    }

    return 0;
}
```
