# Spikes

Findings only (numbers, yes/no, how). No code to keep.

## S1 — librqbit API check (M0-02) · 2026-10-14

Source: `ikatson/rqbit` master, `crates/librqbit`, version 9.0.1, Apache-2.0. Verified by reading source, not by running it.

| Question | Answer | Evidence / consequence |
|---|---|---|
| IP blocklist, inbound and outbound | **Yes** | `SessionOptions.blocklist_url` and `allowlist_url`; checked on incoming (`session.rs` ~924) and on outgoing connects (`torrent_state/live/mod.rs` ~637). |
| Blocklist formats | **Partial** | Plain text `name:start-end` per line (`#` comments), IPv4 and IPv6, gzip auto-detected, `file://` URLs accepted. **Not** supported: eMule `ipfilter.dat` (`a.b.c.d - e.f.g.h , level , desc`), zip. Our `core::ipfilter` downloads, unzips, validates, and writes a normalized `name:start-end` file that we hand over via `file://`. |
| Hot-swap blocklist at runtime | **No** | `Session.blocklist` is a plain `IpRanges` loaded once at start. Updates need a session rebuild (fast with `fastresume`) or a small upstream patch (swap in an `ArcSwap`). Decide in M5-05; start with rebuild. |
| Runtime rate limits | **Yes** | `Limits::set_upload_bps / set_download_bps` (ArcSwap-backed). |
| File re-selection after add | **Yes** | `Session::update_only_files`. |
| Pause / resume / delete(keep or delete files) | **Yes** | `Session::pause`, `unpause`, `delete(id, delete_files)`. |
| Recheck, force re-announce | **Not found** | No such functions in the torrent state or API. Drop "recheck" and "force announce" from v1, or emulate later. |
| Session persistence, fast resume | **Yes** | `SessionOptions.persistence`, `fastresume`. |
| Stats: uploaded and downloaded bytes, per torrent | **Yes** | `TorrentStats` carries `uploaded_bytes`, `progress_bytes`; live counters in `torrent_state/live`. |
| HTTP streaming of a file | **Yes** | `Api::api_stream` returns a `FileStream` (seekable, piece-prioritised); HTTP handler is behind the `http-api` feature. We serve our own loopback endpoint from the `FileStream`. |
| uTP | **Yes** | `ListenerOptions.utp_opts` (separate crate `librqbit-utp`). Must be provided to enable. |
| UPnP | **Yes** | `ListenerOptions.enable_upnp_port_forwarding`. |
| Bind to interface | **Yes, untested on Windows** | `SessionOptions.bind_device_name` (docs describe Linux/macOS mechanisms). Needs a Windows test before M5-01. |
| SOCKS5 proxy | **Yes** | `ConnectionOptions` with `SocksProxyConfig`. |
| Encryption (MSE/PE) | **No** | No code found. Peer-count impact measured in S2. |
| Peer limit per torrent | **Yes** | `SessionOptions.peer_limit`. |
| IPv4-only toggle, LSD toggle | **Yes** | `ipv4_only`, `disable_local_service_discovery`. |

Consequences for the plan:
1. FEATURES: IP filter is feasible without forking (rebuild-on-update); recheck and force announce are out of v1.
2. ROADMAP M5-05 reworded; M5-07/08 stay (we own format conversion and safety checks).
3. Streaming (M4-04) is feasible via `FileStream`.
4. Risk R1 (no MSE) remains open until S2.

### Notes from building the adapter (M0-06)
- uTP is **off by default** in librqbit (`ListenerMode::TcpOnly`, upstream "once uTP is stable"). We enable `TcpAndUtp` only if S2 shows it helps.
- DHT is on by default in `SessionOptions`; we turn it off for loopback tests.
- Re-adding a magnet for a managed torrent makes librqbit try to resolve metadata first; the adapter checks the info-hash itself.
- Loopback test (a real librqbit seeder session vs. our adapter, 127.0.0.1, no DHT/trackers) downloads 3 MiB in about 0.25 s.

## S2 — encryption and uTP peer-count spike (M0-03)
Not run yet.

## S3 — Windows I/O spike (M0-04)
Not run yet.
