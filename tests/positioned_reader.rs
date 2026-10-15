#[path = "../src/io/positioned.rs"]
mod positioned;

use positioned::PositionedReader;
use std::cell::Cell;
use std::io::{self, Cursor, Read, Seek, SeekFrom};
use std::rc::Rc;

struct CountedStream {
    data: Cursor<Vec<u8>>,
    reads: Rc<Cell<usize>>,
    seeks: Rc<Cell<usize>>,
    interrupt: bool,
    short_reads: bool,
}

impl Read for CountedStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.reads.set(self.reads.get() + 1);
        if self.interrupt {
            self.interrupt = false;
            return Err(io::ErrorKind::Interrupted.into());
        }
        let count = if self.short_reads {
            buf.len().min(3)
        } else {
            buf.len()
        };
        self.data.read(&mut buf[..count])
    }
}

impl Seek for CountedStream {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        self.seeks.set(self.seeks.get() + 1);
        self.data.seek(from)
    }
}

fn stream(data: Vec<u8>) -> (CountedStream, Rc<Cell<usize>>, Rc<Cell<usize>>) {
    let reads = Rc::new(Cell::new(0));
    let seeks = Rc::new(Cell::new(0));
    (
        CountedStream {
            data: Cursor::new(data),
            reads: reads.clone(),
            seeks: seeks.clone(),
            interrupt: false,
            short_reads: false,
        },
        reads,
        seeks,
    )
}

#[test]
fn positions_and_buffered_skips_do_not_seek_the_file() -> io::Result<()> {
    let (source, reads, seeks) = stream((0..100).collect());
    let mut reader = PositionedReader::new(source);
    let mut value = [0];
    reader.read_exact(&mut value)?;
    assert_eq!(value, [0]);
    assert_eq!(reader.stream_position()?, 1);
    assert_eq!(reader.seek(SeekFrom::Start(20))?, 20);
    reader.read_exact(&mut value)?;
    assert_eq!(value, [20]);
    assert_eq!(reader.seek(SeekFrom::Current(-11))?, 10);
    reader.read_exact(&mut value)?;
    assert_eq!(value, [10]);
    assert_eq!(reader.seek(SeekFrom::Current(1))?, 12);
    assert_eq!(reader.seek(SeekFrom::Start(11))?, 11);
    assert_eq!(reader.seek(SeekFrom::Start(11))?, 11);
    assert_eq!(reader.stream_position()?, 11);
    assert_eq!(reads.get(), 1);
    assert_eq!(seeks.get(), 0);
    Ok(())
}

#[test]
fn skips_outside_buffer_and_end_relative_seeks() -> io::Result<()> {
    let (source, _, seeks) = stream(vec![7; 30_000]);
    let mut reader = PositionedReader::new(source);
    let mut value = [0; 10];
    reader.read_exact(&mut value)?;
    assert_eq!(reader.seek(SeekFrom::Start(20_000))?, 20_000);
    assert_eq!(seeks.get(), 1);
    reader.read_exact(&mut value)?;
    assert_eq!(reader.stream_position()?, 20_010);
    assert_eq!(reader.seek(SeekFrom::Start(5))?, 5);
    assert_eq!(reader.seek(SeekFrom::End(-3))?, 29_997);
    assert_eq!(reader.read(&mut value)?, 3);
    assert_eq!(reader.stream_position()?, 30_000);
    assert_eq!(reader.read(&mut value)?, 0);
    assert_eq!(reader.stream_position()?, 30_000);
    assert_eq!(seeks.get(), 3);
    Ok(())
}

#[test]
fn short_reads_interruption_and_partial_eof_track_consumed_bytes() -> io::Result<()> {
    let (mut source, _, _) = stream(b"abcdefgh".to_vec());
    source.interrupt = true;
    source.short_reads = true;
    let mut reader = PositionedReader::new(source);
    let mut first = [0; 5];
    reader.read_exact(&mut first)?;
    assert_eq!(&first, b"abcde");
    assert_eq!(reader.stream_position()?, 5);
    let mut tail = [0; 4];
    assert_eq!(
        reader.read_exact(&mut tail).unwrap_err().kind(),
        io::ErrorKind::UnexpectedEof,
    );
    assert_eq!(reader.stream_position()?, 8);
    assert_eq!(&tail[..3], b"fgh");
    assert_eq!(reader.seek(SeekFrom::Start(0))?, 0);
    reader.read_exact(&mut first)?;
    assert_eq!(&first, b"abcde");
    Ok(())
}

#[test]
fn invalid_relative_seek_does_not_change_position() -> io::Result<()> {
    let (source, _, seeks) = stream(vec![1, 2, 3]);
    let mut reader = PositionedReader::new(source);
    let mut value = [0];
    reader.read_exact(&mut value)?;
    assert_eq!(
        reader.seek(SeekFrom::Current(-2)).unwrap_err().kind(),
        io::ErrorKind::InvalidInput,
    );
    assert_eq!(reader.stream_position()?, 1);
    reader.read_exact(&mut value)?;
    assert_eq!(reader.stream_position()?, 2);
    assert_eq!(seeks.get(), 0);
    Ok(())
}

#[test]
fn large_absolute_seek_uses_absolute_fallback() -> io::Result<()> {
    let (source, _, seeks) = stream(Vec::new());
    let mut reader = PositionedReader::new(source);
    assert_eq!(reader.seek(SeekFrom::Start(u64::MAX))?, u64::MAX);
    assert_eq!(
        reader.seek(SeekFrom::Current(1)).unwrap_err().kind(),
        io::ErrorKind::InvalidInput,
    );
    assert_eq!(reader.stream_position()?, u64::MAX);
    assert_eq!(reader.seek(SeekFrom::Start(0))?, 0);
    assert_eq!(seeks.get(), 2);
    Ok(())
}
