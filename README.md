# tdms-rs

[![Crates.io](https://img.shields.io/crates/v/tdms-rs.svg)](https://crates.io/crates/tdms-rs)
[![Documentation](https://docs.rs/tdms-rs/badge.svg)](https://docs.rs/tdms-rs)
[![License](https://img.shields.io/crates/l/tdms-rs.svg)](#license)

A pure Rust library for reading and writing National Instruments TDMS (Technical Data Management Streaming) files with high performance and efficient reads into caller-provided buffers.

## 🚀 Key Features

- **⚡ High Performance**: Designed for high-throughput I/O. Achieves near-disk bandwidth by minimizing syscalls and using efficient buffer management.
- **📦 Lazy Loading**: Only loads data when requested. Metadata is indexed eagerly, while raw data is read on-demand to minimize memory footprint.
- **⚡ Fast Opens via `.tdms_index`**: Uses NI/`nptdms`-style companion index files (`<file>.tdms_index`) to build the metadata index quickly, opening the data file lazily only when channel data is requested. Missing or stale indexes are detected and regenerated on the fly (best-effort, so read-only directories keep working).
- **🛡️ Type Safe**: Strongly-typed channel access ensures data integrity at compile time.
- **🔗 Pure Rust**: No external C dependencies, making cross-compilation seamless.
- **📊 Full Format Support**: Supports all TDMS data types, hierarchical structures, and multi-segment files (Little-Endian).

## 📖 Documentation

- [**Quick Start Guide**](docs/API.md) - Get up and running in minutes.
- [**Architecture & Design**](docs/ARCHITECTURE.md) - Deep dive into internal reader/writer models and performance tradeoffs.
- [**API Reference** (docs.rs)](https://docs.rs/tdms-rs) - Detailed module and function-level documentation.

## 🛠️ Quick Start

### Reading

Companion index files (`<file>.tdms_index`) are used automatically to speed up
opening, and raw channel data is read lazily from the `.tdms` data file:

```rust
use tdms_rs::TdmsFile;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let file = TdmsFile::open("data.tdms")?;
    let group = file.group("Sensors").ok_or("Group not found")?;
    let channel = group.channel("Temperature").ok_or("Channel not found")?;

    let mut data = vec![0.0f64; channel.len()];
    channel.read(0..channel.len(), &mut data)?;

    println!("Read {} samples", data.len());
    Ok(())
}
```

By default, a missing, empty, or stale index is skipped in favor of the data
file and regenerated (best-effort). Tune this with `OpenOptions`:

```rust
use tdms_rs::OpenOptions;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Never use an index; always read metadata from the data file.
    let file = OpenOptions::new().use_index_file(false).open("data.tdms")?;

    // Do not generate an index when none exists.
    let file = OpenOptions::new().create_index_if_missing(false).open("data.tdms")?;

    // Verify the index matches the data file by parsing both.
    let file = OpenOptions::new().verify_index(true).open("data.tdms")?;
    Ok(())
}
```

### Writing

```rust
use tdms_rs::TdmsWriter;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    {
        let mut writer = TdmsWriter::create("output.tdms")?;
        let mut group = writer.add_group("DAQ")?;
        let mut channel = group.add_channel::<f64>("Voltage")?;
        
        channel.write(&[1.0, 2.0, 3.0])?;
        // Optional: explicitly flush to ensure data is written
        writer.flush()?;
        // File is automatically flushed and closed when writer goes out of scope
    }
    Ok(())
}
```

To also emit a `.tdms_index` companion file (for fast opens by this crate,
`nptdms`, or NI tools), use `TdmsWriterOptions`:

```rust
use tdms_rs::TdmsWriterOptions;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let writer = TdmsWriterOptions::new()
        .write_index_file(true)
        .create("output.tdms")?;
    // File (and its .tdms_index companion) is flushed and closed on drop.
    drop(writer);
    Ok(())
}
```

## 📐 Philosophy & Guarantees

1.  **Memory Efficiency**: Never load data you don't ask for. Metadata is indexed; raw data is lazy-loaded.
2.  **Fast, Safe Opens**: `.tdms_index` companion files make opening large files cheap without sacrificing correctness — stale/invalid indexes are regenerated and a read-only directory never breaks an otherwise-readable file.
3.  **Safety First**: Safe wrappers around `unsafe` memory operations.
4.  **Broad MSRV**: Supports Rust 1.70.0 and later.

## 🤝 License

Licensed under either of [Apache License 2.0](LICENSE-APACHE) or [MIT License](LICENSE-MIT) at your option.
