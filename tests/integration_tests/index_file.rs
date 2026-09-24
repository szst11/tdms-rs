//! Tests for `.tdms_index` companion files: creating, reading via index,
//! on-the-fly generation, staleness handling, and verification.

use tdms_rs::{OpenOptions, TdmsFile, TdmsWriter, TdmsWriterOptions};

use super::common::remove_tdms;

fn create_dir() -> std::io::Result<()> {
    std::fs::create_dir_all("tests/output")
}

#[test]
fn writer_options_create_index_file() -> Result<(), Box<dyn std::error::Error>> {
    create_dir()?;
    let path = "tests/output/index_writer_options.tdms";

    {
        let writer = TdmsWriterOptions::new()
            .write_index_file(true)
            .create(path)?;
        // Auto-flush on drop.
        drop(writer);
    }

    assert!(
        std::path::Path::new(path).exists(),
        "data file should exist"
    );
    assert!(
        std::path::Path::new(&format!("{}_index", path)).exists(),
        "index file should be created"
    );

    remove_tdms(path);
    Ok(())
}

#[test]
fn default_writer_creates_no_index_file() -> Result<(), Box<dyn std::error::Error>> {
    create_dir()?;
    let path = "tests/output/index_default_writer.tdms";

    {
        let mut writer = TdmsWriter::create(path)?;
        let mut group = writer.add_group("G")?;
        let mut channel = group.add_channel::<f64>("C")?;
        channel.write(&[1.0, 2.0])?;
    }

    assert!(std::path::Path::new(path).exists());
    assert!(
        !std::path::Path::new(&format!("{}_index", path)).exists(),
        "default writer must not create an index file"
    );

    remove_tdms(path);
    Ok(())
}

#[test]
fn read_via_index_file_matches_data_file() -> Result<(), Box<dyn std::error::Error>> {
    create_dir()?;
    let path = "tests/output/index_read_via_index.tdms";

    {
        let mut writer = TdmsWriterOptions::new()
            .write_index_file(true)
            .create(path)?;
        let mut group = writer.add_group("Test")?;
        let mut channel = group.add_channel::<f64>("Data")?;
        channel.write(&[1.0, 2.0, 3.0])?;
    }

    // Open with index enabled: metadata comes from the .tdms_index file.
    let file = TdmsFile::open(path)?;
    let channel = file.group("Test").unwrap().channel("Data").unwrap();
    assert_eq!(channel.len(), 3);
    assert_eq!(channel.dtype(), tdms_rs::DataType::Double);

    // Data still comes from the .tdms data file.
    let mut data = vec![0.0f64; 3];
    channel.read(0..3, &mut data)?;
    assert_eq!(data, &[1.0, 2.0, 3.0]);

    remove_tdms(path);
    Ok(())
}

#[test]
fn open_without_index_recreates_index() -> Result<(), Box<dyn std::error::Error>> {
    create_dir()?;
    let path = "tests/output/index_recreate.tdms";

    {
        let mut writer = TdmsWriter::create(path)?;
        let mut group = writer.add_group("G")?;
        let mut channel = group.add_channel::<i32>("C")?;
        channel.write(&[7, 8, 9])?;
    }

    let index_path = format!("{}_index", path);
    assert!(
        !std::path::Path::new(&index_path).exists(),
        "no index should exist yet"
    );

    // Default open generates the missing index on the fly.
    let file = TdmsFile::open(path)?;
    assert!(
        std::path::Path::new(&index_path).exists(),
        "missing index should have been generated"
    );
    let channel = file.group("G").unwrap().channel("C").unwrap();
    let mut data = vec![0i32; 3];
    channel.read(0..3, &mut data)?;
    assert_eq!(data, &[7, 8, 9]);

    remove_tdms(path);
    Ok(())
}

