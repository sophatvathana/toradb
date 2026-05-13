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

Optional: `pip install pandas pyarrow` for dataframe / Arrow ingest.
