use std::fs::{self, File};
use std::io::Write;
use tdms_rs::OpenOptions;

fn segment(file: &mut File, metadata: &[u8], raw: &[u8], padding: u64) -> std::io::Result<()> {
    file.write_all(b"TDSm")?;
    file.write_all(&(if raw.is_empty() { 6u32 } else { 14u32 }).to_le_bytes())?;
    file.write_all(&4713u32.to_le_bytes())?;
    file.write_all(&(metadata.len() as u64 + padding + raw.len() as u64).to_le_bytes())?;
    file.write_all(&(metadata.len() as u64 + padding).to_le_bytes())?;
    file.write_all(metadata)?;
    file.write_all(&vec![0; padding as usize])?;
    file.write_all(raw)
}

#[test]
fn mixed_dense_sparse_segments_preserve_channel_values_and_generated_index(
) -> Result<(), Box<dyn std::error::Error>> {
    fs::create_dir_all("tests/output")?;
    let path = "tests/output/adaptive_mixed_segments.tdms";
    let mut file = File::create(path)?;
    let mut root = Vec::new();
    root.extend_from_slice(&1u32.to_le_bytes());
    root.extend_from_slice(&1u32.to_le_bytes());
    root.extend_from_slice(b"/");
    root.extend_from_slice(&u32::MAX.to_le_bytes());
    root.extend_from_slice(&0u32.to_le_bytes());
    for _ in 0..16 {
        segment(&mut file, &root, &[], 0)?;
    }
    segment(&mut file, &root, &[], 128 * 1024)?;

    let mut expected_len = 0usize;
    for count in [
        1024usize, 1024, 1024, 1024, 1024, 300_000, 1024, 1024, 1024, 1024, 1024, 400_000, 1024,
    ] {
        let mut metadata = Vec::new();
        metadata.extend_from_slice(&1u32.to_le_bytes());
        metadata.extend_from_slice(&8u32.to_le_bytes());
        metadata.extend_from_slice(b"/'G'/'C'");
        metadata.extend_from_slice(&20u32.to_le_bytes());
        metadata.extend_from_slice(&3u32.to_le_bytes());
        metadata.extend_from_slice(&1u32.to_le_bytes());
        metadata.extend_from_slice(&(count as u64).to_le_bytes());
        metadata.extend_from_slice(&0u32.to_le_bytes());
        let raw: Vec<_> = (expected_len..expected_len + count)
            .flat_map(|value| (value as i32).to_le_bytes())
            .collect();
        segment(&mut file, &metadata, &raw, 0)?;
        expected_len += count;
    }
    file.flush()?;
    drop(file);

    for indexed in [false, true, true] {
        let file = OpenOptions::new()
            .use_index_file(indexed)
            .verify_index(indexed)
            .open(path)?;
        let channel = file.group("G").unwrap().channel("C").unwrap();
        assert_eq!(channel.len(), expected_len);
        let mut values = vec![0i32; expected_len];
        channel.read(0..expected_len, &mut values)?;
        assert!(values
            .iter()
            .enumerate()
            .all(|(i, value)| *value == i as i32));
    }
    fs::remove_file(path)?;
    fs::remove_file(format!("{path}_index"))?;
    Ok(())
}
