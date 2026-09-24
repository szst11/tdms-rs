//! Shared helpers for integration tests.

/// Remove a `.tdms` file and its optional `.tdms_index` companion.
///
/// `TdmsFile::open` may generate a missing index file on the fly, so tests that
/// clean up after themselves must remove both.
pub fn remove_tdms(path: &str) {
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(format!("{}_index", path));
}
