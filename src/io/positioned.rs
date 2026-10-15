use std::io::{self, BufReader, Read, Seek, SeekFrom};

/// Tracks the logical position without querying the underlying file handle.
pub(crate) struct PositionedReader<R> {
    reader: BufReader<R>,
    position: u64,
}

impl<R: Read + Seek> PositionedReader<R> {
    /// The supplied stream must start at offset zero.
    pub(crate) fn new(reader: R) -> Self {
        Self {
            reader: BufReader::new(reader),
            position: 0,
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
