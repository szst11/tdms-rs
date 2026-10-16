# tdms-rs Architecture

## Overview
`tdms-rs` is designed as a high-performance, type-safe Rust library for the National Instruments TDMS format. It prioritizes memory efficiency (especially for large datasets) and ergonomic API usage.

## Reader Model
The reader uses an indexed approach. When a file is opened:
1.  All segments are scanned and their metadata parsed into an in-memory index (`IndexMap`).
2.  Raw data locations (offsets and lengths) are stored without loading the actual data.

- **Metadata**: Loaded eagerly to provide fast navigation.
- **Raw Data**: Loaded lazily when `read()` is called, using file seeking and direct reads into user-provided buffers to minimize memory overhead.

Metadata parsing wraps its buffered stream in `PositionedReader`. Logical
offsets are updated from successful read counts rather than querying the file
position for every segment. Nearby absolute skips are translated to
`BufReader::seek_relative`, preserving unread buffer contents when possible;
skips beyond the buffer still seek the underlying stream. This applies to
both data-file metadata scans and companion-index scans, without changing
channel data locations or the on-disk format.

Data-file metadata reads start with 64 KiB read-ahead. Four consecutive
data-bearing segments whose skipped raw-data gaps are at most 64 KiB promote
refills to 1 MiB. A larger gap immediately reduces subsequent refills to
8 KiB and resets the dense-segment count; four nearby gaps are required to
regrow. Metadata-only segments and incomplete final segments do not train
the policy. Large gaps still use seeks, not sequential raw-data reads.
Companion indexes contain contiguous metadata and use 1 MiB refills directly.

The buffer allocation is bounded at 1 MiB per metadata scan. A small reader
adapter limits the underlying read request on each refill; changing that limit
never replaces the buffer or discards unread bytes. Previously fetched bytes
remain usable after a policy change. Read-ahead may fetch skipped raw bytes,
but does not decode them or retain whole channels. This layout-based policy
reduces small-I/O latency on fragmented files without using path/drive guesses,
timing-based tuning, or a public configuration option. It is not guaranteed to
beat the smaller buffer on cached, CPU-bound scans.

### Companion Index Files (`.tdms_index`)
When a sibling `<file>.tdms_index` exists (see `src/format/index.rs`), the reader
prefers it for building the metadata index. The index file stores the same
lead-ins and metadata as the data file but with a `TDSh` tag and no raw data, so
scanning it is much cheaper than scanning a large data file. The `.tdms` file is
only opened once a channel is actually read.

Key points:
- The index file layout mirrors `nptdms`: `next_segment_offset = metadata_size + data_size`,
  `raw_data_offset = metadata_size` (both as if the raw data were present).
- Because an index file omits the raw bytes of earlier segments, physical
  segment positions in the index differ from those in the data file after the
  first segment. `TdmsReaderInternal` therefore tracks the **data-file**
  segment start separately (`data_segment_start`) and resolves raw-data
  locations against the data-file layout, while seeking within the index by its
  own physical offsets.
- An existing index is only trusted when it is non-empty and not older than the
  data file (mtime check). A missing, empty, stale, or truncated/corrupt index
  is skipped in favor of parsing the data file, and (when enabled, the default)
  regenerated. Corruption is caught two ways: truncation *inside* a segment is
  an error during index parsing (data-file parsing instead tolerates an
  in-flight append), and truncation that lands exactly on a segment boundary is
  caught by comparing the end-of-data position implied by the parsed index
  against the data file's actual length (a stat, not a read).
- Regeneration writes to a temp file and atomically renames it into place, so a
  concurrent reader sees either the old index or a complete new one, never a
  partially-written index.
- Index regeneration batches serialized segments through a bounded 4 MiB
  write buffer instead of the standard 8 KiB default. This reduces small-write
  overhead on high-latency filesystems without changing the index format or
  retaining arbitrarily large serialized indexes in memory. Indexes smaller
  than the buffer are emitted in one final write. The final partial buffer
  is explicitly flushed before the temporary file is renamed.
- Index generation on open is best-effort: a write failure (e.g. read-only
  directory) never fails the open, so `open()` stays safe for read-only inputs.
- Index usage can be toggled (`OpenOptions::use_index_file`), generation can be
  disabled (`OpenOptions::create_index_if_missing`), and contents can be
  verified against the data file (`OpenOptions::verify_index`), which parses both
  and compares the resulting structure.

## Writer Model
The writer uses a staged approach to optimize for disk I/O throughput:
1.  **Metadata Staging**: All metadata (groups, channels, properties) is built in memory first.
2.  **Single Segment Emission**: `tdms-rs` currently writes data in a single large segment to maximize sequential write performance.
3.  **Buffering**: 
    - Metadata is written through an 8MB `BufWriter` to batch small syscalls.
    - Raw data is written directly to the file handle, bypassing the user-space buffer for large transfers.

When `TdmsWriterOptions::write_index_file` is enabled, the same metadata is also
serialized to a `<file>.tdms_index` companion file (with `TDSh` lead-in tags and
no raw data), so the file can be opened quickly by other tools.

### Write Performance Insights
From performance audits, the primary bottleneck for TDMS writes is often the OS-level sync or filesystem overhead rather than the library's serialization logic. `tdms-rs` achieves near-disk bandwidth by:
- Using `unsafe` pointer casting for zero-copy serialization of numeric slices.
- Minimizing syscalls (typically ~4 syscalls for a 1GB file write).

- `TdmsFile` owns the file path and the index.
- `TdmsChannel` and `TdmsGroup` are lightweight views into the index.
- Data is read directly into user-provided buffers via the `read()` method.

## Performance Tradeoffs
- **In-memory Index**: For files with millions of objects, memory usage for the index may be significant.
- **Single Segment Writing**: While fast, it requires knowing the data schema/size beforehand or buffering data in memory.
