//! Reproduces and verifies reading a big-endian TDMS file made by nptdms.

use super::common::remove_tdms;
use std::process::Command;
use tdms_rs::TdmsFile;

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
