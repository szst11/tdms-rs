use crate::error::{Result, TdmsError};
use crate::format::index::{index_path_for, raw_data_size, write_index_file};
use crate::format::metadata::ParsingMetadata;
use crate::format::segment::Segment;
use crate::io::ext::TdmsReadExt;
use crate::model::channel::{DataLocation, TdmsChannelData};
use crate::model::datatypes::{DataType, PropertyValue};
use crate::model::file::TdmsFileInner;
use crate::model::group::TdmsGroupData;
use indexmap::IndexMap;
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::ops::Range;
use std::path::Path;

struct StringReadTask<'a> {
    location: &'a DataLocation,
    read_start: usize,
    read_count: usize,
}

struct StringReadBlock {
    read_start: usize,
    read_count: usize,
    offsets: Vec<usize>,
    bytes: Vec<u8>,
}

const LEAD_IN_LEN: u64 = 28;
/// `next_segment_offset` value meaning the last segment is incomplete.
const INCOMPLETE_SEGMENT_OFFSET: u64 = 0xFFFF_FFFF_FFFF_FFFF;

/// A TDMS file handle for reading.
///
/// This struct indexes the file structure (groups, channels, properties) on open
/// without loading raw data into memory. When a sibling `<file>.tdms_index`
/// companion file exists it is used to build the metadata index (see
/// [`format::index`](crate::format::index)), so opening a large file only scans
/// the small index instead of the whole file. The `.tdms` data file is only
/// opened lazily when channel data is actually read.
pub struct TdmsFile {
    pub(crate) inner: TdmsFileInner,
}

/// A group within a TDMS file.
pub struct TdmsGroup<'a> {
    pub(crate) file: &'a TdmsFile,
    pub(crate) data: &'a TdmsGroupData,
}

/// A channel within a TDMS group.
pub struct TdmsChannel<'a> {
    pub(crate) file: &'a TdmsFile,
    pub(crate) data: &'a TdmsChannelData,
}

/// Options controlling how a TDMS file is opened for reading.
///
/// By default the reader looks for a sibling `<file>.tdms_index` companion
/// index and uses it to build the metadata index quickly. If the index is
/// missing, empty, or older than the data file it is skipped and (re)generated
/// from the data file on the fly. Index generation is best-effort, so opening a
/// file in a read-only directory still succeeds. Both behaviors can be disabled,
/// and index contents can be verified against the data file.
///
/// # Example
///
/// ```no_run
/// use tdms_rs::{TdmsFile, OpenOptions};
///
/// # fn main() -> Result<(), tdms_rs::TdmsError> {
/// let _file = OpenOptions::new()
///     .use_index_file(true)
///     .verify_index(true)
///     .open("data.tdms")?;
/// # Ok(())
/// # }
/// ```
pub struct OpenOptions {
    use_index_file: bool,
    create_index_if_missing: bool,
    verify_index: bool,
}

#[allow(clippy::derivable_impls)]
impl Default for OpenOptions {
    fn default() -> Self {
        Self {
            use_index_file: true,
            create_index_if_missing: true,
            verify_index: false,
        }
    }
}

impl OpenOptions {
    /// Create options with defaults:
    /// * index files are used when present,
    /// * a missing index is generated on the fly,
    /// * index contents are not verified.
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether a sibling `.tdms_index` file should be used to build the
    /// metadata index. Defaults to `true`.
    pub fn use_index_file(mut self, on: bool) -> Self {
        self.use_index_file = on;
        self
    }

    /// When no usable index file is present (missing, empty, or stale), write
    /// one next to the data file while opening so future opens are fast. The
    /// write is best-effort and never fails the open. Only takes effect when
    /// index files are enabled. Defaults to `true`.
    pub fn create_index_if_missing(mut self, on: bool) -> Self {
        self.create_index_if_missing = on;
        self
    }

    /// Verify that the `.tdms_index` file matches the `.tdms` data file by
    /// parsing both and comparing the resulting metadata. Defaults to `false`.
    ///
    /// This re-reads the data file, so it costs roughly the same as a no-index
    /// open. It is the reliable check against stale indexes: the automatic
    /// staleness detection compares modification times, which have limited
    /// granularity on some filesystems.
    pub fn verify_index(mut self, on: bool) -> Self {
        self.verify_index = on;
        self
    }

