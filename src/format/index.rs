//! TDMS index-file helpers.
//!
//! A `.tdms_index` file is a companion acceleration index for a `.tdms` data
//! file. It stores exactly the same lead-ins and metadata as the data file but
//! with a `TDSh` tag instead of `TDSm` and without any raw channel data. Because
//! the data locations are derived purely from metadata, a reader can build the
//! full group/channel/property index by scanning only the (small) index file and
//! defer opening the data file until a channel is actually read.
//!
//! Layout follows the `nptdms` implementation:
//! * lead-in tag is `TDSh`;
//! * `next_segment_offset` equals `metadata_size + data_size` (as if the raw
//!   data were present);
//! * `raw_data_offset` equals `metadata_size`, so the following segment always
//!   begins `28 + raw_data_offset` bytes after the segment start.

use crate::error::Result;
use crate::format::metadata::RawDataMeta;
use crate::format::segment::Segment;
use crate::io::ext::TdmsWriteExt;
use crate::model::datatypes::{DataType, PropertyValue};
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

/// Tag used by a `.tdms` data file segment.
pub const DATA_TAG: [u8; 4] = *b"TDSm";
/// Tag used by a `.tdms_index` companion file segment.
pub const INDEX_TAG: [u8; 4] = *b"TDSh";
/// Bit in the ToC mask indicating the segment carries a new object list.
pub const TOC_NEW_OBJ_LIST: u32 = 1 << 2;
/// `raw_data_index` value meaning "no raw data in this segment".
pub const NO_RAW_DATA_INDEX: u32 = 0xFFFF_FFFF;

/// Bytes occupied by one object's raw data within a segment.
pub fn raw_data_size(meta: &RawDataMeta) -> u64 {
    if meta.data_type == DataType::String {
        meta.total_size_bytes
            .unwrap_or(meta.number_of_values.saturating_mul(4))
    } else {
        (meta.data_type.itemsize() as u64) * meta.number_of_values
    }
}

/// Write a 28-byte TDMS lead-in.
///
/// Both offsets are relative to the end of the lead-in, matching the format
/// specification and [`crate::format::index`] consumption.
pub fn write_lead_in<W: Write>(
    w: &mut W,
    tag: &[u8; 4],
    toc_mask: u32,
    version: u32,
    next_segment_offset: u64,
    raw_data_offset: u64,
) -> Result<()> {
    w.write_all(tag)?;
    w.write_u32(toc_mask)?;
    w.write_u32(version)?;
    w.write_u64(next_segment_offset)?;
    w.write_u64(raw_data_offset)?;
    Ok(())
}

/// Serialize a single object's metadata in the on-disk TDMS layout.
///
/// `raw` describes the raw-data index to emit: `(data_type, number_of_values,
/// total_size_bytes)`. `total_size_bytes` is only meaningful for string data.
/// `properties` are written in iterator order. Returns the number of bytes
/// written.
pub fn write_object_metadata<'a, W: Write, P>(
    w: &mut W,
    path: &str,
    raw: Option<(DataType, u64, Option<u64>)>,
    prop_count: usize,
    properties: P,
) -> Result<u64>
where
    P: Iterator<Item = (&'a String, &'a PropertyValue)>,
{
    write_object_metadata_with_raw_index(w, path, raw, NO_RAW_DATA_INDEX, prop_count, properties)
}

fn write_object_metadata_with_raw_index<'a, W: Write, P>(
    w: &mut W,
    path: &str,
    raw: Option<(DataType, u64, Option<u64>)>,
    empty_raw_index: u32,
    prop_count: usize,
    properties: P,
) -> Result<u64>
where
    P: Iterator<Item = (&'a String, &'a PropertyValue)>,
{
    let mut written = 0u64;

    w.write_u32(path.len() as u32)?;
    w.write_all(path.as_bytes())?;
    written += 4 + path.len() as u64;

    match raw {
        Some((dtype, count, total_size_bytes)) => {
            if dtype == DataType::String {
                w.write_u32(28)?;
                w.write_u32(dtype.to_u32())?;
                w.write_u32(1)?;
                w.write_u64(count)?;
                w.write_u64(total_size_bytes.unwrap_or(count.saturating_mul(4)))?;
                written += 4 + 4 + 4 + 8 + 8;
            } else {
                w.write_u32(20)?;
                w.write_u32(dtype.to_u32())?;
                w.write_u32(1)?;
                w.write_u64(count)?;
                written += 4 + 4 + 4 + 8;
            }
        }
        None => {
            w.write_u32(empty_raw_index)?;
            written += 4;
        }
    }

    w.write_u32(prop_count as u32)?;
    written += 4;
    for (key, value) in properties {
        w.write_u32(key.len() as u32)?;
        w.write_all(key.as_bytes())?;
        written += 4 + key.len() as u64;
        written += write_property_value(w, value)?;
    }
    Ok(written)
}

