use super::common::remove_tdms;
use std::io::{Read, Seek, SeekFrom};
use tdms_rs::{DataLocation, OpenOptions, TdmsWriterOptions};

#[test]
fn indexed_locations_borrow_existing_metadata() -> Result<(), Box<dyn std::error::Error>> {
    let path = "tests/output/indexed_data_locations.tdms";
    std::fs::create_dir_all("tests/output")?;
    {
        let mut writer = TdmsWriterOptions::new()
            .write_index_file(true)
            .create(path)?;
        let mut group = writer.add_group("G")?;
        group.add_channel::<i32>("A")?.write(&[1, -2, 3])?;
        group.add_channel::<i32>("Empty")?.write(&[])?;
        writer.flush()?;
    }
    for use_index in [false, true] {
        let file = OpenOptions::new()
            .use_index_file(use_index)
            .create_index_if_missing(false)
            .open(path)?;
        let group = file.group("G").ok_or("missing group")?;
        let channel = group.channel("A").ok_or("missing channel")?;
        let locations: &[DataLocation] = channel.data_locations();
        assert_eq!(locations.len(), 1);
        assert_eq!(locations[0].number_of_values, 3);
        assert!(!locations[0].big_endian);
        assert_eq!(locations.as_ptr(), channel.data_locations().as_ptr());
        let mut data = std::fs::File::open(path)?;
        data.seek(SeekFrom::Start(locations[0].offset))?;
        let mut bytes = [0; 12];
        data.read_exact(&mut bytes)?;
        let values: Vec<_> = bytes
            .chunks_exact(4)
            .map(|b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .collect();
        assert_eq!(values, [1, -2, 3]);
        let empty = group.channel("Empty").ok_or("missing empty channel")?;
        assert!(empty
            .data_locations()
            .iter()
            .all(|loc| loc.number_of_values == 0));
    }
    let file = OpenOptions::new()
        .create_index_if_missing(false)
        .open(path)?;
    std::fs::remove_file(path)?;
    assert_eq!(
        file.group("G")
            .ok_or("missing group")?
            .channel("A")
            .ok_or("missing channel")?
            .data_locations()[0]
            .number_of_values,
        3,
    );
    remove_tdms(path);
    Ok(())
}