    /// Open a TDMS file for reading using these options.
    ///
    /// When an index is used it must be present, non-empty, and not older than
    /// the data file; a stale or invalid index is skipped in favor of parsing
    /// the data file (and regenerating the index, if enabled).
    pub fn open<P: AsRef<Path>>(&self, path: P) -> Result<TdmsFile> {
        let path = path.as_ref();
        let index_path = index_path_for(path);

        if self.is_index_usable(&index_path, path) {
            // A corrupt-but-non-empty index (e.g. truncated by a crash or a
            // concurrent open) is treated like a stale one: fall back to the
            // data file rather than failing an otherwise-readable open.
            if let Ok((segments, implied_end)) = Self::parse_segments_from_path(&index_path, true) {
                // A truncated index that happens to end on a segment boundary
                // parses without error. Such a cut makes the index describe
                // fewer segments than the data file actually holds, so compare
                // the implied end-of-data position against the real file length
                // (a stat, not a read). Verification is exempt: it inspects the
                // index by design and reports the mismatch itself.
                let covers_data = self.verify_index
                    || matches!(std::fs::metadata(path), Ok(m) if m.len() == implied_end);
                if covers_data {
                    let inner = Self::build_inner(path, &segments);

                    if self.verify_index {
                        let (data_segments, _) = Self::parse_segments_from_path(path, false)?;
                        let data_inner = Self::build_inner(path, &data_segments);
                        if inner != data_inner {
                            return Err(TdmsError::IndexMismatch(
                                "index file structure does not match the data file".to_string(),
                            ));
                        }
                    }

                    return Ok(TdmsFile { inner });
                }
            }
        }

        let (segments, _) = Self::parse_segments_from_path(path, false)?;
        let inner = Self::build_inner(path, &segments);

        if self.use_index_file && self.create_index_if_missing {
            // The index is an optimization only: a write failure here (e.g.
            // a read-only directory) must not fail an otherwise-readable
            // open, so generation is best-effort.
            let _ = write_index_file(&index_path, &segments);
        }

        Ok(TdmsFile { inner })
    }

    /// Decide whether a sibling index file can be trusted for this open.
    fn is_index_usable(&self, index_path: &Path, data_path: &Path) -> bool {
        if !self.use_index_file || !index_path.is_file() {
            return false;
        }

        // A zero-length index is invalid (e.g. a crash during creation) and
        // would silently hide a readable data file; treat it as missing.
        if index_path.metadata().map(|m| m.len() == 0).unwrap_or(false) {
            return false;
        }

        // Refuse to trust a stale index: if the data file was modified more
        // recently than the index, the index may describe outdated contents.
        // When the mtimes cannot be compared, err on the side of the data file.
        match (
            index_path.metadata().and_then(|m| m.modified()),
            data_path.metadata().and_then(|m| m.modified()),
        ) {
            (Ok(index_mtime), Ok(data_mtime)) => index_mtime >= data_mtime,
            _ => false,
        }
    }

    fn is_eof_error(e: &std::io::Error) -> bool {
        e.kind() == std::io::ErrorKind::UnexpectedEof
            || (e.kind() == std::io::ErrorKind::Other && e.to_string().contains("UnexpectedEof"))
    }

    fn parse_segments_from_path(path: &Path, is_index_file: bool) -> Result<(Vec<Segment>, u64)> {
        let file = File::open(path)?;
        let mut reader = TdmsReaderInternal::new(BufReader::new(file), is_index_file);
        let mut segments = Vec::new();

        loop {
            match reader.read_segment() {
                Ok(Some(segment)) => segments.push(segment),
                Ok(None) => break,
                // Tolerate a truncated tail in the data file (e.g. an in-flight
                // append) by returning the segments parsed so far. For an index
                // file any truncation is corruption: report it so the caller can
                // fall back to the data file instead of trusting a partial index.
                Err(TdmsError::Io(e)) if !is_index_file && Self::is_eof_error(&e) => break,
                Err(e) => return Err(e),
            }
        }

        // The reader tracks the data-file position where the next segment would
        // start; after the last segment this equals the end of the data file the
        // index describes. Comparing it against the actual file length catches an
        // index truncated exactly on a segment boundary, which parses as a clean
        // EOF and would otherwise be trusted silently.
        Ok((segments, reader.data_segment_start))
    }

