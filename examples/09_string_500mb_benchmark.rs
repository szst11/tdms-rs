//! Creates a ~500 MB TDMS file with one group containing one string column and
//! three f64 data columns, then measures how long it takes to read the whole
//! file into memory with tdms-rs.

use std::env;
use std::fs;
use std::path::Path;
use std::time::Instant;
use tdms_rs::{TdmsFile, TdmsWriter};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    let keep = args.iter().any(|a| a == "--keep");
    let file_name = args
        .iter()
        .position(|a| a == "--file")
        .and_then(|i| args.get(i + 1))
        .cloned()
        .unwrap_or_else(|| "tdms_string_500mb.tdms".to_string());

    // ~500 MB of raw channel data:
    //   string column: 3_400_000 strings x (4 B offset + 120 B content)  = 421.6 MB
    //   3 f64 columns: 3_400_000 x 24 B                                  =  81.6 MB
    //   total                                                           ~ 503 MB
    const N: usize = 3_400_000;
    const STRING_BYTES: usize = 120;

    // ---- Write ----
    println!("Generating data...");
    let strings: Vec<String> = (0..N)
        .map(|i| String::from_utf8(vec![b'a' + (i % 26) as u8; STRING_BYTES]).unwrap())
        .collect();
    let data: Vec<f64> = (0..N as u64).map(|i| i as f64).collect();

    println!("Writing {} ->", file_name);
    let wstart = Instant::now();
    {
        let mut writer = TdmsWriter::create(&file_name)?;
        let mut group = writer.add_group("Data")?;
        let mut ch_str = group.add_channel::<String>("names")?;
        ch_str.write(&strings)?;
        let mut ch_a = group.add_channel::<f64>("data1")?;
        ch_a.write(&data)?;
        let mut ch_b = group.add_channel::<f64>("data2")?;
        ch_b.write(&data)?;
        let mut ch_c = group.add_channel::<f64>("data3")?;
        ch_c.write(&data)?;
    }
    drop(strings);
    drop(data);
    let wsecs = wstart.elapsed().as_secs_f64();
    let fsize = fs::metadata(&file_name)?.len() as f64;
    println!(
        "write: {:.2} s ({:.2} MB, {:.2} MB/s)",
        wsecs,
        fsize / 1_000_000.0,
        fsize / 1_000_000.0 / wsecs
    );

    // ---- Read ----
    let ostart = Instant::now();
    let file = TdmsFile::open(Path::new(&file_name))?;
    let osecs = ostart.elapsed().as_secs_f64();
    println!("open (metadata parse): {:.3} s", osecs);

    let group = file.group("Data").ok_or("group 'Data' not found")?;

    let rstart = Instant::now();

    let ch_str = group.channel("names").ok_or("channel 'names' not found")?;
    let mut names = vec![String::new(); N];
    ch_str.read_strings(0..N, &mut names)?;

    for name in ["data1", "data2", "data3"] {
        let ch = group.channel(name).ok_or("channel not found")?;
        let mut buf = vec![0.0f64; N];
        ch.read(0..N, &mut buf)?;
        std::hint::black_box(buf[0]);
        drop(buf);
    }

    let rsecs = rstart.elapsed().as_secs_f64();
    let total = osecs + rsecs;
    std::hint::black_box(names.len());

    println!(
        "full data read: {:.2} s ({:.2} MB/s)",
        rsecs,
        fsize / 1_000_000.0 / rsecs
    );
    println!(
        "total open+read: {:.2} s ({:.2} MB/s)",
        total,
        fsize / 1_000_000.0 / total
    );

    if !keep {
        fs::remove_file(&file_name)?;
    }
    Ok(())
}
