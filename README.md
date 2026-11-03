# Gemini Key Router ⚡

A high-performance, resilient Google Gemini API Key Router, Load Balancer, and Proxy Gateway built in Rust with Tokio, Axum, SQLx + SQLite, an in-memory smart rotation engine, parallel fast tester, and a developer-first antislop-ui dashboard.

---

## 🚀 Key Highlights & Architectural Features

- **High-Capacity Key Pool**: Stores and manages up to 100+ Gemini API keys with automatic SHA-256 deduplication and in-memory read-write caching.
- **5 Smart Rotation Strategies**:
  1. `round_robin`: Atomic index increment across eligible keys.
  2. `least_recently_used`: Routes to the key with the oldest `last_used_at` timestamp.
  3. `lowest_failure_rate`: Selects the key with the minimum failure ratio ($failed / total$).
  4. `lowest_latency`: Selects the key with the lowest rolling average latency.
  5. `weighted`: Combines failure rate, latency, and idle time for optimal traffic distribution.
- **Fast Failover & Adaptive Cooldown**:
  - Automatically identifies **Temporary Rate Limits** (HTTP 429 TPM/RPM) vs **Quota Exhaustion** (daily resource quota exceeded).
  - Respects upstream `Retry-After` headers and applies exponential backoff with randomized jitter.
  - Failover retry budget ensures requests seamlessly rotate to healthy keys without client disruption.
- **Fast Parallel Tester**:
  - Validates all keys concurrently using Tokio `Semaphore` (configurable concurrency: 1–50).
  - Consumes minimal tokens (1 output token ping) to prevent quota depletion during tests.
  - Precise status classification: `ACTIVE`, `INVALID`, `RATE_LIMITED`, `QUOTA_EXHAUSTED`, `BILLING_ERROR`, `TEMPORARY_ERROR`, `DISABLED`.
- **Wire-Compatible Gemini Reverse Proxy**:
  - `POST /v1beta/models/{model}:generateContent`
  - `POST /v1beta/models/{model}:streamGenerateContent?alt=sse`
  - `GET /v1/models` & `GET /v1beta/models`
  - `GET /health`
  - Zero-copy native chunk streaming without RAM buffering (minimal TTFT).
  - Transparent passthrough for `generationConfig`, `systemInstruction`, tools, `functionDeclarations`, `toolConfig`, and multimodal images/audio/video.
  - Strips upstream API keys from headers and logs.
- **Vanilla antislop-ui Dashboard**:
  - Served directly from the Rust binary at `http://127.0.0.1:8080/`.
  - Professional developer dark theme (`#09090b`), high contrast (WCAG AA), zero AI-slop gradients.
  - Real-time Server-Sent Events (SSE) telemetry connection with auto-reconnect.
  - Bulk import drawer supporting textarea paste and `.txt` file drag-and-drop.
  - Interactive live test progress bar, masked key table, and dynamic strategy switcher.

---

## 🛠 Tech Stack

- **Backend**: Rust 2021, Tokio 1.40, Axum 0.7, Reqwest 0.12 (rustls), Tower 0.5, SQLx 0.8 + SQLite (WAL mode), Tracing 0.1, Subtle 0.2.
- **Frontend**: Pure HTML5, CSS3, Vanilla ES2022 JavaScript, Fetch API, EventSource SSE.

---

## 📦 Ubuntu VPS Installation & Deployment Guide

### 1. Prerequisites
```bash
sudo apt update && sudo apt install -y curl build-essential pkg-config libssl-dev git
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source $HOME/.cargo/env
```

### 2. Clone and Build
```bash
git clone https://github.com/dresar/gemini-router.git
cd gemini-router
cp .env.example .env

# Build optimized release binary
cargo build --release
```
The compiled executable will be located at `./target/release/gemini-router` (~13 MB).

### 3. Environment Configuration (`.env`)
```env
HOST=127.0.0.1
PORT=8080
DATABASE_URL=sqlite:data/gemini_router.db?mode=rwc
ADMIN_KEY=your-secure-admin-secret-key-here
DEFAULT_MODEL=gemini-2.5-flash
DEFAULT_STRATEGY=round_robin
TESTER_CONCURRENCY=10
REQUEST_TIMEOUT_SECS=30
RETRY_BUDGET=3
COOLDOWN_RATE_LIMIT_SECS=60
COOLDOWN_QUOTA_HOURS=12
RUST_LOG=info,gemini_router=debug
```

### 4. Running as a Systemd Service
Create the service unit file:
```bash
sudo cp gemini-router.service /etc/systemd/system/gemini-router.service
sudo systemctl daemon-reload
sudo systemctl enable --now gemini-router
```
Check service status:
```bash
sudo systemctl status gemini-router
journalctl -u gemini-router -f
```

---

## 💻 Dashboard User Guide

1. Open `http://127.0.0.1:8080` in your web browser.
2. Enter your `ADMIN_KEY` configured in `.env` to unlock the dashboard.
3. **Import Keys**:
   - Click **Import Keys** on the toolbar.
   - Paste keys (one per line, comma-separated, or space-separated) or drag-and-drop a `.txt` file containing keys.
   - The system automatically deduplicates and hashes keys before saving.
4. **Test Keys**:
   - Click **Test All Keys** to trigger a parallel batch health check with live progress.
   - Click **Test** on any individual key row to test that key on-demand.
5. **Switch Strategy**:
   - Click **Settings** to change the active rotation strategy (`Round Robin`, `Least Recently Used`, `Lowest Failure Rate`, `Lowest Latency`, or `Weighted`).

---

## 🔌 API Endpoint Integration Examples

### Standard Content Generation
Replace `https://generativelanguage.googleapis.com` with your router URL:
```bash
curl -X POST http://127.0.0.1:8080/v1beta/models/gemini-2.5-flash:generateContent \
  -H "Content-Type: application/json" \
  -d '{
    "contents": [{
      "parts": [{"text": "Explain quantum computing in one sentence."}]
    }]
  }'
```

### Native Streaming Generation
```bash
curl -N -X POST "http://127.0.0.1:8080/v1beta/models/gemini-2.5-flash:streamGenerateContent?alt=sse" \
  -H "Content-Type: application/json" \
  -d '{
    "contents": [{
      "parts": [{"text": "Write a short poem about Rust."}]
    }]
  }'
```

### Python SDK Integration
```python
import google.generativeai as genai

# Point client to Gemini Key Router (API Key can be dummy or admin key)
genai.configure(
    api_key="router-managed",
    client_options={"api_endpoint": "http://127.0.0.1:8080"}
)

model = genai.GenerativeModel("gemini-2.5-flash")
response = model.generate_content("Hello from Python!")
print(response.text)
```

---

## 🧪 Automated Test Verification

Run all unit, integration, and mock tests:
```bash
cargo test --all
```

Test Results Summary:
- `backend_core_tests.rs`: 4 passed (SQLite pool, migrations, key masking, admin auth).
- `rotation_and_tester_tests.rs`: 9 passed (5 strategies, cooldown expiry, 429 vs quota classifier, parallel runner).
- `proxy_mock_tests.rs`: 5 passed (generateContent, stream SSE, 429 failover to secondary key, tool calling preservation, auth gate).
- Total: **18 passed, 0 failed**.

---

## 📄 License
MIT License.