    fn build_inner(path: &Path, segments: &[Segment]) -> TdmsFileInner {
        let mut groups = IndexMap::new();
        let mut file_properties = IndexMap::new();

        for segment in segments {
            for obj in &segment.objects {
                if let Some(g_name) = obj.path.group_name() {
                    let group = groups
                        .entry(g_name.to_string())
                        .or_insert_with(|| TdmsGroupData {
                            name: g_name.to_string(),
                            channels: IndexMap::new(),
                            properties: IndexMap::new(),
                        });

                    if let Some(c_name) = obj.path.channel_name() {
                        let channel =
                            group.channels.entry(c_name.to_string()).or_insert_with(|| {
                                TdmsChannelData {
                                    name: c_name.to_string(),
                                    dtype: DataType::Double,
                                    len: 0,
                                    data_locations: Vec::new(),
                                    properties: IndexMap::new(),
                                }
                            });

                        channel.properties.extend(obj.properties.clone());

                        if let Some(loc) = &obj.data_location {
                            channel.len += loc.number_of_values as usize;
                            channel.data_locations.push(DataLocation {
                                offset: loc.offset,
                                number_of_values: loc.number_of_values,
                            });
                        }

                        if let Some(meta) = &obj.raw_data_meta {
                            channel.dtype = meta.data_type;
                        }
                    } else {
                        group.properties.extend(obj.properties.clone());
                    }
                } else if obj.path.is_root() {
                    file_properties.extend(obj.properties.clone());
                }
            }
        }

        TdmsFileInner {
            path: path.to_path_buf(),
            groups,
            properties: file_properties,
        }
    }
}

impl TdmsFile {
    /// Open a TDMS file for reading.
    ///
    /// This parses and indexes all segment metadata eagerly, but does not read
    /// raw channel data until it is requested. When a sibling `.tdms_index`
    /// companion file exists it is used for the metadata index (a missing,
    /// empty, or stale index is skipped and the data file is parsed instead;
    /// see [`OpenOptions`]).
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        OpenOptions::new().open(path)
    }

    /// Look up a group by name.
    pub fn group(&self, name: &str) -> Option<TdmsGroup<'_>> {
        let data = self.inner.groups.get(name)?;
        Some(TdmsGroup { file: self, data })
    }

    /// Look up a file-level property by name.
    pub fn property(&self, name: &str) -> Option<&PropertyValue> {
        self.inner.properties.get(name)
    }

    /// Iterate over all file-level properties.
    pub fn properties(&self) -> impl Iterator<Item = (&str, &PropertyValue)> {
        self.inner.properties.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// Iterate over all groups in the file.
    pub fn groups(&self) -> impl Iterator<Item = TdmsGroup<'_>> {
        self.inner
            .groups
            .values()
            .map(move |data| TdmsGroup { file: self, data })
    }
}

impl<'a> TdmsGroup<'a> {
    /// Return the group name.
    pub fn name(&self) -> &str {
        &self.data.name
    }

    /// Look up a group-level property by name.
    pub fn property(&self, name: &str) -> Option<&PropertyValue> {
        self.data.properties.get(name)
    }

    /// Iterate over all group-level properties.
    pub fn properties(&self) -> impl Iterator<Item = (&str, &PropertyValue)> {
        self.data.properties.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// Look up a channel by name.
    pub fn channel(&self, name: &str) -> Option<TdmsChannel<'a>> {
        let data = self.data.channels.get(name)?;
        Some(TdmsChannel {
            file: self.file,
            data,
        })
    }

    /// Iterate over all channels in the group.
    pub fn channels(&self) -> impl Iterator<Item = TdmsChannel<'a>> {
        let file = self.file;
        self.data
            .channels
            .values()
            .map(move |data| TdmsChannel { file, data })
    }
}

impl<'a> TdmsChannel<'a> {
    /// Return the channel name.
    pub fn name(&self) -> &str {
        &self.data.name
    }

    /// Return the channel data type.
    pub fn dtype(&self) -> DataType {
        self.data.dtype
    }

    /// Return the number of values in the channel.
    pub fn len(&self) -> usize {
        self.data.len
    }

    /// Returns `true` if the channel contains no values.
    pub fn is_empty(&self) -> bool {
        self.data.len == 0
    }

