use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, Seek, Write};
use std::ops::Range;
use std::path::Path;
use tdms_rs::{DataLocation, DataType, OpenOptions, PropertyValue, TdmsChannel, TdmsFile};

const A: &str = "/'DAQ/main'/'Value'";
const B: &str = "/'Other'/'Value'";
const FLOAT: &str = "/'DAQ/main'/'Float'";
const FLAG: &str = "/'DAQ/main'/'Flag'";
const TEXT: &str = "/'DAQ/main'/'Text/label'";
const EMPTY: &str = "/'DAQ/main'/'Empty'";
const NEVER: &str = "/'DAQ/main'/'Never'";
const LATE: &str = "/'Late'/'Value'";
const VERY_LATE: &str = "/'Late'/'VeryLate'";

#[derive(Clone)]
enum Values {
    I16(Vec<i16>),
    I32(Vec<i32>),
    I64(Vec<i64>),
    Double(Vec<f64>),
    Boolean(Vec<bool>),
    String(Vec<String>),
}

impl Values {
    fn layout(&self) -> (DataType, usize) {
        match self {
            Self::I16(v) => (DataType::I16, v.len()),
            Self::I32(v) => (DataType::I32, v.len()),
            Self::I64(v) => (DataType::I64, v.len()),
            Self::Double(v) => (DataType::Double, v.len()),
            Self::Boolean(v) => (DataType::Boolean, v.len()),
            Self::String(v) => (DataType::String, v.len()),
        }
    }

    fn encode(&self) -> Vec<u8> {
        match self {
            Self::I16(v) => v.iter().flat_map(|v| v.to_le_bytes()).collect(),
            Self::I32(v) => v.iter().flat_map(|v| v.to_le_bytes()).collect(),
            Self::I64(v) => v.iter().flat_map(|v| v.to_le_bytes()).collect(),
            Self::Double(v) => v.iter().flat_map(|v| v.to_le_bytes()).collect(),
            Self::Boolean(v) => v.iter().map(|v| u8::from(*v)).collect(),
            Self::String(v) => {
                let mut bytes = Vec::new();
                let mut end = 0u32;
                for value in v {
                    end += value.len() as u32;
                    bytes.extend_from_slice(&end.to_le_bytes());
                }
                for value in v {
                    bytes.extend_from_slice(value.as_bytes());
                }
                bytes
            }
        }
    }

    fn append(&mut self, other: &Self) {
        match (self, other) {
            (Self::I16(a), Self::I16(b)) => a.extend_from_slice(b),
            (Self::I32(a), Self::I32(b)) => a.extend_from_slice(b),
            (Self::I64(a), Self::I64(b)) => a.extend_from_slice(b),
            (Self::Double(a), Self::Double(b)) => a.extend_from_slice(b),
            (Self::Boolean(a), Self::Boolean(b)) => a.extend_from_slice(b),
            (Self::String(a), Self::String(b)) => a.extend_from_slice(b),
            _ => panic!("fixture channel changed its data type"),
        }
    }

    fn check(&self, channel: &TdmsChannel<'_>, range: Range<usize>) -> tdms_rs::Result<()> {
        macro_rules! numeric {
            ($values:expr, $type:ty) => {{
                let mut actual = vec![<$type>::default(); range.len()];
                channel.read(range.clone(), &mut actual)?;
                assert_eq!(actual, $values[range]);
            }};
        }
        match self {
            Self::I16(v) => numeric!(v, i16),
            Self::I32(v) => numeric!(v, i32),
            Self::I64(v) => numeric!(v, i64),
            Self::Double(v) => numeric!(v, f64),
            Self::Boolean(v) => numeric!(v, bool),
            Self::String(v) => {
                let mut offsets = Vec::new();
                let mut bytes = Vec::new();
                let count = channel.read_string_buffers(range.clone(), &mut offsets, &mut bytes)?;
                assert_eq!(count, range.len());
                assert_eq!(offsets.len(), count + 1);
                assert_eq!(offsets[0], 0);
                assert_eq!(offsets[count] as usize, bytes.len());
                let actual: Vec<_> = offsets
                    .windows(2)
                    .map(|pair| {
                        std::str::from_utf8(&bytes[pair[0] as usize..pair[1] as usize]).unwrap()
                    })
                    .collect();
                assert_eq!(actual, v[range]);
            }
        }
        Ok(())
    }
}

#[derive(Clone)]
struct Object {
    path: &'static str,
    values: Option<Values>,
    reuse: bool,
    properties: Vec<(&'static str, PropertyValue)>,
}

impl Object {
    fn data(path: &'static str, values: Values) -> Self {
        Self {
            path,
            values: Some(values),
            reuse: false,
            properties: Vec::new(),
        }
    }