/// Serialize one property value and return the number of bytes written.
pub fn write_property_value<W: Write>(w: &mut W, value: &PropertyValue) -> Result<u64> {
    match value {
        PropertyValue::I8(v) => {
            w.write_u32(DataType::I8.to_u32())?;
            w.write_i8(*v)?;
            Ok(8)
        }
        PropertyValue::I16(v) => {
            w.write_u32(DataType::I16.to_u32())?;
            w.write_i16(*v)?;
            Ok(8)
        }
        PropertyValue::I32(v) => {
            w.write_u32(DataType::I32.to_u32())?;
            w.write_i32(*v)?;
            Ok(8)
        }
        PropertyValue::I64(v) => {
            w.write_u32(DataType::I64.to_u32())?;
            w.write_i64(*v)?;
            Ok(12)
        }
        PropertyValue::U8(v) => {
            w.write_u32(DataType::U8.to_u32())?;
            w.write_u8(*v)?;
            Ok(5)
        }
        PropertyValue::U16(v) => {
            w.write_u32(DataType::U16.to_u32())?;
            w.write_u16(*v)?;
            Ok(6)
        }
        PropertyValue::U32(v) => {
            w.write_u32(DataType::U32.to_u32())?;
            w.write_u32(*v)?;
            Ok(8)
        }
        PropertyValue::U64(v) => {
            w.write_u32(DataType::U64.to_u32())?;
            w.write_u64(*v)?;
            Ok(12)
        }
        PropertyValue::Float(v) => {
            w.write_u32(DataType::Float.to_u32())?;
            w.write_f32(*v)?;
            Ok(8)
        }
        PropertyValue::Double(v) => {
            w.write_u32(DataType::Double.to_u32())?;
            w.write_f64(*v)?;
            Ok(12)
        }
        PropertyValue::String(s) => {
            w.write_u32(DataType::String.to_u32())?;
            w.write_u32(s.len() as u32)?;
            w.write_all(s.as_bytes())?;
            Ok(8 + s.len() as u64)
        }
        PropertyValue::Boolean(b) => {
            w.write_u32(DataType::Boolean.to_u32())?;
            w.write_u8(if *b { 1 } else { 0 })?;
            Ok(5)
        }
        PropertyValue::TimeStamp((secs, frac)) => {
            w.write_u32(DataType::TimeStamp.to_u32())?;
            w.write_u64(*frac)?;
            w.write_i64(*secs)?;
            Ok(16)
        }
    }
}

/// Append the path of the companion index file for `path`.
///
/// Mirrors `nptdms`: an index for `data.tdms` is `data.tdms_index`.
pub fn index_path_for(path: &Path) -> std::path::PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push("_index");
    s.into()
}

/// Write a single segment (lead-in + metadata) to an index file.
///
/// The raw data of a segment is not written, but the lead-in reuses the
/// segment's *recorded* data-file offsets (`next_segment_offset` and
/// `raw_data_offset`), so a regenerated index always mirrors the data file's
/// layout even for files with continuation segments or metadata padding.
pub fn write_segment_index<W: Write>(w: &mut W, segment: &Segment) -> Result<()> {
    let mut metadata: Vec<u8> = Vec::new();

    if (segment._toc_mask & TOC_NEW_OBJ_LIST) != 0 {
        metadata.write_u32(segment.objects.len() as u32)?;
        for obj in &segment.objects {
            let raw = obj
                .raw_data_meta
                .as_ref()
                .map(|m| (m.data_type, m.number_of_values, m.total_size_bytes));
            write_object_metadata_with_raw_index(
                &mut metadata,
                &obj.path.raw,
                raw,
                obj.raw_data_index,
                obj.properties.len(),
                obj.properties.iter(),
            )?;
        }
    }

    write_lead_in(
        w,
        &INDEX_TAG,
        segment._toc_mask,
        segment._version,
        segment._next_segment_offset,
        segment._raw_data_offset,
    )?;
    w.write_all(&metadata)?;
    Ok(())
}

/// Create a `.tdms_index` file from already-parsed segments.
///
/// The generated index mirrors the metadata of the supplied segments and lets a
/// later [`crate::api::reader::TdmsFile::open`] reconstruct the same
/// group/channel index without re-reading the data file.
///
/// The index is written to a temporary file and atomically renamed into place,
/// so a concurrent reader either sees the previous index or the complete new
/// one, never a partially-written index.
pub fn write_index_file<P: AsRef<Path>>(index_path: P, segments: &[Segment]) -> Result<()> {
    let index_path = index_path.as_ref();
    let temp_path = index_path.with_extension("tdms_index.tmp");
    let file = File::create(&temp_path)?;
    let mut writer = BufWriter::new(file);
    for segment in segments {
        write_segment_index(&mut writer, segment)?;
    }
    writer.flush()?;
    if let Err(e) = std::fs::rename(&temp_path, index_path) {
        // Don't leave an orphaned temp file behind when the rename fails
        // (e.g. the index path is taken by a directory).
        let _ = std::fs::remove_file(&temp_path);
        return Err(e.into());
    }
    Ok(())
}
