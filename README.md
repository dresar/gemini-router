# Gemini Key Router

A production-ready, ultra-low-latency Google Gemini API Key Router and Gateway built in Rust with Tokio, Axum, SQLx + SQLite, in-memory smart rotation engine, parallel fast tester, and an antislop-ui dashboard.

## Features
- **Capacity**: Manages up to 100+ Gemini API keys with automatic deduplication.
- **5 Smart Rotation Strategies**: Round Robin, Least Recently Used (LRU), Lowest Failure Rate, Lowest Latency, Weighted Selection.
- **Fast Failover**: Automatic detection of 429 rate limits vs quota exhaustion, honoring `Retry-After` headers and jittered backoff.
- **Parallel Fast Tester**: Tokio concurrency-limited test runner with minimal token consumption (1 token ping).
- **Gemini Native Proxy**: Full wire-compatibility for `generateContent` and streaming `streamGenerateContent`, preserving tools/function calling and multimodal payloads.
- **Antislop-UI Dashboard**: Clean, responsive developer dashboard in pure HTML5/CSS3/Vanilla JS served by Axum, updated in real time via Server-Sent Events (SSE).
- **Security**: Constant-time admin authentication, masked key storage/logging, SQLite WAL mode.

## Quick Start
```bash
cp .env.example .env
cargo run --release
```

Open dashboard at `http://127.0.0.1:8080` and log in with the admin key configured in `.env`.
