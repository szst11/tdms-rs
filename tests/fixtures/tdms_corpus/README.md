# TDMS Test Corpus

This directory contains the golden reference files used for integration and correctness testing.

Every `.tdms` file has a companion `.tdms_index` file (written by `nptdms` with `index_file=True`), so the tests also exercise index-file-based reads.

## Regenerating Fixtures

If you need to update or regenerate these files, use the Python scripts provided in the `tools/` directory (managed via `uv`; dependencies are listed in `requirements.txt` at the repo root).

### Steps
1. From this directory (`tests/fixtures/`), run the corpus generator:
   ```bash
   uv run --with-requirements ../../requirements.txt python ../../tools/generate_corpus.py
   ```
   The scripts use relative `CORPUS_DIR = "tdms_corpus"`, so they must be run with this directory as the working directory (not from `tools/`).
2. Regenerate the golden JSON references:
   ```bash
   uv run --with-requirements ../../requirements.txt python ../../tools/generate_json.py
   ```
3. Validate the generated files against their JSON representation:
   ```bash
   uv run --with-requirements ../../requirements.txt python ../../tools/validate_json.py
   ```

## Structure
- `01_minimal/`: Basic single-channel files.
- `03_datatypes/`: Coverage for all supported TDMS types.
- `06_properties/`: Nested metadata scenarios.
- ... and so on.

The Rust tests in `tests/golden_tests/` iterate through these directories to ensure `tdms-rs` can parse them correctly and match the expected values.
