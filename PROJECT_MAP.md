# PROJECT_MAP.md

## Project: Gemini Key Router (Rust / Axum)
Production-grade Gemini API Key Router, Load Balancer, Parallel Tester, and antislop-ui Dashboard.

### Architecture & Tech Stack
- Language: Rust 2021
- Runtime: Tokio 1.40 (Full async I/O)
- Web Framework: Axum 0.7 + Tower 0.5 + Tower-HTTP 0.5
- Persistence: SQLx 0.8 + SQLite (WAL mode, connection pooling)
- HTTP Client: Reqwest 0.12 (rustls, connection reuse, chunk streaming)
- Frontend: HTML5, CSS3, Vanilla JS (antislop-ui developer dark theme, SSE real-time updates)

### Directory Structure & Responsibilities
- `Cargo.toml`: Root dependencies and crate definitions.
- `migrations/0001_init.sql`: SQLite schema for `api_keys` and `system_settings`.
- `static/`: Frontend dashboard (ANTISLOP-UI):
  - `index.html`: Responsive dashboard markup, header, metrics grid, test progress bar, rotation monitor, keys table, modals (login, bulk import, add key, settings).
  - `style.css`: ANTISLOP-UI developer dark theme (#09090b, #18181b, #27272a, #10b981), mobile touch-friendly (>=44px), zero slop/gradients.
  - `app.js`: Vanilla JS client, auto-reconnecting SSE to `/admin/events`, Bearer auth, REST handlers, real-time key counter & TXT drag-drop.
- `src/`:
  - `lib.rs`: Library root exporting admin, config, database, gemini, key_manager, proxy, rotation, streaming, tester, and AppState.
  - `main.rs`: Entrypoint, server boot, proxy/admin router mount, static/ file serving, graceful shutdown.
  - `config/`: Environment configuration, masking utilities, and gemini base url configuration.
  - `database/`: SQLx connection pool & migrations runner.
  - `key_manager/`: SQLite repository & thread-safe in-memory key cache.
  - `rotation/`: 5 rotation strategies (RoundRobin, LRU, LowestFailure, LowestLatency, Weighted), failover & cooldown engine.
  - `tester/`: Parallel tester with Tokio semaphore and Gemini response classifier.
  - `proxy/`: Transparent Gemini reverse proxy with key rotation, failover, and tools preservation (`handler.rs`, `mod.rs`).
  - `streaming/`: Native chunked SSE streamer via Reqwest bytes_stream (`sse.rs`, `mod.rs`).
  - `gemini/`: Gemini request/response models and error parser.
  - `admin/`: REST endpoints for dashboard, bulk import, test triggers, rotation info, and auth middleware (`handlers.rs`, `auth.rs`, `mod.rs`).
  - `middleware/`: Security, logging with key masking, CORS.
  - `monitoring/`: Real-time telemetry, SSE broadcast channel.
- `tests/`:
  - `backend_core_tests.rs`: Core unit & integration test suite for database, key repository, auth, and masking.
  - `rotation_and_tester_tests.rs`: Rotation strategies, cooldown recovery, 429 classification, failover, and parallel tester mock suite.
  - `proxy_mock_tests.rs`: WireMock integration test suite for generateContent, SSE streaming, failover on 429, tools preservation, and admin auth protection.

### Roles & Branch Owners
- AGY01 (Lead Orchestrator): Architecture, worktree coordination, integration merge.
- AGY02 (Backend Core / `work/agy02-backend`): Cargo.toml, config, database migrations, key repository, admin auth, main.rs.
- AGY03 (Rotation & Tester / `work/agy03-rotation`): Rotation strategies, failover engine, parallel fast tester.
- AGY04 (Frontend / `work/agy04-frontend`): static/index.html, static/style.css, static/app.js (antislop-ui).
- AGY05 (Proxy & Tests / `work/agy05-proxy`): Gemini proxy handler, streaming SSE, wiremock test suite.
