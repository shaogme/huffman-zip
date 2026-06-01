use chacha20::ChaCha20;
use chacha20::cipher::{KeyIvInit, StreamCipher};
use pbkdf2::hmac::Hmac;
use pbkdf2::pbkdf2;
use sha2::Sha256;
use std::io::{self, Read, Write};

/// 从密码和 salt 中派生出 32 字节的 AES/ChaCha20 密钥
pub fn derive_key(password: &str, salt: &[u8; 16]) -> [u8; 32] {
    let mut key = [0u8; 32];
    // 使用 PBKDF2-HMAC-SHA256 算法，进行 100,000 次迭代
    pbkdf2::<Hmac<Sha256>>(password.as_bytes(), salt, 100_000, &mut key)
        .expect("PBKDF2 key derivation failed");
    key
}

/// 支持流式透明加密的写入包装器
pub struct EncryptWriter<W: Write> {
    inner: W,
    cipher: ChaCha20,
}

impl<W: Write> EncryptWriter<W> {
    pub fn new(inner: W, key: &[u8; 32], nonce: &[u8; 12]) -> Self {
        let cipher = ChaCha20::new(key.into(), nonce.into());
        Self { inner, cipher }
    }

    /// 消费掉包装器，返回底层的写入器
    pub fn into_inner(self) -> W {
        self.inner
    }
}

impl<W: Write> Write for EncryptWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut temp = buf.to_vec();
        self.cipher.apply_keystream(&mut temp);
        self.inner.write_all(&temp)?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// 支持流式透明解密的读取包装器
pub struct DecryptReader<R: Read> {
    inner: R,
    cipher: ChaCha20,
}

impl<R: Read> DecryptReader<R> {
    pub fn new(inner: R, key: &[u8; 32], nonce: &[u8; 12]) -> Self {
        let cipher = ChaCha20::new(key.into(), nonce.into());
        Self { inner, cipher }
    }

    /// 消费掉包装器，返回底层的读取器
    pub fn into_inner(self) -> R {
        self.inner
    }
}

impl<R: Read> Read for DecryptReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        if n > 0 {
            self.cipher.apply_keystream(&mut buf[..n]);
        }
        Ok(n)
    }
}
