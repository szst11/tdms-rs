use std::io::{self, BufReader, Read, Seek, SeekFrom};

const INITIAL_READ: usize = 64 * 1024;
const SPARSE_READ: usize = 8 * 1024;
const MAX_READ: usize = 1024 * 1024;
const DENSE_SEGMENTS: u8 = 4;

struct ReadAhead<R> {
    inner: R,
    limit: usize,
}

impl<R: Read> Read for ReadAhead<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let len = buf.len().min(self.limit);
        self.inner.read(&mut buf[..len])
    }
}

impl<R: Seek> Seek for ReadAhead<R> {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        self.inner.seek(from)
    }
}

/// Tracks the logical position without querying the underlying file handle.
pub(crate) struct PositionedReader<R> {
    reader: BufReader<ReadAhead<R>>,
    position: u64,
    dense_segments: u8,
}

impl<R: Read + Seek> PositionedReader<R> {
    /// The supplied stream must start at offset zero.
    pub(crate) fn new(reader: R) -> Self {
        Self {
            // Change refill size, not the buffer itself, so unread bytes survive
            // dense/sparse transitions without seeking or copying.
            reader: BufReader::with_capacity(
                MAX_READ,
                ReadAhead {
                    inner: reader,
                    limit: INITIAL_READ,
                },
            ),
            position: 0,
            dense_segments: 0,
        }
    }

    pub(crate) fn new_index(reader: R) -> Self {
        let mut result = Self::new(reader);
        result.reader.get_mut().limit = MAX_READ;
        result
    }

    /// Observe only data-bearing segments with a known next-segment offset.
    pub(crate) fn observe_raw_gap(&mut self, gap: u64) {
        if gap == 0 {
            return;
        }
        if gap > INITIAL_READ as u64 {
            self.dense_segments = 0;
            self.reader.get_mut().limit = SPARSE_READ;
        } else {
            self.dense_segments = self.dense_segments.saturating_add(1).min(DENSE_SEGMENTS);
            if self.dense_segments == DENSE_SEGMENTS {
                self.reader.get_mut().limit = MAX_READ;
            }
        }
    }
}

impl<R: Read> Read for PositionedReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let count = self.reader.read(buf)?;
        self.position = self
            .position
            .checked_add(count as u64)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "read position overflow"))?;
        Ok(count)
    }
}

impl<R: Read + Seek> Seek for PositionedReader<R> {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let target = match from {
            SeekFrom::Start(position) => position,
            SeekFrom::Current(offset) => {
                self.position.checked_add_signed(offset).ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidInput, "seek position out of range")
                })?
            }
            SeekFrom::End(offset) => {
                self.position = self.reader.seek(SeekFrom::End(offset))?;
                return Ok(self.position);
            }
        };
        if target != self.position {
            let distance = i128::from(target) - i128::from(self.position);
            if let Ok(offset) = i64::try_from(distance) {
                self.reader.seek_relative(offset)?;
            } else {
                self.reader.seek(SeekFrom::Start(target))?;
            }
            self.position = target;
        }
        Ok(self.position)
    }

    fn stream_position(&mut self) -> io::Result<u64> {
        Ok(self.position)
    }
}