#[test]
fn open_without_recreation_does_not_write_index() -> Result<(), Box<dyn std::error::Error>> {
    create_dir()?;
    let path = "tests/output/index_no_recreate.tdms";

    {
        let mut writer = TdmsWriter::create(path)?;
        let mut group = writer.add_group("G")?;
        let mut channel = group.add_channel::<f64>("C")?;
        channel.write(&[1.0])?;
    }

    let index_path = format!("{}_index", path);
    let file = OpenOptions::new()
        .create_index_if_missing(false)
        .open(path)?;
    assert!(
        !std::path::Path::new(&index_path).exists(),
        "index generation must be deactivatable"
    );
    let channel = file.group("G").unwrap().channel("C").unwrap();
    assert_eq!(channel.len(), 1);

    remove_tdms(path);
    Ok(())
}

#[test]
fn use_index_file_disabled_reads_data_file_only() -> Result<(), Box<dyn std::error::Error>> {
    create_dir()?;
    let path = "tests/output/index_disabled.tdms";

    {
        let mut writer = TdmsWriterOptions::new()
            .write_index_file(true)
            .create(path)?;
        let mut group = writer.add_group("Test")?;
        let mut channel = group.add_channel::<f64>("Data")?;
        channel.write(&[1.0, 2.0, 3.0])?;
    }

    // Even though an index exists, disabling index usage reads the data file.
    let file = OpenOptions::new().use_index_file(false).open(path)?;
    let channel = file.group("Test").unwrap().channel("Data").unwrap();
    let mut data = vec![0.0f64; 3];
    channel.read(0..3, &mut data)?;
    assert_eq!(data, &[1.0, 2.0, 3.0]);

    remove_tdms(path);
    Ok(())
}

#[test]
fn verify_index_detects_mismatch() -> Result<(), Box<dyn std::error::Error>> {
    create_dir()?;
    let data_path = "tests/output/index_verify_data.tdms";
    let other_path = "tests/output/index_verify_other.tdms";

    // Data file with channel counts [1, 2, 3].
    {
        let mut writer = TdmsWriter::create(data_path)?;
        let mut group = writer.add_group("G")?;
        let mut channel = group.add_channel::<f64>("C")?;
        channel.write(&[1.0, 2.0, 3.0])?;
    }

    // A second file (with index) that describes different channel contents.
    {
        let mut writer = TdmsWriterOptions::new()
            .write_index_file(true)
            .create(other_path)?;
        let mut group = writer.add_group("G")?;
        let mut channel = group.add_channel::<f64>("C")?;
        channel.write(&[9.0, 9.0, 9.0, 9.0, 9.0])?;
    }
    // Overwrite the data file's index with a non-matching one.
    std::fs::copy(
        format!("{}_index", other_path),
        format!("{}_index", data_path),
    )?;

    let result = OpenOptions::new()
        .use_index_file(true)
        .verify_index(true)
        .open(data_path);
    match result {
        Ok(_) => panic!("verification should fail when index does not match the data file"),
        Err(tdms_rs::TdmsError::IndexMismatch(_)) => {}
        Err(other) => panic!("expected IndexMismatch, got {:?}", other),
    }

    remove_tdms(data_path);
    remove_tdms(other_path);
    Ok(())
}

#[test]
fn verify_index_passes_for_matching_pair() -> Result<(), Box<dyn std::error::Error>> {
    create_dir()?;
    let path = "tests/output/index_verify_ok.tdms";

    {
        let mut writer = TdmsWriterOptions::new()
            .write_index_file(true)
            .create(path)?;
        let mut group = writer.add_group("Test")?;
        let mut channel = group.add_channel::<f64>("Data")?;
        channel.write(&[1.0, 2.0, 3.0])?;
    }

    let file = OpenOptions::new().verify_index(true).open(path)?;
    let channel = file.group("Test").unwrap().channel("Data").unwrap();
    assert_eq!(channel.len(), 3);

    remove_tdms(path);
    Ok(())
}

