#!/usr/bin/env bash
set -euo pipefail

# ==============================================================================
# High-Concurrency (700 VU) & Memory Footprint (< 50 MB RSS) Verification Script
# ==============================================================================

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT_DIR}"

MAX_RSS_LIMIT_KB=51200 # 50 MB in KB
CONCURRENCY=700
DURATION="10s"

# 1. Raise file descriptor soft limit to accommodate 700+ sockets (client + server)
ulimit -n 65536 2>/dev/null || ulimit -n 16384 2>/dev/null || ulimit -n 10240 2>/dev/null || true
echo "[1/6] Active file descriptor limit (ulimit -n): $(ulimit -n)"

# 2. Build optimized release binary so RSS measurements reflect rusty-loader directly
echo "[2/6] Building release binary (cargo build --release --bin rusty-loader)..."
cargo build --release --bin rusty-loader

BINARY="${ROOT_DIR}/target/release/rusty-loader"
if [[ ! -x "${BINARY}" ]]; then
    echo "ERROR: Release binary not found or not executable at ${BINARY}" >&2
    exit 1
fi

TMP_DIR="$(mktemp -d)"
PORT_FILE="${TMP_DIR}/server_port.txt"
REPORT_JSON="${TMP_DIR}/report_700_vu.json"
SERVER_PID=""
LOADER_PID=""

cleanup() {
    if [[ -n "${LOADER_PID}" ]] && kill -0 "${LOADER_PID}" 2>/dev/null; then
        kill "${LOADER_PID}" 2>/dev/null || true
    fi
    if [[ -n "${SERVER_PID}" ]] && kill -0 "${SERVER_PID}" 2>/dev/null; then
        kill "${SERVER_PID}" 2>/dev/null || true
        wait "${SERVER_PID}" 2>/dev/null || true
    fi
    rm -rf "${TMP_DIR}"
}
trap cleanup EXIT

# 3. Start lightweight local async HTTP/1.1 keep-alive mock server
echo "[3/6] Starting local async HTTP/1.1 keep-alive mock server..."
python3 - "${PORT_FILE}" <<'PYEOF' &
import asyncio
import sys

port_file = sys.argv[1]
BODY = b'{"status":"ok"}'
RESPONSE = (
    b"HTTP/1.1 200 OK\r\n"
    b"Content-Type: application/json\r\n"
    b"Content-Length: 15\r\n"
    b"Connection: keep-alive\r\n"
    b"\r\n" + BODY
)

async def handle_client(reader: asyncio.StreamReader, writer: asyncio.StreamWriter) -> None:
    try:
        while True:
            data = await reader.read(4096)
            if not data:
                break
            writer.write(RESPONSE)
            await writer.drain()
    except Exception:
        pass
    finally:
        try:
            writer.close()
        except Exception:
            pass

async def main() -> None:
    server = await asyncio.start_server(
        handle_client, host="127.0.0.1", port=0, backlog=2048
    )
    port = server.sockets[0].getsockname()[1]
    with open(port_file, "w", encoding="utf-8") as f:
        f.write(str(port))
        f.flush()
    async with server:
        await server.serve_forever()

asyncio.run(main())
PYEOF
SERVER_PID=$!

# Wait for the mock server to bind and write its ephemeral port
for _ in {1..50}; do
    if [[ -s "${PORT_FILE}" ]]; then
        break
    fi
    sleep 0.1
done

if [[ ! -s "${PORT_FILE}" ]]; then
    echo "ERROR: Local mock server failed to start within 5 seconds." >&2
    exit 1
fi

PORT="$(cat "${PORT_FILE}")"
TARGET_URL="http://127.0.0.1:${PORT}/ping"
echo "      Mock server listening on ${TARGET_URL} (PID ${SERVER_PID})"

# 4. Launch rusty-loader with 700 concurrent virtual users for 10s
echo "[4/6] Launching rusty-loader (-c ${CONCURRENCY} -d ${DURATION})..."
"${BINARY}" \
    --url "${TARGET_URL}" \
    --concurrency "${CONCURRENCY}" \
    --duration "${DURATION}" \
    --timeout "5000ms" \
    --output-json "${REPORT_JSON}" &
LOADER_PID=$!

