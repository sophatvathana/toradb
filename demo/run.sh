#!/usr/bin/env bash
# Start ToraDB demo API + web UI.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

DB_PATH="${TORADB_DB_PATH:-$ROOT/data/msmarco_1m}"
API_DIR="$ROOT/demo/api"
WEB_DIR="$ROOT/demo/web"
REQ="$API_DIR/requirements.txt"

pick_python() {
  # Prefer an environment that already has toradb + fastapi (e.g. repo .venv).
  local candidates=()
  if [[ -n "${VIRTUAL_ENV:-}" && -x "${VIRTUAL_ENV}/bin/python" ]]; then
    candidates+=("${VIRTUAL_ENV}/bin/python")
  fi
  if [[ -x "$ROOT/.venv/bin/python" ]]; then
    candidates+=("$ROOT/.venv/bin/python")
  fi
  if [[ -x "$API_DIR/.venv/bin/python" ]]; then
    candidates+=("$API_DIR/.venv/bin/python")
  fi
  candidates+=("$(command -v python3)")

  local py
  for py in "${candidates[@]}"; do
    [[ -n "$py" && -x "$py" ]] || continue
    if "$py" -c "import toradb, fastapi" 2>/dev/null; then
      echo "$py"
      return 0
    fi
  done
  echo ""
}

ensure_api_deps() {
  local py="$1"
  if "$py" -c "import fastapi, uvicorn, pyarrow" 2>/dev/null; then
    return 0
  fi
  echo "==> Installing API dependencies into $(dirname "$py")"
  "$py" -m pip install -q -r "$REQ"
  "$py" -c "import fastapi, uvicorn, pyarrow"
}

if [[ ! -d "$DB_PATH" ]]; then
  echo "==> Building demo database at $DB_PATH"
  PY_BOOT="$(pick_python || true)"
  if [[ -z "$PY_BOOT" ]] || ! "$PY_BOOT" -c "import toradb" 2>/dev/null; then
    echo "Install ToraDB first from repo root: maturin develop"
    exit 1
  fi
  "$PY_BOOT" examples/full_example.py
fi

PYTHON="$(pick_python)"
if [[ -z "$PYTHON" ]]; then
  echo "==> No Python with toradb found; creating demo/api/.venv"
  python3 -m venv "$API_DIR/.venv"
  PYTHON="$API_DIR/.venv/bin/python"
  "$PYTHON" -m pip install -q -U pip
fi

if ! "$PYTHON" -c "import toradb" 2>/dev/null; then
  echo "error: toradb is not installed for $PYTHON"
  echo "From repo root with your venv active: maturin develop"
  exit 1
fi

ensure_api_deps "$PYTHON"

if [[ ! -d "$WEB_DIR/node_modules" ]]; then
  echo "==> Installing web dependencies"
  (cd "$WEB_DIR" && npm install)
fi

export TORADB_DB_PATH="$DB_PATH"
export TORADB_HOST="${TORADB_HOST:-127.0.0.1}"
export TORADB_PORT="${TORADB_PORT:-8787}"
# ~45 segment BM25 sidecars @ ~130MB each; default 256MB cache causes constant eviction.
export TORADB_CACHE_INDEX_BYTES="${TORADB_CACHE_INDEX_BYTES:-8589934592}"
# Warmup runs in a subprocess after bind (see demo/api/main.py); hides ~20s first-search latency.
export TORADB_WARMUP_ON_START="${TORADB_WARMUP_ON_START:-1}"

cleanup() {
  [[ -n "${API_PID:-}" ]] && kill "$API_PID" 2>/dev/null || true
}
trap cleanup EXIT INT TERM

echo "==> API http://${TORADB_HOST}:${TORADB_PORT}"
echo "    python: $PYTHON"
echo "    db:     $DB_PATH"

"$PYTHON" "$API_DIR/main.py" &
API_PID=$!

echo "==> Waiting for API..."
# Large segment_only DBs can take 20–30s to open with a multi-GB index cache.
READY_WAIT_SECS="${TORADB_API_READY_SECS:-90}"
ready=0
for _ in $(seq 1 $((READY_WAIT_SECS * 2))); do
  if curl -sf "http://${TORADB_HOST}:${TORADB_PORT}/api/health" >/dev/null 2>&1; then
    ready=1
    break
  fi
  if ! kill -0 "$API_PID" 2>/dev/null; then
    echo "error: API process exited. Run manually to see the traceback:"
    echo "  $PYTHON $API_DIR/main.py"
    exit 1
  fi
  sleep 0.5
done

if [[ "$ready" -ne 1 ]]; then
  echo "error: API did not become ready on port ${TORADB_PORT}"
  kill "$API_PID" 2>/dev/null || true
  exit 1
fi

if [[ "${TORADB_WARMUP_ON_START}" == "1" ]]; then
  echo "==> BM25 cache warmup running in background (first search faster after ~20s)"
fi

echo "==> Web http://127.0.0.1:5173"
cd "$WEB_DIR"
exec npm run dev
