# ToraDB Python examples

## Setup

From the repo root:

```bash
python3 -m venv .venv
source .venv/bin/activate
pip install maturin
maturin develop
```

## Run

```bash
python examples/full_example.py
```

Optional: `pip install pandas pyarrow` for dataframe / Arrow ingest (`add_arrow` uses zero-copy PyCapsule import in Rust).

CLI after install:

```bash
toradb smoke
toradb query ./examples/_demo_db articles "Nikola Tesla motor"
```
