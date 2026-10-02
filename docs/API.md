# tdms-rs API Overview

This document provides a high-level overview of the `tdms-rs` public API. For detailed function documentation, please refer to the [rustdoc](https://docs.rs/tdms-rs).

## Core Concepts

### Reading Files
The hierarchy follows the TDMS standard: `TdmsFile` → `TdmsGroup` → `TdmsChannel`.
Files are automatically closed when the `TdmsFile` is dropped.

```rust
let file = TdmsFile::open("data.tdms")?;
let group = file.group("Sensors").ok_or("Group not found")?;
let channel = group.channel("Temperature").ok_or("Channel not found")?;

// Read into a pre-allocated buffer
let mut data = vec![0.0f64; channel.len()];
channel.read(0..channel.len(), &mut data)?;
// File is automatically closed when 'file' goes out of scope
```

#### Companion Index Files (`.tdms_index`)
When a sibling `<file>.tdms_index` exists (as written by NI tools, `nptdms`,
or `TdmsWriterOptions::write_index_file`), `TdmsFile::open` uses it to build the
group/channel/property index quickly without scanning the data file. Raw channel
data is still read lazily from the `.tdms` file when requested.

An index is only trusted when it is non-empty and not older than the data file;
a missing, empty, stale, or truncated/corrupt index is skipped in favor of
parsing the data file, and the index is regenerated so future opens are fast.
Regeneration is best-effort, so opening a file (or a file in a read-only
directory) never fails because an index could not be written.

Tune this behavior with `OpenOptions`:

```rust,no_run
use tdms_rs::OpenOptions;

// Never use an index (always read metadata from the data file).
let file = OpenOptions::new().use_index_file(false).open("data.tdms")?;

// Do not generate an index when none exists.
let file = OpenOptions::new().create_index_if_missing(false).open("data.tdms")?;

// Verify the index matches the data file by parsing both.
let file = OpenOptions::new().verify_index(true).open("data.tdms")?;
```

### Writing Files
Writing uses a builder-like pattern through `TdmsWriter` with RAII-based lifecycle.

```rust
{
    let mut writer = TdmsWriter::create("output.tdms")?;
    let mut group = writer.add_group("Measurement")?;
    let mut channel = group.add_channel::<f64>("Voltage")?;

    channel.write(&[1.0, 2.0, 3.0])?;
    // Optional: explicitly flush to ensure data is written
    writer.flush()?; 
    // File is automatically flushed and closed when writer goes out of scope
}
```

To also write a `<file>.tdms_index` companion file, use `TdmsWriterOptions`:

```rust,no_run
use tdms_rs::TdmsWriterOptions;

{
    let writer = TdmsWriterOptions::new()
        .write_index_file(true)
        .create("output.tdms")?;
    // File (and its .tdms_index companion) is written when the writer drops
}
```

## Data Types
The following types are supported for properties and channel data:
- **Integers**: `i8`, `u8`, `i16`, `u16`, `i32`, `u32`, `i64`, `u64`
- **Floats**: `f32`, `f64`
- **Strings**: `String`
- **Boolean**: `bool`
- **Time**: `TimeStamp` (custom struct)

## Advanced Features

### Chunked Reading
To process large files without loading everything into memory, you can iterate over ranges:
```rust
let chunk_size = 1024;
let total_len = channel.len();
let mut buffer = vec![0.0f64; chunk_size];

for start in (0..total_len).step_by(chunk_size) {
    let end = (start + chunk_size).min(total_len);
    let count = end - start;
    channel.read(start..end, &mut buffer[0..count])?;
    // process buffer[0..count]...
}
```

### Direct Buffer Access
Minimize allocations by reading into a pre-allocated buffer:
```rust
let mut buffer = vec![0.0f64; 1000];
channel.read(0..1000, &mut buffer)?;
```
