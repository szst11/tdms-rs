# Development Tools

Python scripts for TDMS corpus generation and debugging.

## Requirements
- Python 3.9+
- Python dependencies are managed with `uv` and listed in `requirements.txt` at the repo root (numpy, nptdms, psutil, PyYAML).

## Scripts
- `generate_corpus.py` - Generate test TDMS files (and `.tdms_index` companion files)
- `generate_json.py` - Create golden reference JSON
- `validate_json.py` - Verify JSON/TDMS consistency
- `debug_hex.py` - Hex dump TDMS files
- `debug_props.py` - Extract TDMS properties

## Usage

`generate_corpus.py`/`generate_json.py`/`validate_json.py` use a *relative* `CORPUS_DIR = "tdms_corpus"` path, so they must be run from `tests/fixtures/`:

```bash
cd tests/fixtures/
uv run --with-requirements ../../requirements.txt python ../../tools/generate_corpus.py
uv run --with-requirements ../../requirements.txt python ../../tools/generate_json.py
uv run --with-requirements ../../requirements.txt python ../../tools/validate_json.py
```

## Note
These tools are for development only. The Rust crate has no Python dependencies.
