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

1. Start the server on a machine at the **other end** of the link you want to test. Pick one:

   - **Docker:** `docker compose up -d --build`
   - **Ubuntu/Debian without Docker:** `sudo bash deploy/install-debian.sh` (see [below](#run-the-server-on-ubuntudebian-without-docker))

   Open these ports in the firewall: `7001/udp` and `7002/tcp`.

2. On Windows, run `target\release\network-tester.exe` (build it first, see below).
3. Type the server address and press **Start test**.

## The Windows app

- **Overview:** download, upload, latency and jitter at a glance. It also shows how well the link suits video streaming, gaming and video calls, and a list of what was found. When an earlier run exists, each number shows the change against it.
- **Speed:** a live chart of the speed test.
- **Latency & jitter:** UDP ping, TCP ping and jitter in detail.
- **Breaking point:** heat maps of packet size against send rate. Green is clean, amber is an early warning, red is broken. Hover a cell for the details.
- **Choose what to test** (under the server box) lets you skip tests. The time shown next to the Start button is an estimate.
- **Copy summary** puts the main numbers on the clipboard. **Light mode / Dark mode** switches the theme.

Test flags (optional): `--server <host>`, `--autostart`, `--light`, `--tab <0-3>`.

For accurate results, run the server on a Linux host with `network_mode: host`. See `docker-compose.yml`. Docker Desktop on Windows adds NAT. NAT can drop fast UDP bursts. In our test, UDP upload at 500 Mbps lost 44-79% of packets through Docker Desktop. The same test on a direct server was clean.

## Run the server on Ubuntu/Debian without Docker

The server is one small program. It needs no libraries at run time. Tested on Ubuntu 24.04.

### Option A: the install script (easiest)

On the server, get the project folder (for example `git clone` it or copy it with `scp`). Then:

```bash
cd NetworkTester
sudo bash deploy/install-debian.sh
```

The script does these steps:

1. Installs `build-essential`, `curl`, `ca-certificates` and `pkg-config` with apt.
2. Installs Rust with rustup (minimal profile) into `/opt/nt-build`, if `cargo` is not found.
3. Builds the server. The first build takes a few minutes.
4. Installs it as `/usr/local/bin/nt-server`.
5. Creates a system user `ntserver` and a systemd service `nt-server` that starts at boot.
6. Opens `7001/udp` and `7002/tcp` in `ufw`, if `ufw` is active.

When it ends, it prints the addresses of the machine. Use one of them in the Windows app.

Useful commands:

```bash
systemctl status nt-server          # is it running?
journalctl -u nt-server -f          # logs
sudo systemctl restart nt-server
sudo ss -lunp | grep 7001           # UDP port is open?
sudo ss -ltnp | grep 7002           # TCP port is open?
```

To update: get the new project files (`git pull`, or copy the folder again), then run the script again. It rebuilds, installs the new binary and restarts the service.

The script also writes `/etc/sysctl.d/99-nt-server.conf`. It raises `net.core.rmem_max` and `wmem_max` to 8 MB. Without this, Linux gives the server a small UDP buffer (about 400 KB). A small buffer drops packets at high rates, and the test then shows loss that the network did not cause. When the server starts, it prints the buffer size it got. It warns if the size is small.

To remove: `sudo bash deploy/install-debian.sh --uninstall`. It keeps `/opt/nt-build` (about 1 GB). Delete it with `sudo rm -rf /opt/nt-build`.

### Option B: by hand

```bash
sudo apt update
sudo apt install -y build-essential curl ca-certificates pkg-config
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh   # choose the default install
. "$HOME/.cargo/env"

cd NetworkTester
cargo build --release --locked -p server
sudo install -m 0755 target/release/server /usr/local/bin/nt-server

# run it in the foreground to try it
nt-server
```

To run it as a service, copy `deploy/nt-server.service` to `/etc/systemd/system/`, create the user, and start it:

```bash
sudo useradd --system --no-create-home --shell /usr/sbin/nologin ntserver
sudo cp deploy/nt-server.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now nt-server
```

### Firewall

Open **UDP 7001** and **TCP 7002**.

```bash
sudo ufw allow 7001/udp
sudo ufw allow 7002/tcp
```

On a cloud server (AWS, Azure, Google Cloud, and others), also open these ports in the provider's firewall or security group. This is the most common reason for "no answer" from the server.

### Check from the Windows side

```bash
nt-cli <server-address> ping
```

If this shows 0 packets lost, the UDP port is open. Then try `nt-cli <server-address> speed` to check the TCP port.

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
- **High-rate UDP can hit a packets-per-second limit on the PC.** On our Windows test PC, UDP stopped near 48,000 packets per second (about 570 Mbps with 1472-byte packets) in both directions on a real network adapter. We have not proven the cause. Compare with TCP speed and with `iperf3` before you decide the network is the limit. Packets larger than the MTU (for example 8000 bytes) are split into fragments. Many networks drop fragments, so those cells often fail on a healthy link.
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
deploy/          Ubuntu/Debian install script and systemd service
```
