use tdms_rs::TdmsFile;

fn decode(offsets: &[u64], bytes: &[u8], n: usize) -> Vec<String> {
    (0..n)
        .map(|i| {
            std::str::from_utf8(&bytes[offsets[i] as usize..offsets[i + 1] as usize])
                .unwrap()
                .to_string()
        })
        .collect()
}

#[test]
fn string_buffers_roundtrip() {
    let tdms = TdmsFile::open("tests/fixtures/tdms_corpus/03_datatypes/strings.tdms").unwrap();
    let ch = tdms.group("Strings").unwrap().channel("Basic").unwrap();
    assert_eq!(ch.dtype(), tdms_rs::DataType::String);
    assert_eq!(ch.len(), 5);

    let mut offsets = Vec::new();
    let mut bytes = Vec::new();
    let n = ch
        .read_string_buffers(0..5, &mut offsets, &mut bytes)
        .unwrap();
    assert_eq!(n, 5);
    assert_eq!(offsets.len(), 6);
    assert_eq!(offsets[0], 0);
    assert_eq!(offsets[5] as usize, bytes.len());
    assert_eq!(
        decode(&offsets, &bytes, 5),
        vec!["Hello", "World", "", "TDMS", "File Format"]
    );

    let mut sub_offsets = Vec::new();
    let mut sub_bytes = Vec::new();
    let sub = ch
        .read_string_buffers(1..4, &mut sub_offsets, &mut sub_bytes)
        .unwrap();
    assert_eq!(sub, 3);
    assert_eq!(sub_offsets, vec![0, 5, 5, 9]);
    assert_eq!(&sub_bytes, b"WorldTDMS");

    let mut empty_offsets = Vec::new();
    let mut empty_bytes = Vec::new();
    let empty = ch
        .read_string_buffers(3..3, &mut empty_offsets, &mut empty_bytes)
        .unwrap();
    assert_eq!(empty, 0);
    assert_eq!(empty_offsets, vec![0]);
    assert!(empty_bytes.is_empty());
}

#[test]
fn string_buffers_multisegment() {
    let tdms =
        TdmsFile::open("tests/fixtures/tdms_corpus/05_string_edge_cases/edge_cases.tdms").unwrap();
    let ch = tdms.group("Strings").unwrap().channel("Detailed").unwrap();

    let mut offsets = Vec::new();
    let mut bytes = Vec::new();
    let n = ch
        .read_string_buffers(0..ch.len(), &mut offsets, &mut bytes)
        .unwrap();
    assert_eq!(n, ch.len());
    assert_eq!(offsets.len(), n + 1);
    assert_eq!(offsets[n] as usize, bytes.len());
    let decoded = decode(&offsets, &bytes, n);
    assert_eq!(decoded[1], "Null\u{0}Byte");
    assert!(decoded[2].contains("\u{1F600}"));
}
