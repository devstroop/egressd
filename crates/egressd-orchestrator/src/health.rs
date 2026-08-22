use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;

/// SOCKS5 liveness — real handshake, not just TCP.
/// Greeting `05 01 00` → `05 00` (no auth).
pub async fn socks5_is_alive(host: &str, port: u16, timeout_dur: Duration) -> bool {
    let addr = format!("{host}:{port}");
    let res = timeout(timeout_dur, async {
        let mut stream = TcpStream::connect(&addr).await?;
        stream.write_all(&[0x05, 0x01, 0x00]).await?;
        let mut buf = [0u8; 2];
        stream.read_exact(&mut buf).await?;
        Ok::<_, std::io::Error>(buf == [0x05, 0x00])
    })
    .await;
    matches!(res, Ok(Ok(true)))
}

/// HTTP CONNECT liveness — via proxy's HTTP port.
/// Sends `CONNECT {connect_host}:{connect_port} HTTP/1.1` and checks `200`.
pub async fn http_is_alive(
    host: &str,
    port: u16,
    connect_host: &str,
    connect_port: u16,
    timeout_dur: Duration,
) -> bool {
    let addr = format!("{host}:{port}");
    let res = timeout(timeout_dur, async {
        let mut stream = TcpStream::connect(&addr).await?;
        let req = format!(
            "CONNECT {connect_host}:{connect_port} HTTP/1.1\r\nHost: {connect_host}:{connect_port}\r\n\r\n"
        );
        stream.write_all(req.as_bytes()).await?;
        let mut buf = vec![0u8; 1024];
        let n = stream.read(&mut buf).await?;
        let resp = &buf[..n];
        let status_line = resp.split(|&b| b == b'\r').next().unwrap_or(b"");
        Ok::<_, std::io::Error>(status_line.windows(5).any(|w| w == b" 200 "))
    })
    .await;
    matches!(res, Ok(Ok(true)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn socks5_probe_ok() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (mut s, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 3];
            let _ = s.read_exact(&mut buf).await;
            assert_eq!(buf, [0x05, 0x01, 0x00]);
            let _ = s.write_all(&[0x05, 0x00]).await;
        });
        assert!(socks5_is_alive("127.0.0.1", port, Duration::from_secs(2)).await);
    }

    #[tokio::test]
    async fn socks5_probe_fail() {
        // No listener on 1 — should fail
        assert!(!socks5_is_alive("127.0.0.1", 1, Duration::from_millis(200)).await);
    }

    #[tokio::test]
    async fn socks5_probe_wrong_response() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (mut s, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 3];
            let _ = s.read_exact(&mut buf).await;
            let _ = s.write_all(&[0x05, 0xFF]).await; // reject
        });
        assert!(!socks5_is_alive("127.0.0.1", port, Duration::from_secs(2)).await);
    }

    #[tokio::test]
    async fn http_probe_ok() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (mut s, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 1024];
            let n = s.read(&mut buf).await.unwrap();
            let req = String::from_utf8_lossy(&buf[..n]);
            assert!(req.starts_with("CONNECT"));
            let _ = s.write_all(b"HTTP/1.1 200 Connection established\r\n\r\n").await;
        });
        assert!(http_is_alive("127.0.0.1", port, "example.com", 443, Duration::from_secs(2)).await);
    }

    #[tokio::test]
    async fn http_probe_fail() {
        assert!(!http_is_alive("127.0.0.1", 1, "example.com", 443, Duration::from_millis(200)).await);
    }

    #[tokio::test]
    async fn http_probe_non_200() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (mut s, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 1024];
            let _ = s.read(&mut buf).await;
            let _ = s.write_all(b"HTTP/1.1 502 Bad Gateway\r\n\r\n").await;
        });
        assert!(!http_is_alive("127.0.0.1", port, "example.com", 443, Duration::from_secs(2)).await);
    }
}
