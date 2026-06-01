use crate::bitstream::BitReader;
use crate::error::HuffmanError;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CanonicalCode {
    pub code: u128,
    pub len: u8,
}

/// 范式哈夫曼树的紧凑数据表表示
pub struct CanonicalTree {
    pub code_lengths: [u8; 256],
}

impl CanonicalTree {
    /// 从原始码长列表构造范式表
    pub fn new(lengths: Vec<(u8, u8)>) -> Self {
        let mut code_lengths = [0u8; 256];
        for (sym, len) in lengths {
            code_lengths[sym as usize] = len;
        }
        Self { code_lengths }
    }

    /// 序列化为 HAF 格式的字节流（Compressed Tree）
    pub fn serialize(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        let active: Vec<(u8, u8)> = self
            .code_lengths
            .iter()
            .enumerate()
            .filter(|&(_, &len)| len > 0)
            .map(|(sym, &len)| (sym as u8, len))
            .collect();

        let n = active.len();
        if n == 0 {
            return buf;
        }

        // 用 N-1 存储，表示 1..=256 个字符数量
        buf.push((n - 1) as u8);
        for &(sym, len) in &active {
            buf.push(sym);
            buf.push(len);
        }
        buf
    }

    /// 从字节序列反序列化出范式码表
    pub fn deserialize(buf: &[u8]) -> Result<Self, HuffmanError> {
        if buf.is_empty() {
            return Ok(Self {
                code_lengths: [0u8; 256],
            });
        }
        let n_val = buf[0] as usize;
        let n = n_val + 1;

        if buf.len() < 1 + 2 * n {
            return Err(HuffmanError::CorruptedArchive(
                "Compressed tree buffer size is corrupted",
            ));
        }

        let mut code_lengths = [0u8; 256];
        let mut idx = 1;
        for _ in 0..n {
            let sym = buf[idx];
            let len = buf[idx + 1];
            if len > 127 {
                return Err(HuffmanError::CorruptedArchive(
                    "Invalid code length in tree",
                ));
            }
            code_lengths[sym as usize] = len;
            idx += 2;
        }

        Ok(Self { code_lengths })
    }

    /// 生成范式哈夫曼编码字典
    pub fn generate_encoder_table(&self) -> [Option<CanonicalCode>; 256] {
        let mut sorted_symbols = Vec::new();
        for (sym, &len) in self.code_lengths.iter().enumerate() {
            if len > 0 {
                sorted_symbols.push((sym as u8, len));
            }
        }

        // 根据码长升序，若码长相同，按符号 ASCII 升序
        sorted_symbols.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));

        let mut num_codes = [0usize; 256];
        for &(_, len) in &sorted_symbols {
            num_codes[len as usize] += 1;
        }

        let mut next_code = [0u128; 256];
        let mut current_code = 0u128;
        for i in 1..256 {
            current_code = (current_code + num_codes[i - 1] as u128) << 1;
            next_code[i] = current_code;
        }

        let mut encoder_table = [const { None }; 256];
        for &(sym, len) in &sorted_symbols {
            let code_val = next_code[len as usize];
            next_code[len as usize] += 1;
            encoder_table[sym as usize] = Some(CanonicalCode {
                code: code_val,
                len,
            });
        }

        encoder_table
    }

    /// 在解压端重构解码查找树
    pub fn build_decode_tree(&self) -> DecodeNode {
        let encoder_table = self.generate_encoder_table();
        let mut root = DecodeNode::new();
        for (sym, code_opt) in encoder_table.iter().enumerate() {
            if let Some(code) = code_opt {
                root.insert(code.code, code.len, sym as u8);
            }
        }
        root
    }
}

/// 解码树节点，用于在解压时前缀码的快速比特流匹配
pub struct DecodeNode {
    pub symbol: Option<u8>,
    pub left: Option<Box<DecodeNode>>,
    pub right: Option<Box<DecodeNode>>,
}

impl DecodeNode {
    pub fn new() -> Self {
        Self {
            symbol: None,
            left: None,
            right: None,
        }
    }

    pub fn insert(&mut self, code: u128, len: u8, symbol: u8) {
        let mut curr = self;
        for i in (0..len).rev() {
            let bit = ((code >> i) & 1) != 0;
            if bit {
                if curr.right.is_none() {
                    curr.right = Some(Box::new(DecodeNode::new()));
                }
                curr = curr.right.as_mut().unwrap();
            } else {
                if curr.left.is_none() {
                    curr.left = Some(Box::new(DecodeNode::new()));
                }
                curr = curr.left.as_mut().unwrap();
            }
        }
        curr.symbol = Some(symbol);
    }

    /// 从比特流读取器中解码出一个字节
    pub fn decode_byte<R: std::io::Read>(&self, reader: &mut BitReader<R>) -> std::io::Result<u8> {
        let mut curr = self;
        loop {
            if let Some(sym) = curr.symbol {
                return Ok(sym);
            }
            let bit = reader.read_bit()?;
            if bit {
                let Some(ref right) = curr.right else {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "Invalid Huffman code during bit decoding",
                    ));
                };
                curr = right;
            } else {
                let Some(ref left) = curr.left else {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "Invalid Huffman code during bit decoding",
                    ));
                };
                curr = left;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_canonical_generation() {
        // 构建包含 A, B, C 的测试频次码长
        // A: 2, B: 1, C: 2
        let lengths = vec![(b'A', 2), (b'B', 1), (b'C', 2)];
        let tree = CanonicalTree::new(lengths);
        let table = tree.generate_encoder_table();

        // 预期编码：
        // B: 0 (len 1)
        // A: 2 (len 2) -> 10
        // C: 3 (len 2) -> 11
        assert_eq!(
            table[b'B' as usize],
            Some(CanonicalCode { code: 0, len: 1 })
        );
        assert_eq!(
            table[b'A' as usize],
            Some(CanonicalCode { code: 2, len: 2 })
        );
        assert_eq!(
            table[b'C' as usize],
            Some(CanonicalCode { code: 3, len: 2 })
        );
    }

    #[test]
    fn test_serialization_roundtrip() {
        let lengths = vec![(b'X', 3), (b'Y', 1), (b'Z', 4)];
        let original_tree = CanonicalTree::new(lengths);

        let bytes = original_tree.serialize();
        let recovered_tree = CanonicalTree::deserialize(&bytes).unwrap();

        assert_eq!(recovered_tree.code_lengths[b'X' as usize], 3);
        assert_eq!(recovered_tree.code_lengths[b'Y' as usize], 1);
        assert_eq!(recovered_tree.code_lengths[b'Z' as usize], 4);
        assert_eq!(recovered_tree.code_lengths[b'A' as usize], 0);
    }
}