    fn metadata(path: &'static str) -> Self {
        Self {
            path,
            values: None,
            reuse: false,
            properties: Vec::new(),
        }
    }

    fn reuse(mut self) -> Self {
        self.reuse = true;
        self
    }

    fn property(mut self, name: &'static str, value: PropertyValue) -> Self {
        self.properties.push((name, value));
        self
    }
}

fn string(bytes: &mut Vec<u8>, value: &str) {
    bytes.extend_from_slice(&(value.len() as u32).to_le_bytes());
    bytes.extend_from_slice(value.as_bytes());
}

fn property(bytes: &mut Vec<u8>, value: &PropertyValue) {
    match value {
        PropertyValue::I32(value) => {
            bytes.extend_from_slice(&3u32.to_le_bytes());
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        PropertyValue::Double(value) => {
            bytes.extend_from_slice(&10u32.to_le_bytes());
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        PropertyValue::String(value) => {
            bytes.extend_from_slice(&32u32.to_le_bytes());
            string(bytes, value);
        }
        PropertyValue::TimeStamp((seconds, fraction)) => {
            bytes.extend_from_slice(&68u32.to_le_bytes());
            bytes.extend_from_slice(&fraction.to_le_bytes());
            bytes.extend_from_slice(&seconds.to_le_bytes());
        }
        _ => panic!("unsupported property type in fixture encoder"),
    }
}

struct Fixture {
    file: File,
    expected: HashMap<&'static str, Values>,
    locations: HashMap<&'static str, Vec<DataLocation>>,
    order: Vec<&'static str>,
    layouts: HashMap<&'static str, (DataType, usize, usize)>,
}

impl Fixture {
    fn new(path: &Path) -> io::Result<Self> {
        Ok(Self {
            file: File::create(path)?,
            expected: HashMap::new(),
            locations: HashMap::new(),
            order: Vec::new(),
            layouts: HashMap::new(),
        })
    }

    fn segment(&mut self, objects: Vec<Object>, metadata: bool, padding: usize) -> io::Result<()> {
        let order: Vec<_> = objects.iter().map(|object| object.path).collect();
        let mut meta = Vec::new();
        if metadata {
            self.order = order;
            meta.extend_from_slice(&(objects.len() as u32).to_le_bytes());
        } else {
            assert_eq!(order, self.order);
        }
        let mut raw = Vec::new();
        let mut fragments = Vec::new();
        for object in objects {
            let bytes = object.values.as_ref().map(Values::encode);
            if metadata {
                string(&mut meta, object.path);
            }
            if let Some(values) = &object.values {
                let (dtype, count) = values.layout();
                let bytes = bytes.as_ref().unwrap();
                let layout = (dtype, count, bytes.len());
                if object.reuse || !metadata {
                    assert_eq!(self.layouts.get(object.path), Some(&layout));
                    if metadata {
                        meta.extend_from_slice(&0u32.to_le_bytes());
                    }
                } else {
                    self.layouts.insert(object.path, layout);
                    meta.extend_from_slice(
                        &(if dtype == DataType::String {
                            28u32
                        } else {
                            20u32
                        })
                        .to_le_bytes(),
                    );
                    meta.extend_from_slice(&dtype.to_u32().to_le_bytes());
                    meta.extend_from_slice(&1u32.to_le_bytes());
                    meta.extend_from_slice(&(count as u64).to_le_bytes());
                    if dtype == DataType::String {
                        meta.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
                    }
                }
                self.expected
                    .entry(object.path)
                    .and_modify(|expected| expected.append(values))
                    .or_insert_with(|| values.clone());
                if count > 0 {
                    fragments.push((object.path, raw.len(), count));
                }
                raw.extend_from_slice(bytes);
            } else if metadata {
                meta.extend_from_slice(&u32::MAX.to_le_bytes());
            }
            if metadata {
                meta.extend_from_slice(&(object.properties.len() as u32).to_le_bytes());
                for (name, value) in &object.properties {
                    string(&mut meta, name);
                    property(&mut meta, value);
                }
            } else {
                assert!(object.properties.is_empty());
            }
        }
        let raw_offset = meta.len() + padding;
        let raw_start = self.file.stream_position()? + 28 + raw_offset as u64;
        for (path, offset, count) in fragments {
            self.locations.entry(path).or_default().push(DataLocation {
                offset: raw_start + offset as u64,
                number_of_values: count as u64,
                big_endian: false,
            });
        }
        let mask = if metadata { 6u32 } else { 0 } | if raw.is_empty() { 0 } else { 8 };
        self.file.write_all(b"TDSm")?;
        self.file.write_all(&mask.to_le_bytes())?;
        self.file.write_all(&4713u32.to_le_bytes())?;
        self.file
            .write_all(&((raw_offset + raw.len()) as u64).to_le_bytes())?;
        self.file.write_all(&(raw_offset as u64).to_le_bytes())?;
        self.file.write_all(&meta)?;
        self.file.write_all(&vec![0; padding])?;
        self.file.write_all(&raw)
    }

    fn check(&self, file: &TdmsFile) -> tdms_rs::Result<()> {
        assert_eq!(
            file.groups()
                .map(|group| group.name().to_owned())
                .collect::<Vec<_>>(),
            ["DAQ/main", "Other", "Late"]
        );
        let group = file.group("DAQ/main").unwrap();
        assert_eq!(
            group
                .channels()
                .map(|channel| channel.name().to_owned())
                .collect::<Vec<_>>(),
            ["Value", "Float", "Flag", "Text/label", "Empty", "Never"]
        );
        assert_eq!(
            file.property("phase"),
            Some(&PropertyValue::from("finished"))
        );
        assert_eq!(file.property("cycle"), Some(&PropertyValue::I32(31)));
        assert_eq!(
            file.property("created"),
            Some(&PropertyValue::TimeStamp((1234, 1 << 63)))
        );
        assert_eq!(
            group.property("kind"),
            Some(&PropertyValue::from("primary"))
        );
        assert_eq!(group.property("gain"), Some(&PropertyValue::Double(30.25)));
        let channel = group.channel("Value").unwrap();
        assert_eq!(channel.property("serial"), Some(&PropertyValue::I32(7)));
        assert_eq!(
            channel.property("unit"),
            Some(&PropertyValue::from("unit-30"))
        );
        assert_eq!(
            file.group("Other").unwrap().property("kind"),
            Some(&PropertyValue::from("secondary"))
        );
        assert_eq!(
            file.group("Late").unwrap().property("kind"),
            Some(&PropertyValue::from("late"))
        );
        for (path, group, name) in [
            (A, "DAQ/main", "Value"),
            (B, "Other", "Value"),
            (FLOAT, "DAQ/main", "Float"),
            (FLAG, "DAQ/main", "Flag"),
            (TEXT, "DAQ/main", "Text/label"),
            (EMPTY, "DAQ/main", "Empty"),
            (NEVER, "DAQ/main", "Never"),
            (LATE, "Late", "Value"),
            (VERY_LATE, "Late", "VeryLate"),
        ] {
            let expected = &self.expected[path];
            let (dtype, len) = expected.layout();
            let channel = file.group(group).unwrap().channel(name).unwrap();
            assert_eq!(channel.dtype(), dtype, "{path}");
            assert_eq!(channel.len(), len, "{path}");
            let locations = self.locations.get(path).map(Vec::as_slice).unwrap_or(&[]);
            assert_eq!(channel.data_locations(), locations, "{path}");
            expected.check(&channel, 0..len)?;
            expected.check(&channel, 0..0)?;
            let mut boundary = 0;
            for fragment in locations {
                boundary += fragment.number_of_values as usize;
                expected.check(
                    &channel,
                    boundary.saturating_sub(1)..(boundary + 2).min(len),
                )?;
            }
        }
        Ok(())
    }
}

#[test]
fn mixed_objects_metadata_and_raw_segments_remain_readable(
) -> Result<(), Box<dyn std::error::Error>> {
    fs::create_dir_all(Path::new("tests").join("output"))?;
    let path = Path::new("tests")
        .join("output")
        .join("object_id_mixed.tdms");
    let index = path.with_extension("tdms_index");
    let mut fixture = Fixture::new(&path)?;
    fixture.segment(
        vec![
            Object::metadata("/").property("created", PropertyValue::TimeStamp((1234, 1 << 63))),
            Object::data(A, Values::I32(vec![1, 2, 3]))
                .property("serial", 7.into())
                .property("unit", "raw".into()),
            Object::metadata("/'Other'").property("kind", "secondary".into()),
            Object::data(B, Values::I16(vec![-7])),
            Object::data(FLOAT, Values::Double(vec![0.25, 1.5])),
            Object::metadata("/'DAQ/main'").property("kind", "primary".into()),
            Object::data(FLAG, Values::Boolean(vec![false, true])),
            Object::data(TEXT, Values::String(vec!["start".into(), "".into()])),
            Object::data(EMPTY, Values::I32(vec![])),
            Object::data(NEVER, Values::I32(vec![])),
        ],
        true,
        13,
    )?;
    fixture.segment(
        vec![
            Object::data(
                TEXT,
                Values::String(vec!["x".into(), "yy".into(), "zzz".into()]),
            ),
            Object::data(B, Values::I16(vec![-6, -5])),
            Object::data(A, Values::I32(vec![4, 5, 6])).reuse(),
            Object::data(FLOAT, Values::Double(vec![2.5, 3.5])).reuse(),
            Object::data(FLAG, Values::Boolean(vec![true, false])).reuse(),
        ],
        true,
        0,
    )?;
    fixture.segment(
        vec![
            Object::data(
                TEXT,
                Values::String(vec!["q".into(), "rr".into(), "sss".into()]),
            ),
            Object::data(B, Values::I16(vec![-4, -3])),
            Object::data(A, Values::I32(vec![7, 8, 9])),
            Object::data(FLOAT, Values::Double(vec![4.5, 5.5])),
            Object::data(FLAG, Values::Boolean(vec![false, false])),
        ],
        false,
        0,
    )?;
    fixture.segment(
        vec![
            Object::metadata("/'Late'").property("kind", "late".into()),
            Object::metadata(A).property("unit", "metadata-only".into()),
        ],
        true,
        7,
    )?;
    fixture.segment(
        vec![
            Object::data(LATE, Values::I64(vec![1 << 40, -(1 << 41)])),
            Object::data(A, Values::I32(vec![10, 11, 12, 13])),
            Object::data(TEXT, Values::String(vec!["late".into()])),
            Object::data(B, Values::I16(vec![-2, -1])).reuse(),
        ],
        true,
        0,
    )?;
    for cycle in 0..32i32 {
        let count = if cycle == 7 { 30_000 } else { cycle % 5 + 1 };
        let text = if cycle == 11 {
            "pad".repeat(30_000)
        } else {
            format!("value-{cycle}")
        };
        let mut objects = vec![
            Object::data(
                A,
                Values::I32((0..count).map(|i| cycle * 100 + i).collect()),
            ),
            Object::data(TEXT, Values::String(vec!["".into(), text])),
            Object::data(B, Values::I16(vec![cycle as i16, -(cycle as i16)])).reuse(),
            Object::data(
                FLOAT,
                Values::Double(vec![cycle as f64 + 0.125, -(cycle as f64)]),
            )
            .reuse(),
            Object::data(FLAG, Values::Boolean(vec![cycle % 2 == 0, cycle % 3 == 0])).reuse(),
            Object::metadata("/").property("cycle", cycle.into()),
        ];
        if cycle % 2 != 0 {
            objects.reverse();
        }
        fixture.segment(objects.clone(), true, if cycle % 7 == 0 { 17 } else { 0 })?;
        if cycle % 4 == 0 {
            for object in &mut objects {
                object.properties.clear();
            }
            fixture.segment(objects, false, 0)?;
        }
        if cycle % 3 == 0 {
            fixture.segment(
                vec![
                    Object::metadata(A).property("unit", format!("unit-{cycle}").into()),
                    Object::metadata("/'DAQ/main'").property("gain", (cycle as f64 + 0.25).into()),
                ],
                true,
                0,
            )?;
        }
    }
    fixture.segment(
        vec![
            Object::data(VERY_LATE, Values::I64(vec![i64::MAX - 2, i64::MIN + 5])),
            Object::data(EMPTY, Values::I32(vec![777, -777])),
            Object::metadata("/").property("phase", "finished".into()),
        ],
        true,
        0,
    )?;
    fixture.file.flush()?;
    for (use_index, create, verify) in [
        (false, false, false),
        (true, true, false),
        (true, false, true),
    ] {
        let file = OpenOptions::new()
            .use_index_file(use_index)
            .create_index_if_missing(create)
            .verify_index(verify)
            .open(&path)?;
        fixture.check(&file)?;
    }
    assert!(index.is_file());

    // A bad source lead-in makes an unnoticed index fallback fail, while the
    // raw payload and its offsets remain readable through the valid index.
    fs::OpenOptions::new()
        .write(true)
        .open(&path)?
        .write_all(b"BAD!")?;
    fs::OpenOptions::new()
        .write(true)
        .open(&index)?
        .write_all(b"TDSh")?;
    assert!(OpenOptions::new()
        .use_index_file(false)
        .open(&path)
        .is_err());
    fixture.check(
        &OpenOptions::new()
            .create_index_if_missing(false)
            .open(&path)?,
    )?;
    drop(fixture);
    fs::remove_file(path)?;
    fs::remove_file(index)?;
    Ok(())
}
