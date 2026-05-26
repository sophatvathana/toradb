# Install

## Prerequisites

- Rust stable toolchain (`rustup`, `cargo`)
- Python 3.8+
- `pip`
- `maturin` (for building/installing the Python extension)

The repo pins Rust to stable via `rust-toolchain.toml`.

## 1) Clone and enter repository

```bash
git clone https://github.com/sophatvathana/toradb.git toradb
cd toradb
```

## 2) Build Rust workspace

```bash
cargo build
```

## 3) Set up Python environment

```bash
python3 -m venv .venv
source .venv/bin/activate
pip install --upgrade pip
pip install maturin
```

## 4) Install ToraDB Python package locally

```bash
maturin develop
```

## 5) Verify installation

```bash
python -c "import toradb; print('toradb import ok')"
toradb smoke
```

## Run tests

Rust tests:

```bash
cargo test
```

Python smoke tests:

```bash
pytest tests/python_smoke.py -q
```

## Optional benchmark run

```bash
cargo bench -p toradb-storage --bench segment_read
```

## Troubleshooting

- If `maturin develop` fails, confirm the virtual environment is activated.
- If Python cannot import `toradb`, rerun `maturin develop`.
- If command `toradb` is not found, ensure the venv `bin` directory is on your shell path (activation usually handles this).