    /// Look up a channel-level property by name.
    pub fn property(&self, name: &str) -> Option<&PropertyValue> {
        self.data.properties.get(name)
    }

    /// Iterate over all channel-level properties.
    pub fn properties(&self) -> impl Iterator<Item = (&str, &PropertyValue)> {
        self.data.properties.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// Read a range of values into the provided output buffer.
    ///
    /// The element type `T` must match the TDMS channel element size.
    pub fn read<T: Pod>(&self, range: Range<usize>, out: &mut [T]) -> Result<usize> {
        if range.end > self.data.len {
            return Err(TdmsError::InvalidRange(
                range.start,
                range.end,
                self.data.len,
            ));
        }
        if std::mem::size_of::<T>() != self.data.dtype.itemsize() {
            return Err(TdmsError::TypeMismatch);
        }
        let requested = range.end - range.start;
        if out.len() < requested {
            return Err(TdmsError::InvalidFormat(
                "output buffer too small for requested range".to_string(),
            ));
        }

        let out_bytes = unsafe {
            std::slice::from_raw_parts_mut(
                out.as_mut_ptr() as *mut u8,
                requested * std::mem::size_of::<T>(),
            )
        };

        // Open the data file lazily and seek to the requested offsets.
        // `TdmsFile` itself does not hold the file open; a handle is created
        // per read so opening (even via a `.tdms_index`) never touches the data.
        let file = File::open(&self.file.inner.path)?;
        let mut reader = BufReader::new(file);
        self.read_range_into_bytes(&range, out_bytes, &mut reader)?;
        Ok(requested)
    }

    /// Read a range of string values into the provided output buffer.
    ///
    /// The channel must have [`DataType::String`] dtype; otherwise a
    /// [`TdmsError::TypeMismatch`] is returned. Returns the number of strings read.
    pub fn read_strings(&self, range: Range<usize>, out: &mut [String]) -> Result<usize> {
        if range.end > self.data.len {
            return Err(TdmsError::InvalidRange(
                range.start,
                range.end,
                self.data.len,
            ));
        }
        if self.data.dtype != DataType::String {
            return Err(TdmsError::TypeMismatch);
        }
        let requested = range.end - range.start;
        if out.len() < requested {
            return Err(TdmsError::InvalidFormat(
                "output buffer too small for requested range".to_string(),
            ));
        }

        let blocks = self.read_string_blocks(&range)?;
        let mut out_cursor = 0;
        for block in blocks {
            let start_byte = block.offsets[block.read_start];
            for (i, slot) in out[out_cursor..out_cursor + block.read_count]
                .iter_mut()
                .enumerate()
            {
                let idx = block.read_start + i;
                let s = std::str::from_utf8(
                    &block.bytes
                        [block.offsets[idx] - start_byte..block.offsets[idx + 1] - start_byte],
                )
                .map_err(|_| TdmsError::StringEncoding)?;
                *slot = s.to_string();
            }
            out_cursor += block.read_count;
        }

        Ok(requested)
    }

    /// Read a range of string values as raw buffers in Arrow's string layout.
    ///
    /// The channel must have [`DataType::String`] dtype; otherwise a
    /// [`TdmsError::TypeMismatch`] is returned. Returns the number of strings read.
    ///
    /// `out_offsets` is cleared and filled with `range.len() + 1` cumulative
    /// 64-bit byte offsets (first element 0, even for an empty range);
    /// `out_bytes` is cleared and filled with the concatenated UTF-8 bytes of the
    /// requested strings. The 64-bit offsets cover total payloads beyond 4 GiB
    /// and are exactly what an Arrow `LargeUtf8` array needs, so downstream
    /// consumers can build one without per-element copies. The bytes are copied
    /// once from the file; string values spanning multiple segments are
    /// concatenated into a single contiguous buffer.
    pub fn read_string_buffers(
        &self,
        range: Range<usize>,
        out_offsets: &mut Vec<u64>,
        out_bytes: &mut Vec<u8>,
    ) -> Result<usize> {
        if range.end > self.data.len {
            return Err(TdmsError::InvalidRange(
                range.start,
                range.end,
                self.data.len,
            ));
        }
        if self.data.dtype != DataType::String {
            return Err(TdmsError::TypeMismatch);
        }
        let requested = range.end - range.start;

        out_offsets.clear();
        out_offsets.reserve(requested + 1);
        out_bytes.clear();

        if requested == 0 {
            out_offsets.push(0);
            return Ok(0);
        }

        let blocks = self.read_string_blocks(&range)?;

        for block in blocks {
            let base = block.offsets[block.read_start];
            let out_base = out_bytes.len();
            if out_offsets.is_empty() {
                out_offsets.push(out_base as u64);
            }
            for offset in &block.offsets[block.read_start + 1..=block.read_start + block.read_count]
            {
                out_offsets.push(out_base as u64 + *offset as u64 - base as u64);
            }
            out_bytes.extend_from_slice(&block.bytes);
        }

        Ok(requested)
    }

    fn read_string_blocks(&self, range: &Range<usize>) -> Result<Vec<StringReadBlock>> {
        let mut tasks = Vec::new();
        let mut remaining = range.end - range.start;
        let mut current_idx = range.start;

        for location in &self.data.data_locations {
            let location_count = location.number_of_values as usize;
            if current_idx >= location_count {
                current_idx -= location_count;
                continue;
            }

            let read_start = current_idx;
            let read_count = location_count.min(current_idx + remaining) - read_start;
            if read_count == 0 {
                break;
            }
            tasks.push(StringReadTask {
                location,
                read_start,
                read_count,
            });
            remaining -= read_count;
            current_idx = 0;
            if remaining == 0 {
                break;
            }
        }

        let file_len = std::fs::metadata(&self.file.inner.path)?.len();
        let file = File::open(&self.file.inner.path)?;
        let mut reader = BufReader::new(file);
        tasks
            .iter()
            .map(|task| {
                let (offsets, bytes) = self.read_string_block(
                    &mut reader,
                    task.location,
                    task.read_start,
                    task.read_count,
                    file_len,
                )?;
                Ok(StringReadBlock {
                    read_start: task.read_start,
                    read_count: task.read_count,
                    offsets,
                    bytes,
                })
            })
            .collect()
    }

    /// Read the string offset array for one data location and return only the
    /// bytes for `read_start..read_start + read_count` as a contiguous slice.
    ///
    /// The offset array (one `u32` per value) must be read in full, but the
    /// concatenated string bytes are sliced to the requested range instead of
    /// materializing the whole segment block. The file-supplied offsets are
    /// validated to be monotonic and to fit inside the file, so a corrupt input
    /// cannot panic or trigger a huge allocation.
    fn read_string_block(
        &self,
        reader: &mut BufReader<File>,
        loc: &DataLocation,
        read_start: usize,
        read_count: usize,
        file_len: u64,
    ) -> Result<(Vec<usize>, Vec<u8>)> {
        let loc_count = loc.number_of_values as usize;

        reader.seek(SeekFrom::Start(loc.offset))?;
        // A valid string block stores at least one 4-byte offset per value, so a
        // declared count larger than what the remaining file can hold means the
        // header is corrupt. Without this guard a crafted count would drive a
        // multi-GiB allocation below.
        if loc_count as u64 > file_len.saturating_sub(loc.offset) / 4 {
            return Err(TdmsError::InvalidFormat(
                "string channel value count exceeds file size".to_string(),
            ));
        }
        let offsets_byte_len = loc_count
            .checked_mul(std::mem::size_of::<u32>())
            .ok_or_else(|| {
                TdmsError::InvalidFormat("string channel offset table is too large".to_string())
            })?;
        let mut raw_offsets = vec![0; offsets_byte_len];
        reader.read_exact(&mut raw_offsets)?;

        let mut offsets = Vec::with_capacity(loc_count + 1);
        offsets.push(0);
        let mut prev = 0usize;
        for bytes in raw_offsets.chunks_exact(std::mem::size_of::<u32>()) {
            let offset = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
            if offset < prev {
                return Err(TdmsError::InvalidFormat(
                    "string channel offsets must be monotonic non-decreasing".to_string(),
                ));
            }
            prev = offset;
            offsets.push(offset);
        }

        let block_len = prev;
        let bytes_start = loc.offset + loc_count as u64 * 4;
        if block_len as u64 > file_len.saturating_sub(bytes_start) {
            return Err(TdmsError::InvalidFormat(
                "string channel data exceeds remaining file size".to_string(),
            ));
        }

        let start_byte = offsets[read_start];
        let end_byte = offsets[read_start + read_count];
        let mut bytes = vec![0u8; end_byte - start_byte];
        if start_byte != 0 {
            reader.seek(SeekFrom::Start(bytes_start + start_byte as u64))?;
        }
        reader.read_exact(&mut bytes)?;

        Ok((offsets, bytes))
    }

    fn read_range_into_bytes<R: std::io::Read + std::io::Seek>(
        &self,
        range: &Range<usize>,
        out: &mut [u8],
        reader: &mut R,
    ) -> Result<()> {
        let itemsize = self.data.dtype.itemsize();
        let total_bytes = (range.end - range.start) * itemsize;
        if out.len() != total_bytes {
            return Err(TdmsError::InvalidFormat(
                "output buffer length must exactly match requested byte length".to_string(),
            ));
        }

        let mut remaining = range.end - range.start;
        let mut current_offset = range.start;
        let mut out_cursor = 0;

        for loc in &self.data.data_locations {
            let loc_end = loc.number_of_values as usize;

            if current_offset >= loc_end {
                current_offset -= loc_end;
                continue;
            }

            let read_start = current_offset;
            let read_end = loc_end.min(current_offset + remaining);
            let read_count = read_end - read_start;

            if read_count == 0 {
                break;
            }

            let byte_offset = loc.offset + (read_start * itemsize) as u64;
            reader.seek(SeekFrom::Start(byte_offset))?;

            let read_bytes = read_count * itemsize;
            reader.read_exact(&mut out[out_cursor..out_cursor + read_bytes])?;
            out_cursor += read_bytes;

            remaining -= read_count;
            current_offset = 0;

            if remaining == 0 {
                break;
            }
        }

        Ok(())
    }
}

/// Marker trait for plain-old-data types supported by [`TdmsChannel::read`].
pub trait Pod: Copy {}
impl Pod for i8 {}
impl Pod for u8 {}
impl Pod for i16 {}
impl Pod for u16 {}
impl Pod for i32 {}
impl Pod for u32 {}
impl Pod for i64 {}
impl Pod for u64 {}
impl Pod for f32 {}
impl Pod for f64 {}
impl Pod for bool {}

struct TdmsReaderInternal<R: Read + Seek> {
    reader: R,
    is_index_file: bool,
    /// Start of the current segment as it appears in the `.tdms` data file.
    ///
    /// For the data file this always equals the physical stream position. For
    /// an index file the physical position skips the raw data of earlier
    /// segments, so the data-file equivalent must be tracked separately: raw
    /// data locations are resolved against the data file, not the index.
    data_segment_start: u64,
    active_meta: std::collections::HashMap<String, crate::format::metadata::RawDataMeta>,
    object_order: Vec<String>,
}

impl<R: Read + Seek> TdmsReaderInternal<R> {
    fn new(reader: R, is_index_file: bool) -> Self {
        Self {
            reader,
            is_index_file,
            data_segment_start: 0,
            active_meta: std::collections::HashMap::new(),
            object_order: Vec::new(),
        }
    }

