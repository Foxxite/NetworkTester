# NetworkTester

A tool to find network problems. It shows:

- **Ping** (UDP and TCP)
- **Jitter**
- **Download and upload speed** (TCP)
- **Breakdown point**: the packet size and rate where UDP and TCP start to fail
- **Path MTU**: the largest packet that is not split up

It has two parts:

| Part | What it is | Where it runs |
|---|---|---|
| `server` | Echo, sink and source for UDP and TCP | Docker (any Linux host or VPS) |
| `network-tester` | Windows app (egui, one `.exe`) | Your PC |

A web page cannot do this test. A browser cannot send raw UDP, set the packet size, or set the Don't-Fragment bit. This is why the client is a native app.

## Quick start

1. Start the server on a machine at the **other end** of the link you want to test:

   ```bash
   docker compose up -d --build
   ```

   Open these ports in the firewall: `7001/udp` and `7002/tcp`.

2. On Windows, run `target\release\network-tester.exe` (build it first, see below).
3. Type the server address and press **Start**.

For accurate results, run the server on a Linux host with `network_mode: host`. See `docker-compose.yml`. Docker Desktop on Windows adds NAT. NAT can drop fast UDP bursts. In our test, UDP upload at 500 Mbps lost 44-79% of packets through Docker Desktop. The same test on a direct server was clean.

## Build

You need Rust (rustup).

```bash
cargo build --release          # builds everything
cargo test                     # unit tests
```

Output is in `target/release/`:

- `network-tester.exe` - the Windows app
- `server` - the server (the Docker image builds this)
- `nt-cli` - command-line client, for quick checks
- `impair` - test tool: a proxy that adds loss, rate limit, delay, jitter and an MTU limit

## What each test does

| Test | Method |
|---|---|
| UDP ping | 100 probes, 20 ms apart. Shows min, average, max, 95th percentile, loss. |
| TCP ping | 100 round trips of 8 bytes on an open connection. The handshake is not counted. |
| Jitter | 50 packets per second for 10 s. RFC 3550 method (mean change between packets). Also shows RTT spread. |
| Speed | 4 parallel TCP streams for 10 s. The first 2 s are not counted. Download, then upload. |
| MTU | UDP echo with the Don't-Fragment bit set. Binary search from 576 to 9000 bytes. IPv4 only. |
| UDP breakdown | For each packet size and each rate, send for 2 s. The server counts loss, reordering and jitter. Upload and download. |
| TCP breakdown | For each write size and each rate, send for 2 s. Measures goodput, RTT under load and stalls. Upload and download. |

### When is a cell "bad"?

**UDP:** loss above 1%, or jitter above 3 times the baseline (and above 1 ms), or the sender could not reach the rate.

**TCP:** goodput below 90% of the target, or a stall longer than 500 ms, or the 95th percentile RTT under load above 5 times the idle RTT (and 30 ms above it).

A size stops after two bad rates in a row. The heat map shows those cells in grey.

The **Quick grid** option uses fewer sizes and rates (about 4 minutes for all four sweeps). Clear it for the full grid. The full grid takes much longer.

## Command line

```bash
nt-cli <server> ping
nt-cli <server> jitter
nt-cli <server> speed
nt-cli <server> mtu
nt-cli <server> udp-sweep      [sizes] [rates] [secs]     # upload
nt-cli <server> udp-sweep-down [sizes] [rates] [secs]
nt-cli <server> tcp-sweep      [sizes] [rates] [secs]
nt-cli <server> tcp-sweep-down [sizes] [rates] [secs]
# example: nt-cli 10.0.0.5 udp-sweep 512,1472 10,100,500 2
```

## Results

Each run is saved in `NetworkTester-results\` next to where you start the app:

- `run-<time>.json` - everything, including the numbers shown in the Latency tab for comparison with the next run
- `run-<time>.csv` - the sweep cells, for a spreadsheet

## Testing without a bad network

`impair` sits between the client and the server. Start the server on `127.0.0.1`, then:

```bash
# impair <listen_ip> <upstream_ip> [loss_%] [rate_Mbps] [delay_ms] [jitter_ms] [queue_ms] [mtu]
impair 127.0.0.2 127.0.0.1 0.3 50 0 0 20 1500
nt-cli 127.0.0.2 udp-sweep 1200 10,25,50,100
```

Loss, delay and jitter apply to UDP only. The rate limit applies to UDP and TCP. On Windows, timers are coarse (about 15 ms), so delay and rate are approximate.

## Limits you should know

- **Small writes at high rates can fail because of your PC**, not the network. If a small size fails but a larger size passes at the same rate, suspect the PC.
- **Windows UDP pacing** uses a short busy-wait. A very high packet rate can use a full CPU core. The UDP sweep marks a cell "sender too slow" when the PC cannot reach the rate.
- **MTU:** the server echoes the full packet. A smaller MTU on the return path only shows up if it drops the packet. IPv6 is not supported.
- **TCP retransmit counts** are not shown yet.
- **No authentication.** The server limits server-sent streams to 2000 Mbps and 30 s. Do not leave it open on the public internet for long.
- **Port 7000** (control) is reserved in the code. The server does not use it.

## Layout

```
crates/proto     packet header, report format, constants
crates/server    the Docker server (tokio)
crates/engine    test logic (ping, jitter, speed, sweeps, MTU)
crates/cli       nt-cli
crates/client    Windows app (egui)
crates/impair    test proxy
docker/          Dockerfile (release) and test.Dockerfile (Linux test image)
```
