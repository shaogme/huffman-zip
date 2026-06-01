use std::cmp::Reverse;
use std::collections::BinaryHeap;

#[derive(Debug, Eq, PartialEq)]
pub enum HuffmanNode {
    Leaf {
        symbol: u8,
        weight: usize,
    },
    Internal {
        weight: usize,
        left: Box<HuffmanNode>,
        right: Box<HuffmanNode>,
    },
}

impl HuffmanNode {
    pub fn weight(&self) -> usize {
        match self {
            Self::Leaf { weight, .. } => *weight,
            Self::Internal { weight, .. } => *weight,
        }
    }
}

// 实现比较方法以使用 BinaryHeap 进行堆排序
impl Ord for HuffmanNode {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.weight().cmp(&other.weight())
    }
}

impl PartialOrd for HuffmanNode {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// 统计字节流中各个字符的频次
pub fn count_frequencies(data: &[u8]) -> [usize; 256] {
    let mut freqs = [0usize; 256];
    for &byte in data {
        freqs[byte as usize] += 1;
    }
    freqs
}

/// 根据频次表构建普通哈夫曼树
pub fn build_tree(freqs: &[usize; 256]) -> Option<HuffmanNode> {
    let mut heap = BinaryHeap::new();

    // 收集所有频次大于 0 的字符，包装为叶子节点加入最小堆
    for (symbol, &weight) in freqs.iter().enumerate() {
        if weight > 0 {
            heap.push(Reverse(Box::new(HuffmanNode::Leaf {
                symbol: symbol as u8,
                weight,
            })));
        }
    }

    if heap.is_empty() {
        return None;
    }

    // 边缘情况：如果只有一种字符
    if heap.len() == 1 {
        let single = heap.pop().unwrap().0;
        let parent = HuffmanNode::Internal {
            weight: single.weight(),
            left: single,
            right: Box::new(HuffmanNode::Leaf {
                symbol: 0,
                weight: 0,
            }),
        };
        return Some(parent);
    }

    // 经典哈夫曼树构建流程
    while heap.len() > 1 {
        let Reverse(left) = heap.pop().unwrap();
        let Reverse(right) = heap.pop().unwrap();
        let parent = Box::new(HuffmanNode::Internal {
            weight: left.weight() + right.weight(),
            left,
            right,
        });
        heap.push(Reverse(parent));
    }

    Some(*heap.pop().unwrap().0)
}

/// DFS 遍历哈夫曼树以获取每个字符的分配码长
pub fn get_code_lengths(root: &HuffmanNode) -> Vec<(u8, u8)> {
    let mut lengths = Vec::new();
    fn dfs(node: &HuffmanNode, depth: u8, lengths: &mut Vec<(u8, u8)>) {
        match node {
            HuffmanNode::Leaf { symbol, weight } => {
                if *weight > 0 {
                    lengths.push((*symbol, depth));
                }
            }
            HuffmanNode::Internal { left, right, .. } => {
                dfs(left, depth + 1, lengths);
                dfs(right, depth + 1, lengths);
            }
        }
    }
    dfs(root, 0, &mut lengths);
    lengths
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_frequencies() {
        let freqs = [0usize; 256];
        assert!(build_tree(&freqs).is_none());
    }

    #[test]
    fn test_single_symbol() {
        let mut freqs = [0usize; 256];
        freqs[b'A' as usize] = 10;
        let tree = build_tree(&freqs).unwrap();
        let lengths = get_code_lengths(&tree);
        assert_eq!(lengths.len(), 1);
        assert_eq!(lengths[0], (b'A', 1));
    }

    #[test]
    fn test_multiple_symbols() {
        let data = b"AABBBCCCC"; // A:2, B:3, C:4
        let freqs = count_frequencies(data);
        let tree = build_tree(&freqs).unwrap();
        let mut lengths = get_code_lengths(&tree);
        lengths.sort_by_key(|k| k.0);

        // C 的频次最高，A 最低
        // 预期的深度结构应当是 C 占短编码，A 和 B 占深一层
        // 例如：C 的长度为 1，A 和 B 的长度为 2
        for &(sym, len) in &lengths {
            if sym == b'C' {
                assert_eq!(len, 1);
            } else {
                assert_eq!(len, 2);
            }
        }
    }
}
