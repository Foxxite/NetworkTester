use proto::*;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UdpSocket};

async fn handle_tcp(mut s: TcpStream) -> std::io::Result<()> {
    s.set_nodelay(true)?;
    let mut mode = [0u8; 1];
    s.read_exact(&mut mode).await?;
    let mut buf = vec![0u8; 65536];
    match mode[0] {
        MODE_ECHO => loop {
            let n = s.read(&mut buf).await?;
            if n == 0 {
                return Ok(());
            }
            s.write_all(&buf[..n]).await?;
        },
        MODE_SINK => loop {
            if s.read(&mut buf).await? == 0 {
                return Ok(());
            }
        },
        MODE_SOURCE => loop {
            s.write_all(&buf).await?; // ends when the client closes
        },
        MODE_PACED => {
            let mut p = [0u8; 6];
            s.read_exact(&mut p).await?;
            let size = u16::from_be_bytes([p[0], p[1]]) as usize;
            let pps = u32::from_be_bytes([p[2], p[3], p[4], p[5]]);
            let mbps = pps as f64 * size as f64 * 8.0 / 1e6;
            if size == 0 || pps == 0 || mbps > MAX_SRC_MBPS {
                return Ok(());
            }
            let data = vec![0u8; size];
            let start = Instant::now();
            let mut sent: u64 = 0;
            while start.elapsed().as_secs_f64() < MAX_SRC_SECS + 2.0 {
                let due = ((start.elapsed().as_secs_f64() * pps as f64) as u64 + 1).min(sent + 4096);
                while sent < due {
                    s.write_all(&data).await?; // fails when the client closes
                    sent += 1;
                }
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let udp = UdpSocket::bind(("0.0.0.0", UDP_PORT)).await?;
    let tcp = TcpListener::bind(("0.0.0.0", TCP_DATA_PORT)).await?;
    println!("udp echo on {UDP_PORT}, tcp data on {TCP_DATA_PORT}");

    tokio::spawn(async move {
        loop {
            if let Ok((s, _)) = tcp.accept().await {
                tokio::spawn(handle_tcp(s));
            }
        }
    });

    let udp = Arc::new(udp);
    let epoch = Instant::now();
    let mut sessions: HashMap<(SocketAddr, u32), (RecvTracker, Instant)> = HashMap::new();
    let mut buf = vec![0u8; 65536];
    loop {
        let (n, from) = udp.recv_from(&mut buf).await?;
        let now = Instant::now();
        let Some(h) = Header::read(&buf[..n]) else { continue };
        if h.flags & FLAG_ECHO != 0 {
            let _ = udp.send_to(&buf[..n], from).await;
        } else if h.flags & FLAG_DATA != 0 {
            let e = sessions.entry((from, h.session)).or_insert_with(|| (RecvTracker::default(), now));
            e.0.on_packet(h.seq, h.send_ts_ns, epoch.elapsed().as_nanos() as u64, n);
            e.1 = now;
        } else if h.flags & FLAG_REPORT_REQ != 0 {
            let r = sessions.get(&(from, h.session)).map(|e| e.0.report()).unwrap_or_default();
            let mut out = [0u8; HEADER_LEN + Report::LEN];
            Header { flags: FLAG_REPORT, size: out.len() as u16, ..h }.write(&mut out);
            r.write(&mut out[HEADER_LEN..]);
            let _ = udp.send_to(&out, from).await;
            if sessions.len() > 1000 {
                sessions.retain(|_, v| now.duration_since(v.1) < Duration::from_secs(60));
            }
        } else if h.flags & FLAG_SRC_REQ != 0 && n >= HEADER_LEN + 8 {
            let size = u16::from_be_bytes(buf[HEADER_LEN..HEADER_LEN + 2].try_into().unwrap());
            let pps = u32::from_be_bytes(buf[HEADER_LEN + 2..HEADER_LEN + 6].try_into().unwrap());
            let count = u32::from_be_bytes(buf[HEADER_LEN + 6..HEADER_LEN + 10].try_into().unwrap());
            tokio::spawn(send_stream(udp.clone(), from, h.session, size, pps, count));
        }
    }
}

/// Send `count` packets of `size` bytes at `pps` to `to`. Refuses anything over the safety limits.
async fn send_stream(udp: Arc<UdpSocket>, to: SocketAddr, session: u32, size: u16, pps: u32, count: u32) {
    let size = size as usize;
    let mbps = pps as f64 * size as f64 * 8.0 / 1e6;
    let secs = count as f64 / pps.max(1) as f64;
    if size < HEADER_LEN || size > MAX_UDP_PAYLOAD || pps == 0 || mbps > MAX_SRC_MBPS || secs > MAX_SRC_SECS {
        return;
    }
    let mut buf = vec![0u8; size];
    let start = Instant::now();
    let mut sent: u32 = 0;
    while sent < count {
        let due = ((start.elapsed().as_secs_f64() * pps as f64) as u64 + 1).min(count as u64) as u32;
        while sent < due {
            let h = Header { session, seq: sent, send_ts_ns: start.elapsed().as_nanos() as u64, size: size as u16, flags: FLAG_DATA };
            h.write(&mut buf);
            let _ = udp.send_to(&buf, to).await;
            sent += 1;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
}
