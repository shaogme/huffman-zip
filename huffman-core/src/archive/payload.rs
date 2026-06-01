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

enum FileContentReader {
    Disk(std::fs::File),
    Memory(std::io::Cursor<Vec<u8>>),
    Empty,
}

impl std::io::Read for FileContentReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Self::Disk(f) => f.read(buf),
            Self::Memory(c) => c.read(buf),
            Self::Empty => Ok(0),
        }
    }
}

pub struct PayloadStream<'a> {
    entries: &'a [PayloadEntry],
    current_entry_idx: usize,
    state: StreamState,
}

enum StreamState {
    EntryCount {
        bytes: [u8; 4],
        offset: usize,
    },
    EntryType {
        entry_type: u8,
    },
    PathLen {
        bytes: [u8; 4],
        offset: usize,
    },
    PathBytes {
        bytes: Vec<u8>,
        offset: usize,
    },
    FileSize {
        bytes: [u8; 8],
        offset: usize,
    },
    FileContent {
        reader: FileContentReader,
        bytes_remaining: u64,
    },
    Done,
}

impl<'a> PayloadStream<'a> {
    pub fn new(entries: &'a [PayloadEntry]) -> Self {
        Self {
            entries,
            current_entry_idx: 0,
            state: StreamState::EntryCount {
                bytes: (entries.len() as u32).to_be_bytes(),
                offset: 0,
            },
        }
    }

    fn transition_to_entry(&mut self, idx: usize) -> StreamState {
        if idx < self.entries.len() {
            self.current_entry_idx = idx;
            let entry = &self.entries[idx];
            let entry_type = match entry {
                PayloadEntry::File { .. } => 0x01,
                PayloadEntry::Directory { .. } => 0x02,
            };
            StreamState::EntryType { entry_type }
        } else {
            StreamState::Done
        }
    }
}

impl<'a> std::io::Read for PayloadStream<'a> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }

        let mut bytes_written = 0;

        while bytes_written < buf.len() {
            match &mut self.state {
                StreamState::EntryCount { bytes, offset } => {
                    let rem = bytes.len() - *offset;
                    let to_write = std::cmp::min(rem, buf.len() - bytes_written);
                    buf[bytes_written..bytes_written + to_write]
                        .copy_from_slice(&bytes[*offset..*offset + to_write]);
                    *offset += to_write;
                    bytes_written += to_write;
                    if *offset == bytes.len() {
                        self.state = self.transition_to_entry(0);
                    }
                }
                StreamState::EntryType { entry_type } => {
                    buf[bytes_written] = *entry_type;
                    bytes_written += 1;
                    let entry = &self.entries[self.current_entry_idx];
                    let relative_path = match entry {
                        PayloadEntry::File { relative_path, .. } => relative_path,
                        PayloadEntry::Directory { relative_path } => relative_path,
                    };
                    self.state = StreamState::PathLen {
                        bytes: (relative_path.len() as u32).to_be_bytes(),
                        offset: 0,
                    };
                }
                StreamState::PathLen { bytes, offset } => {
                    let rem = bytes.len() - *offset;
                    let to_write = std::cmp::min(rem, buf.len() - bytes_written);
                    buf[bytes_written..bytes_written + to_write]
                        .copy_from_slice(&bytes[*offset..*offset + to_write]);
                    *offset += to_write;
                    bytes_written += to_write;
                    if *offset == bytes.len() {
                        let entry = &self.entries[self.current_entry_idx];
                        let relative_path = match entry {
                            PayloadEntry::File { relative_path, .. } => relative_path,
                            PayloadEntry::Directory { relative_path } => relative_path,
                        };
                        self.state = StreamState::PathBytes {
                            bytes: relative_path.as_bytes().to_vec(),
                            offset: 0,
                        };
                    }
                }
                StreamState::PathBytes { bytes, offset } => {
                    let rem = bytes.len() - *offset;
                    let to_write = std::cmp::min(rem, buf.len() - bytes_written);
                    buf[bytes_written..bytes_written + to_write]
                        .copy_from_slice(&bytes[*offset..*offset + to_write]);
                    *offset += to_write;
                    bytes_written += to_write;
                    if *offset == bytes.len() {
                        let entry = &self.entries[self.current_entry_idx];
                        match entry {
                            PayloadEntry::File { file_size, .. } => {
                                self.state = StreamState::FileSize {
                                    bytes: file_size.to_be_bytes(),
                                    offset: 0,
                                };
                            }
                            PayloadEntry::Directory { .. } => {
                                let next_idx = self.current_entry_idx + 1;
                                self.state = self.transition_to_entry(next_idx);
                            }
                        }
                    }
                }
                StreamState::FileSize { bytes, offset } => {
                    let rem = bytes.len() - *offset;
                    let to_write = std::cmp::min(rem, buf.len() - bytes_written);
                    buf[bytes_written..bytes_written + to_write]
                        .copy_from_slice(&bytes[*offset..*offset + to_write]);
                    *offset += to_write;
                    bytes_written += to_write;
                    if *offset == bytes.len() {
                        let entry = &self.entries[self.current_entry_idx];
                        if let PayloadEntry::File {
                            source_path,
                            raw_data,
                            file_size,
                            ..
                        } = entry
                        {
                            let reader = if let Some(path) = source_path {
                                FileContentReader::Disk(std::fs::File::open(path)?)
                            } else if let Some(data) = raw_data {
                                FileContentReader::Memory(std::io::Cursor::new(data.clone()))
                            } else {
                                FileContentReader::Empty
                            };
                            self.state = StreamState::FileContent {
                                reader,
                                bytes_remaining: *file_size,
                            };
                        } else {
                            unreachable!();
                        }
                    }
                }
                StreamState::FileContent {
                    reader,
                    bytes_remaining,
                } => {
                    if *bytes_remaining == 0 {
                        let next_idx = self.current_entry_idx + 1;
                        self.state = self.transition_to_entry(next_idx);
                        continue;
                    }
                    let rem = *bytes_remaining as usize;
                    let max_to_read = std::cmp::min(rem, buf.len() - bytes_written);
                    let n = reader.read(&mut buf[bytes_written..bytes_written + max_to_read])?;
                    if n == 0 {
                        let to_pad = max_to_read;
                        for b in &mut buf[bytes_written..bytes_written + to_pad] {
                            *b = 0;
                        }
                        *bytes_remaining -= to_pad as u64;
                        bytes_written += to_pad;
                    } else {
                        *bytes_remaining -= n as u64;
                        bytes_written += n;
                    }
                    if *bytes_remaining == 0 {
                        let next_idx = self.current_entry_idx + 1;
                        self.state = self.transition_to_entry(next_idx);
                    }
                }
                StreamState::Done => {
                    break;
                }
            }
        }

        Ok(bytes_written)
    }
}