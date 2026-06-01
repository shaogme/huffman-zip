use std::ffi::CStr;
use std::os::raw::c_char;
use std::path::PathBuf;

/// 安全地将外部空字符结尾的 C 字符串转换为 PathBuf
pub unsafe fn convert_c_str(ptr: *const c_char) -> Option<PathBuf> {
    if ptr.is_null() {
        return None;
    }
    let c_str = unsafe { CStr::from_ptr(ptr) };
    let str_slice = c_str.to_str().ok()?;
    Some(PathBuf::from(str_slice))
}

/// 安全地将外部 C 字符串指针数组转换为 PathBuf 向量
pub unsafe fn convert_c_str_array(ptr: *const *const c_char, count: usize) -> Option<Vec<PathBuf>> {
    if ptr.is_null() && count > 0 {
        return None;
    }
    let mut paths = Vec::with_capacity(count);
    for i in 0..count {
        unsafe {
            let path_ptr = *ptr.add(i);
            // 充分使用 Rust 2024 的 let-else 语法，防止过度嵌套与猜测
            let Some(path) = convert_c_str(path_ptr) else {
                return None;
            };
            paths.push(path);
        }
    }
    Some(paths)
}

/// 安全地将外部空字符结尾的 C 字符串转换为 String
pub unsafe fn convert_c_str_to_string(ptr: *const c_char) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    let c_str = unsafe { CStr::from_ptr(ptr) };
    let str_slice = c_str.to_str().ok()?;
    Some(str_slice.to_string())
}