# 5. Sample RSS every 500ms and monitor socket usage while rusty-loader is running
echo "[5/6] Sampling RSS memory and active sockets every 500ms (PID ${LOADER_PID})..."
PEAK_RSS_KB=0
SAMPLE_COUNT=0
PEAK_SOCKETS=0

while kill -0 "${LOADER_PID}" 2>/dev/null; do
    CURRENT_RSS_KB="$(ps -o rss= -p "${LOADER_PID}" 2>/dev/null | tr -d ' ' || true)"
    if [[ -n "${CURRENT_RSS_KB}" && "${CURRENT_RSS_KB}" =~ ^[0-9]+$ ]]; then
        SAMPLE_COUNT=$((SAMPLE_COUNT + 1))
        if (( CURRENT_RSS_KB > PEAK_RSS_KB )); then
            PEAK_RSS_KB="${CURRENT_RSS_KB}"
        fi
        CURRENT_MB="$(awk "BEGIN { printf \"%.2f\", ${CURRENT_RSS_KB} / 1024 }")"

        if command -v lsof >/dev/null 2>&1; then
            OPEN_SOCKETS="$(lsof -n -P -p "${LOADER_PID}" 2>/dev/null | grep -c "TCP" || true)"
            if (( OPEN_SOCKETS > PEAK_SOCKETS )); then
                PEAK_SOCKETS="${OPEN_SOCKETS}"
            fi
        fi

        echo "      Sample #${SAMPLE_COUNT}: RSS = ${CURRENT_RSS_KB} KB (${CURRENT_MB} MB) | Active TCP sockets = ${OPEN_SOCKETS:-N/A}"
    fi
    sleep 0.5
done

# Wait for rusty-loader to exit and check its exit status
wait "${LOADER_PID}"
LOADER_EXIT=$?
LOADER_PID=""

if (( LOADER_EXIT != 0 )); then
    echo "ERROR: rusty-loader exited with non-zero code ${LOADER_EXIT}" >&2
    exit 1
fi

PEAK_MB="$(awk "BEGIN { printf \"%.2f\", ${PEAK_RSS_KB} / 1024 }")"
echo "[6/6] Verification Summary:"
echo "      - Samples collected : ${SAMPLE_COUNT}"
echo "      - Peak RSS Memory   : ${PEAK_RSS_KB} KB (${PEAK_MB} MB)"
echo "      - RSS Memory Limit  : ${MAX_RSS_LIMIT_KB} KB (50.00 MB)"
echo "      - Peak TCP Sockets  : ${PEAK_SOCKETS} (expected ~${CONCURRENCY} pooled keep-alive connections)"

if (( SAMPLE_COUNT == 0 || PEAK_RSS_KB == 0 )); then
    echo "ERROR: Failed to capture any RSS samples from rusty-loader." >&2
    exit 1
fi

if (( PEAK_RSS_KB >= MAX_RSS_LIMIT_KB )); then
    echo "FAIL: Peak RSS (${PEAK_RSS_KB} KB) exceeded 50 MB threshold (${MAX_RSS_LIMIT_KB} KB)!" >&2
    exit 1
fi

# Verify JSON output integrity and socket pooling health
python3 - "${REPORT_JSON}" "${CONCURRENCY}" <<'PYEOF'
import json
import sys

report_path = sys.argv[1]
concurrency = int(sys.argv[2])

with open(report_path, "r", encoding="utf-8") as f:
    report = json.load(f)

total_req = report["total_requests"]
success_req = report["successful_requests"]
transport_err = report["transport_errors"]
rps = report["requests_per_sec"]
p50 = report["latencies"]["p50_ms"]
p99 = report["latencies"]["p99_ms"]

print(f"      - Total Requests    : {total_req}")
print(f"      - Successful (200)  : {success_req}")
print(f"      - Transport Errors  : {transport_err}")
print(f"      - Avg Throughput    : {rps:.1f} req/s")
print(f"      - Latency p50 / p99 : {p50:.2f} ms / {p99:.2f} ms")

assert report["concurrency"] == concurrency, f"expected concurrency {concurrency}, got {report['concurrency']}"
assert total_req > 1000, f"expected > 1000 total requests across 10s, got {total_req}"
assert success_req > 0, "expected successful requests > 0"
PYEOF

echo "PASS: 700 VU load test completed with Peak RSS ${PEAK_MB} MB (< 50 MB limit)."
