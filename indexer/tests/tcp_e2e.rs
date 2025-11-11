use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

// External endpoint E2E test. Ignored by default to avoid flakiness in CI.
#[tokio::test]
#[ignore]
async fn tcp_subscription_e2e_ping_and_subscribe() -> Result<(), Box<dyn std::error::Error>> {
    let addr = "54.224.0.187:8080";

    // Connect
    let stream = tokio::time::timeout(Duration::from_secs(5), TcpStream::connect(addr)).await??;
    let (read_half, mut write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);

    // Send PING
    write_half.write_all(b"PING\n").await?;
    write_half.flush().await?;

    // Expect PONG
    let mut line = String::new();
    tokio::time::timeout(Duration::from_secs(5), reader.read_line(&mut line)).await??;
    assert_eq!(line, "PONG\n", "expected PONG from server, got {:?}", line);
    line.clear();

    // Send a valid subscription request
    write_half
        .write_all(b"{\"subscribe\":[\"NewBlock\"]}\n")
        .await?;
    write_half.flush().await?;

    // Try to read one line within 5s:
    // - If we get a JSON line, great.
    // - If we time out, still OK (no new events yet) as long as the connection didn't close.
    // - If we get 0 bytes (EOF) quickly, treat as failure.
    match tokio::time::timeout(Duration::from_secs(5), reader.read_line(&mut line)).await {
        Ok(Ok(0)) => panic!("connection closed by server unexpectedly"),
        Ok(Ok(_n)) => {
            // We received something; it might be an event or a banner.
            // Just assert it's non-empty and valid UTF-8 (already guaranteed by read_line).
            assert!(!line.trim().is_empty(), "received an empty line");
        }
        Ok(Err(e)) => panic!("read error: {:?}", e),
        Err(_elapsed) => {
            // Timeout waiting for an event; acceptable.
        }
    }

    Ok(())
}
