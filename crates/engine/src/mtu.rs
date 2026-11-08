//! Path MTU probe: UDP echo with the Don't-Fragment bit set.
use proto::*;
use std::io;
use std::net::{ToSocketAddrs, UdpSocket};
use std::time::Duration;

/// IPv4 header (20) + UDP header (8).
const OVERHEAD: u32 = 28;

#[cfg(windows)]
fn set_df(sock: &UdpSocket) -> io::Result<()> {
    use std::os::windows::io::AsRawSocket;
    use windows_sys::Win32::Networking::WinSock::{setsockopt, IPPROTO_IP, IP_DONTFRAGMENT};
    let on: u32 = 1;
    let r = unsafe {
        setsockopt(sock.as_raw_socket() as usize, IPPROTO_IP, IP_DONTFRAGMENT, &on as *const u32 as *const u8, 4)
    };
    if r == 0 { Ok(()) } else { Err(io::Error::last_os_error()) }
}

#[cfg(unix)]
fn set_df(sock: &UdpSocket) -> io::Result<()> {
    use std::os::fd::AsRawFd;
    let v: libc::c_int = libc::IP_PMTUDISC_DO;
    let r = unsafe {
        libc::setsockopt(sock.as_raw_fd(), libc::IPPROTO_IP, libc::IP_MTU_DISCOVER, &v as *const _ as *const libc::c_void, 4)
    };
    if r == 0 { Ok(()) } else { Err(io::Error::last_os_error()) }
}

/// Largest IP packet (in bytes) that crosses the path unfragmented, found by binary search.
/// Returns None if even 576 bytes does not get through. IPv4 only.
/// The server echoes the full packet back, so a smaller MTU on the return path also shows up
/// as a failure only if the return packet is dropped, not if it is fragmented.
pub fn probe_path_mtu_blocking(server: &str) -> io::Result<Option<u32>> {
    let addr = (server, UDP_PORT)
        .to_socket_addrs()?
        .find(|a| a.is_ipv4())
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no IPv4 address for server"))?;
    let sock = UdpSocket::bind("0.0.0.0:0")?;
    sock.connect(addr)?;
    set_df(&sock)?;
    sock.set_read_timeout(Some(Duration::from_millis(400)))?;

    let mut seq = 0u32;
    let mut works = |payload: u32| -> bool {
        let mut buf = vec![0u8; payload as usize];
        let mut rbuf = vec![0u8; 65536];
        for _ in 0..3 {
            seq += 1;
            Header { session: 7, seq, send_ts_ns: 0, size: payload as u16, flags: FLAG_ECHO }.write(&mut buf);
            if sock.send(&buf).is_err() {
                return false; // too big for the local interface
            }
            while let Ok(n) = sock.recv(&mut rbuf) {
                if n == payload as usize && Header::read(&rbuf[..n]).map(|h| h.seq) == Some(seq) {
                    return true;
                }
            }
        }
        false
    };

    let (mut lo, mut hi) = (576 - OVERHEAD, 9000 - OVERHEAD);
    if !works(lo) {
        return Ok(None);
    }
    if works(hi) {
        return Ok(Some(hi + OVERHEAD));
    }
    while hi - lo > 1 {
        let mid = (lo + hi) / 2;
        if works(mid) { lo = mid } else { hi = mid }
    }
    Ok(Some(lo + OVERHEAD))
}
