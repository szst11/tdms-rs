//! Reproduces and verifies reading a big-endian TDMS file made by nptdms.

use super::common::remove_tdms;
use std::process::Command;
use tdms_rs::{PropertyValue, TdmsFile};

#[test]
fn nptdms_version_4713_big_endian_roundtrip() -> Result<(), Box<dyn std::error::Error>> {
    let path = "tests/output/nptdms_big_endian.tdms";
    let script = r#"
import numpy as np
from nptdms import ChannelObject, TdmsWriter
with TdmsWriter(r"tests/output/nptdms_big_endian.tdms", version=4713) as writer:
    writer.write_segment(
        ChannelObject("Measurements", "I32", np.array([1, 2, 3], dtype=np.int32)),
        ChannelObject("Measurements", "Double", np.array([1.5, 2.5, 3.5], dtype=np.float64)),
        ChannelObject("Measurements", "Text", np.array(["one", "two", "three"])),
    )
"#;
    let status = Command::new("uv")
        .args(["run", "--with", "nptdms", "python", "-c", script])
        .status()?;
    assert!(status.success(), "nptdms fixture generation failed");

    let file = TdmsFile::open(path)?;
    let group = file.group("Measurements").ok_or("missing group")?;

    let mut integers = [0i32; 3];
    group
        .channel("I32")
        .ok_or("missing integer channel")?
        .read(0..3, &mut integers)?;
    assert_eq!(integers, [1, 2, 3]);

    let mut doubles = [0.0f64; 3];
    group
        .channel("Double")
        .ok_or("missing double channel")?
        .read(0..3, &mut doubles)?;
    assert_eq!(doubles, [1.5, 2.5, 3.5]);

    let mut strings = vec![String::new(); 3];
    group
        .channel("Text")
        .ok_or("missing string channel")?
        .read_strings(0..3, &mut strings)?;
    assert_eq!(strings, ["one", "two", "three"]);

    remove_tdms(path);
    Ok(())
}

#[test]
fn timestamp_property_little_and_big_endian() -> Result<(), Box<dyn std::error::Error>> {
    const SECONDS: i64 = 3_877_675_200;
    const FRACTION: u64 = 3 << 62; // Exactly 0.75 seconds.
    let le_path = "tests/output/timestamp_little.tdms";
    let be_path = "tests/output/timestamp_big.tdms";
    remove_tdms(le_path);
    remove_tdms(be_path);

    let script = r#"
from io import BytesIO
import struct
from nptdms import RootObject, TdmsFile, TdmsWriter
from nptdms.timestamp import TdmsTimestamp

seconds = 3_877_675_200
fraction = 3 << 62

little = BytesIO()
with TdmsWriter(little) as writer:
    writer.write_segment([RootObject(properties={"timestamp": TdmsTimestamp(seconds, fraction)})])
little_bytes = little.getvalue()

name = b"timestamp"
big_metadata = (
    struct.pack(">I", 1)
    + struct.pack(">I", 1) + b"/"
    + struct.pack(">II", 0xFFFFFFFF, 1)
    + struct.pack(">I", len(name)) + name
    + struct.pack(">I", 0x44)
    + struct.pack(">qQ", seconds, fraction)
)
mask = struct.unpack_from("<I", little_bytes, 4)[0] | (1 << 6)
big_bytes = (
    b"TDSm"
    + struct.pack("<I", mask)
    + struct.pack(">IQQ", 4712, len(big_metadata), len(big_metadata))
    + big_metadata
)
for filename, payload in [("tests/output/timestamp_little.tdms", little_bytes),
                          ("tests/output/timestamp_big.tdms", big_bytes)]:
    parsed = TdmsFile.read(BytesIO(payload), raw_timestamps=True)
    actual = parsed.properties["timestamp"]
    assert (actual.seconds, actual.second_fractions) == (seconds, fraction)
    with open(filename, "xb") as destination:
        destination.write(payload)
"#;
    let status = Command::new("uv")
        .args(["run", "--with", "nptdms", "python", "-c", script])
        .status()?;
    assert!(status.success(), "timestamp fixture generation failed");

    for path in [le_path, be_path] {
        let file = TdmsFile::open(path)?;
        let value = file
            .properties()
            .find(|(name, _)| *name == "timestamp")
            .map(|(_, value)| value)
            .ok_or("missing timestamp property")?;
        match value {
            PropertyValue::TimeStamp((seconds, fraction)) => {
                assert_eq!(*seconds, SECONDS, "wrong seconds in {path}");
                assert_eq!(*fraction, FRACTION, "wrong fraction in {path}");
            }
            other => panic!("unexpected property type in {path}: {other:?}"),
        }
        remove_tdms(path);
    }
    Ok(())
}

#[test]
fn target_big_endian_file_matches_nptdms_baseline_when_configured(
) -> Result<(), Box<dyn std::error::Error>> {
    let Ok(path) = std::env::var("TDMS_RS_TARGET_TDMS") else {
        return Ok(());
    };

    let file = TdmsFile::open(path)?;
    assert_eq!(file.groups().count(), 5);
    let channel = file
        .group("MEASUREMENT")
        .ok_or("missing MEASUREMENT group")?
        .channel("Time")
        .ok_or("missing Time channel")?;
    assert_eq!(channel.len(), 1803);

    let mut values = [0.0f64; 3];
    channel.read(0..3, &mut values)?;
    assert_eq!(values, [12.0301934, 12.0301934, 12.0301934]);
    Ok(())
}