    /// Read one segment. Returns `Ok(None)` when the reader is positioned
    /// exactly at a clean end-of-file; returns `Ok(Some(..))` for a parsed
    /// segment. Any truncation *inside* a segment is an error — for index
    /// files that means the caller can fall back to the data file instead of
    /// trusting a partially-written index.
    fn read_segment(&mut self) -> Result<Option<Segment>> {
        let start_pos = self.reader.stream_position()?;

        // Read the whole 28-byte lead-in rather than byte-by-byte so a clean
        // boundary can be told apart from a truncated header.
        let mut lead_in = [0u8; 28];
        let mut filled = 0;
        while filled < 28 {
            let n = match self.reader.read(&mut lead_in[filled..]) {
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                other => other?,
            };
            match n {
                0 if filled == 0 => return Ok(None),
                0 => break,
                n => filled += n,
            }
        }
        if filled < 28 {
            if self.is_index_file {
                return Err(TdmsError::InvalidFormat(
                    "truncated segment lead-in: index file ends mid-header".to_string(),
                ));
            }
            // Tolerate a trailing partial segment in a data file so files that
            // are still being appended can be opened for their completed parts.
            return Ok(None);
        }

        let expected = if self.is_index_file { b"TDSh" } else { b"TDSm" };
        if lead_in[..4] != *expected {
            return Err(TdmsError::InvalidSignature);
        }

        let mut lead = &lead_in[4..];
        let mask_val = lead.read_u32()?;
        let version = lead.read_u32()?;
        let next_segment_offset = lead.read_u64()?;
        let raw_data_offset = lead.read_u64()?;

        let mask = crate::format::segment::Mask::new(mask_val);
        let mut objects = Vec::new();

        if mask.has_new_obj_list() {
            let count = self.reader.read_u32()?;
            self.object_order.clear();

            for _ in 0..count {
                let path_len = self.reader.read_u32()?;
                let mut path_bytes = vec![0u8; path_len as usize];
                self.reader.read_exact(&mut path_bytes)?;
                let path_str =
                    String::from_utf8(path_bytes).map_err(|_| TdmsError::StringEncoding)?;
                self.object_order.push(path_str.clone());

                let raw_data_index = self.reader.read_u32()?;
                let mut raw_data_meta = None;
                let prop_count;

                if raw_data_index != 0 && raw_data_index != 0xFFFFFFFF {
                    let type_code = self.reader.read_u32()?;
                    let data_type = DataType::from_u32(type_code)?;

                    let mut count = 0;
                    let mut total_size = None;

                    if data_type == DataType::String {
                        // String index info is 24 bytes: type (4), dimension (4),
                        // number of values (8), total size in bytes of all string
                        // data including the per-value offset array (8). Some writers
                        // under-report the index length in the header, so read the
                        // fixed-size index from the stream instead of using it.
                        self.reader.read_u32()?; // dimension (must be 1)
                        count = self.reader.read_u64()?;
                        total_size = Some(self.reader.read_u64()?);
                        prop_count = self.reader.read_u32()?;
                        raw_data_meta = Some(crate::format::metadata::RawDataMeta {
                            data_type,
                            number_of_values: count,
                            total_size_bytes: total_size,
                        });
                    } else if raw_data_index >= 4 {
                        // Numeric index info: the declared index length includes the
                        // trailing 4-byte property count, so the number of properties
                        // is stored at the end of the index region.
                        let mut skipped = vec![0u8; (raw_data_index - 4) as usize];
                        self.reader.read_exact(&mut skipped)?;

                        if skipped.len() >= 12 {
                            let mut count_slice = &skipped[4..12];
                            count = count_slice.read_u64()?;
                        }
                        let start = skipped.len().saturating_sub(4);
                        let mut end_slice = &skipped[start..];
                        prop_count = end_slice.read_u32()?;

                        raw_data_meta = Some(crate::format::metadata::RawDataMeta {
                            data_type,
                            number_of_values: count,
                            total_size_bytes: total_size,
                        });
                    } else {
                        return Err(TdmsError::InvalidFormat(format!(
                            "invalid raw data index length {} for non-string data type",
                            raw_data_index
                        )));
                    }
                } else {
                    prop_count = self.reader.read_u32()?;
                }

                let mut properties = std::collections::HashMap::new();
                for _ in 0..prop_count {
                    let key_len = self.reader.read_u32()?;
                    let mut key_bytes = vec![0u8; key_len as usize];
                    self.reader.read_exact(&mut key_bytes)?;
                    let key =
                        String::from_utf8(key_bytes).map_err(|_| TdmsError::StringEncoding)?;
                    let type_code = self.reader.read_u32()?;
                    let val =
                        crate::model::datatypes::DataType::from_u32(type_code).and_then(|dt| {
                            match dt {
                                DataType::I8 => Ok(PropertyValue::I8(self.reader.read_i8()?)),
                                DataType::I16 => Ok(PropertyValue::I16(self.reader.read_i16()?)),
                                DataType::I32 => Ok(PropertyValue::I32(self.reader.read_i32()?)),
                                DataType::I64 => Ok(PropertyValue::I64(self.reader.read_i64()?)),
                                DataType::U8 => Ok(PropertyValue::U8(self.reader.read_u8()?)),
                                DataType::U16 => Ok(PropertyValue::U16(self.reader.read_u16()?)),
                                DataType::U32 => Ok(PropertyValue::U32(self.reader.read_u32()?)),
                                DataType::U64 => Ok(PropertyValue::U64(self.reader.read_u64()?)),
                                DataType::Float => {
                                    Ok(PropertyValue::Float(self.reader.read_f32()?))
                                }
                                DataType::Double => {
                                    Ok(PropertyValue::Double(self.reader.read_f64()?))
                                }
                                DataType::Boolean => {
                                    Ok(PropertyValue::Boolean(self.reader.read_u8()? != 0))
                                }
                                DataType::String => {
                                    let len = self.reader.read_u32()?;
                                    let mut buf = vec![0u8; len as usize];
                                    self.reader.read_exact(&mut buf)?;
                                    let s = String::from_utf8(buf)
                                        .map_err(|_| TdmsError::StringEncoding)?;
                                    Ok(PropertyValue::String(s))
                                }
                                DataType::TimeStamp => {
                                    let fraction = self.reader.read_u64()?;
                                    let seconds = self.reader.read_i64()?;
                                    Ok(PropertyValue::TimeStamp((seconds, fraction)))
                                }
                            }
                        })?;
                    properties.insert(key, val);
                }

                objects.push(ParsingMetadata {
                    path: crate::format::metadata::ObjectPath::new(path_str),
                    raw_data_index,
                    properties,
                    raw_data_meta,
                    data_location: None,
                });
            }
        } else {
            for path_str in &self.object_order {
                objects.push(ParsingMetadata {
                    path: crate::format::metadata::ObjectPath::new(path_str.clone()),
                    raw_data_index: 0,
                    properties: std::collections::HashMap::new(),
                    raw_data_meta: None,
                    data_location: None,
                });
            }
        }

        let data_start = self.data_segment_start;
        let mut current_raw_offset = data_start + LEAD_IN_LEN + raw_data_offset;

        for obj in &mut objects {
            let path_str = obj.path.raw.clone();
            if let Some(meta) = &obj.raw_data_meta {
                self.active_meta.insert(path_str.clone(), meta.clone());
                if meta.number_of_values > 0 {
                    let size = raw_data_size(meta);
                    obj.data_location = Some(crate::format::metadata::DataLocation {
                        offset: current_raw_offset,
                        number_of_values: meta.number_of_values,
                        _data_type: meta.data_type,
                        _total_size_bytes: meta.total_size_bytes,
                    });
                    current_raw_offset += size;
                }
            } else if obj.raw_data_index == 0 {
                if let Some(meta) = self.active_meta.get(&path_str) {
                    if meta.number_of_values > 0 {
                        let size = raw_data_size(meta);
                        obj.data_location = Some(crate::format::metadata::DataLocation {
                            offset: current_raw_offset,
                            number_of_values: meta.number_of_values,
                            _data_type: meta.data_type,
                            _total_size_bytes: meta.total_size_bytes,
                        });
                        current_raw_offset += size;
                    }
                }
            }
        }

        // For an index file there is no raw data, so the following segment
        // starts directly where this segment's metadata ends. This is more
        // robust than assuming `raw_data_offset` equals the metadata size
        // (which is only true for files without metadata padding).
        let metadata_end = self.reader.stream_position()?;
        let target_pos = if self.is_index_file {
            metadata_end
        } else if next_segment_offset != INCOMPLETE_SEGMENT_OFFSET {
            start_pos + LEAD_IN_LEN + next_segment_offset
        } else {
            current_raw_offset
        };

        // Advance the data-file segment start for the next segment. The data
        // file reserves space for the raw data written here, so consecutive
        // segments are `28 + next_segment_offset` bytes apart (or `current_raw_offset`
        // when the offset is incomplete).
        self.data_segment_start = if next_segment_offset != INCOMPLETE_SEGMENT_OFFSET {
            data_start + LEAD_IN_LEN + next_segment_offset
        } else {
            current_raw_offset
        };

        let current_pos = self.reader.stream_position()?;
        if current_pos != target_pos {
            self.reader.seek(SeekFrom::Start(target_pos))?;
        }

        Ok(Some(Segment {
            _version: version,
            _next_segment_offset: next_segment_offset,
            _raw_data_offset: raw_data_offset,
            _toc_mask: mask.convert(),
            objects,
        }))
    }
}