#[test]
fn string_channel_roundtrip_via_index() -> Result<(), Box<dyn std::error::Error>> {
    create_dir()?;
    let path = "tests/output/index_strings.tdms";

    let data = vec![
        "Hello".to_string(),
        "World".to_string(),
        "".to_string(),
        "unicode: café".to_string(),
    ];
    {
        let mut writer = TdmsWriterOptions::new()
            .write_index_file(true)
            .create(path)?;
        let mut group = writer.add_group("Test")?;
        let mut channel = group.add_channel::<String>("Data")?;
        channel.write(&data)?;
    }

    let file = TdmsFile::open(path)?;
    let channel = file.group("Test").unwrap().channel("Data").unwrap();
    let mut read_back = vec![String::new(); data.len()];
    channel.read_strings(0..data.len(), &mut read_back)?;
    assert_eq!(read_back, data);

    remove_tdms(path);
    Ok(())
}

fn force_old_index_mtime(path: &str) -> std::io::Result<()> {
    // Make the index definitively older than the data file. On unix a `touch`
    // with a fixed past date is deterministic; elsewhere fall back to a real
    // sleep, which separates the mtimes on coarse-granularity filesystems.
    #[cfg(unix)]
    {
        let _ = std::process::Command::new("touch")
            .args(["-d", "2000-01-01", &format!("{}_index", path)])
            .status();
    }
    #[cfg(not(unix))]
    std::thread::sleep(std::time::Duration::from_millis(1100));
    Ok(())
}

#[test]
fn stale_index_is_detected_and_regenerated() -> Result<(), Box<dyn std::error::Error>> {
    create_dir()?;
    let path = "tests/output/index_stale.tdms";

    // Write v1 and let the first open create the index describing 3 values.
    {
        let mut writer = TdmsWriter::create(path)?;
        let mut group = writer.add_group("G")?;
        let mut channel = group.add_channel::<f64>("C")?;
        channel.write(&[1.0, 2.0, 3.0])?;
    }
    {
        let file = TdmsFile::open(path)?;
        assert_eq!(file.group("G").unwrap().channel("C").unwrap().len(), 3);
    }
    force_old_index_mtime(path)?;

    // Rewrite the data file with different contents; the index is now stale.
    {
        let mut writer = TdmsWriter::create(path)?;
        let mut group = writer.add_group("G")?;
        let mut channel = group.add_channel::<f64>("C")?;
        channel.write(&[9.0, 9.0, 9.0, 9.0])?;
    }

    // A stale index must be skipped and regenerated from the data file.
    let file = TdmsFile::open(path)?;
    let channel = file.group("G").unwrap().channel("C").unwrap();
    assert_eq!(channel.len(), 4);
    let mut data = vec![0.0f64; 4];
    channel.read(0..4, &mut data)?;
    assert_eq!(data, &[9.0, 9.0, 9.0, 9.0]);

    remove_tdms(path);
    Ok(())
}

#[test]
fn empty_index_file_falls_back_to_data() -> Result<(), Box<dyn std::error::Error>> {
    create_dir()?;
    let path = "tests/output/index_empty.tdms";
    let index_path = format!("{}_index", path);

    {
        let mut writer = TdmsWriter::create(path)?;
        let mut group = writer.add_group("G")?;
        let mut channel = group.add_channel::<i32>("C")?;
        channel.write(&[1, 2, 3])?;
    }

    // A crash during creation could leave a zero-byte index; it must not hide
    // the readable data file.
    std::fs::write(&index_path, b"")?;
    let file = TdmsFile::open(path)?;
    let channel = file.group("G").unwrap().channel("C").unwrap();
    assert_eq!(channel.len(), 3);
    let mut data = vec![0i32; 3];
    channel.read(0..3, &mut data)?;
    assert_eq!(data, &[1, 2, 3]);
    // And the empty index is regenerated in place.
    assert!(std::fs::metadata(&index_path)
        .map(|m| m.len() > 0)
        .unwrap_or(false));

    remove_tdms(path);
    Ok(())
}

#[test]
fn corrupt_index_file_falls_back_to_data() -> Result<(), Box<dyn std::error::Error>> {
    create_dir()?;
    let path = "tests/output/index_corrupt.tdms";
    let index_path = format!("{}_index", path);

    {
        let mut writer = TdmsWriter::create(path)?;
        let mut group = writer.add_group("G")?;
        let mut channel = group.add_channel::<i32>("C")?;
        channel.write(&[1, 2, 3])?;
    }

    // A truncated (but non-empty) index can be left behind by a crash or a
    // concurrent open; it must not hide the readable data file.
    std::fs::write(&index_path, b"\x54\x44\x53\x68\x00")?;
    let file = TdmsFile::open(path)?;
    let channel = file.group("G").unwrap().channel("C").unwrap();
    assert_eq!(channel.len(), 3);
    let mut data = vec![0i32; 3];
    channel.read(0..3, &mut data)?;
    assert_eq!(data, &[1, 2, 3]);
    // And the corrupt index is regenerated in place.
    assert!(std::fs::metadata(&index_path)
        .map(|m| m.len() > 0)
        .unwrap_or(false));

    remove_tdms(path);
    Ok(())
}

#[test]
fn index_truncated_at_segment_boundary_falls_back() -> Result<(), Box<dyn std::error::Error>> {
    create_dir()?;
    let fixture = "tests/fixtures/tdms_corpus/02_structure_variants/multiple_segments.tdms";
    let path = "tests/output/index_boundary_cut.tdms";
    let index_path = format!("{}_index", path);
    std::fs::copy(fixture, path)?;
    std::fs::copy(format!("{}_index", fixture), &index_path)?;

    // Simulate a copy/crash that cut the index exactly on a segment boundary.
    // In the lead-in, raw_data_offset lives at bytes 20..28; segment 2 starts
    // at 28 + raw_data_offset (these files have no metadata padding). Such a
    // truncation parses as a clean EOF without error, so it exercises the
    // end-of-file non-truncated detection.
    let bytes = std::fs::read(&index_path)?;
    let raw_offset = u64::from_le_bytes(bytes[20..28].try_into().unwrap());
    let boundary = 28usize + raw_offset as usize;
    std::fs::write(&index_path, &bytes[..boundary])?;

    let file = TdmsFile::open(path)?;
    let channel = file.group("Group").unwrap().channel("Channel1").unwrap();
    assert_eq!(channel.len(), 6);
    let mut data = vec![0i8; 6];
    channel.read(0..6, &mut data)?;
    assert_eq!(data, &[1, 2, 3, 4, 5, 6]);
    // The truncated index has been regenerated to cover the whole data file.
    assert!(std::fs::metadata(&index_path)
        .map(|m| m.len() as usize > boundary)
        .unwrap_or(false));

    remove_tdms(path);
    Ok(())
}

#[test]
fn index_generation_failure_does_not_fail_open() -> Result<(), Box<dyn std::error::Error>> {
    create_dir()?;
    let path = "tests/output/index_readonly.tdms";
    let index_path = format!("{}_index", path);

    {
        let mut writer = TdmsWriter::create(path)?;
        let mut group = writer.add_group("G")?;
        let mut channel = group.add_channel::<f64>("C")?;
        channel.write(&[1.0, 2.0, 3.0])?;
    }

    // Block index generation deterministically by placing a directory where the
    // index file would be written.
    std::fs::create_dir(&index_path)?;
    let file = TdmsFile::open(path)?;
    let channel = file.group("G").unwrap().channel("C").unwrap();
    assert_eq!(channel.len(), 3);
    let mut data = vec![0.0f64; 3];
    channel.read(0..3, &mut data)?;
    assert_eq!(data, &[1.0, 2.0, 3.0]);

    std::fs::remove_dir(&index_path)?;
    remove_tdms(path);
    Ok(())
}
