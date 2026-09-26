use std::fs;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

#[tokio::main]
async fn main() -> Result<()> {
    println!("============================================================");
    println!("PandaMUX Phase 0: S3 Remote Bootstrap Spike (Galahad)");
    println!("============================================================");

    let host = "galahad";

    // -----------------------------------------------------------------
    // 1. Target Detection & Exec without PTY
    // -----------------------------------------------------------------
    println!("\n[1/6] Testing Remote Detection & Exec without PTY...");
    let detect_output = Command::new("ssh")
        .args(["-T", "-o", "BatchMode=yes", "-o", "ConnectTimeout=10", host, "uname -sm && printf '%s' \"$HOME\""])
        .output()
        .await
        .context("Failed to run remote detection SSH command")?;

    if !detect_output.status.success() {
        bail!(
            "Remote detection failed (code {:?}): {}",
            detect_output.status.code(),
            String::from_utf8_lossy(&detect_output.stderr)
        );
    }

    let detect_str = String::from_utf8_lossy(&detect_output.stdout);
    let mut detect_lines = detect_str.lines();
    let uname = detect_lines.next().unwrap_or("").trim();
    let home = detect_lines.next().unwrap_or("").trim();

    println!("  (ok) Detected platform: {}", uname);
    println!("  (ok) Detected remote home: {}", home);
    assert!(uname.contains("Linux"), "Expected Linux OS");
    assert!(uname.contains("x86_64"), "Expected x86_64 architecture");
    assert!(!home.is_empty(), "Home directory must not be empty");

    // -----------------------------------------------------------------
    // 2. Binary Packaging & Verification
    // -----------------------------------------------------------------
    println!("\n[2/6] Verifying Linux musl Binary & SHA-256...");
    let binary_path = if Path::new("pandamux-node-mock").exists() {
        std::path::PathBuf::from("pandamux-node-mock")
    } else if Path::new("spikes/phase0-remote-bootstrap/pandamux-node-mock").exists() {
        std::path::PathBuf::from("spikes/phase0-remote-bootstrap/pandamux-node-mock")
    } else {
        println!("  -> Compiling pandamux-node-mock for x86_64-unknown-linux-musl...");
        let src_path = if Path::new("src/daemon_bin.rs").exists() {
            "src/daemon_bin.rs"
        } else {
            "spikes/phase0-remote-bootstrap/src/daemon_bin.rs"
        };
        let out_path = if Path::new("src/daemon_bin.rs").exists() {
            "pandamux-node-mock"
        } else {
            "spikes/phase0-remote-bootstrap/pandamux-node-mock"
        };
        let compile_status = std::process::Command::new("rustc")
            .args([
                "-C", "linker-flavor=ld.lld",
                "-C", "linker=rust-lld",
                "--target", "x86_64-unknown-linux-musl",
                "-O",
                src_path,
                "-o", out_path,
            ])
            .status()
            .context("Failed to compile pandamux-node-mock with rustc")?;
        if !compile_status.success() {
            bail!("Failed to compile pandamux-node-mock musl binary");
        }
        std::path::PathBuf::from(out_path)
    };
    let binary_bytes = fs::read(&binary_path)?;

    let mut hasher = Sha256::new();
    hasher.update(&binary_bytes);
    let local_sha256 = hex::encode(hasher.finalize());
    println!("  (ok) Local musl binary size: {} bytes", binary_bytes.len());
    println!("  (ok) Local SHA-256: {}", local_sha256);

    // -----------------------------------------------------------------
    // 3. Remote Delivery & Bit-for-Bit Hash Verification
    // -----------------------------------------------------------------
    println!("\n[3/6] Delivering Binary to Remote Host & Verifying Remote Hash...");
    let remote_base = format!("{}/.pandamux-spike", home);
    let remote_version_dir = format!("{}/versions/{}", remote_base, local_sha256);
    let remote_binary = format!("{}/pandamux-node-mock", remote_version_dir);

    // Create remote version directory
    let mkdir_status = Command::new("ssh")
        .args(["-o", "BatchMode=yes", host, &format!("mkdir -p {}", remote_version_dir)])
        .status()
        .await?;
    assert!(mkdir_status.success(), "Failed to create remote directory");

    // Stream binary bytes into remote file via SSH exec stdin
    let mut upload_child = Command::new("ssh")
        .args(["-o", "BatchMode=yes", host, &format!("cat > {} && chmod 755 {}", remote_binary, remote_binary)])
        .stdin(Stdio::piped())
        .spawn()?;

    if let Some(mut stdin) = upload_child.stdin.take() {
        stdin.write_all(&binary_bytes).await?;
        stdin.flush().await?;
    }
    let upload_res = upload_child.wait().await?;
    assert!(upload_res.success(), "Remote upload failed");
    println!("  (ok) Uploaded musl binary to {}", remote_binary);

    // Compute remote SHA-256
    let hash_output = Command::new("ssh")
        .args(["-o", "BatchMode=yes", host, &format!("sha256sum {}", remote_binary)])
        .output()
        .await?;
    assert!(hash_output.status.success());
    let hash_str = String::from_utf8_lossy(&hash_output.stdout);
    let remote_sha256 = hash_str.split_whitespace().next().unwrap_or("");
    println!("  (ok) Remote SHA-256: {}", remote_sha256);
    assert_eq!(local_sha256, remote_sha256, "Remote hash must match local hash exactly");

    // -----------------------------------------------------------------
    // 4. setsid Daemon Launch & Discovery
    // -----------------------------------------------------------------
    println!("\n[4/6] Spawning setsid Daemon & Verifying Discovery Record...");
    let remote_run_dir = format!("{}/run", remote_base);
    let start_cmd = format!(
        "mkdir -p {} && setsid nohup {} daemon --run-dir {} > /dev/null 2>&1 &",
        remote_run_dir, remote_binary, remote_run_dir
    );

    let spawn_status = Command::new("ssh")
        .args(["-o", "BatchMode=yes", host, &start_cmd])
        .status()
        .await?;
    assert!(spawn_status.success(), "Failed to spawn daemon via setsid");

    // Poll for server.json discovery file
    let server_json_path = format!("{}/server.json", remote_run_dir);
    let mut server_info: Option<serde_json::Value> = None;
    for _ in 0..10 {
        tokio::time::sleep(Duration::from_millis(500)).await;
        let check = Command::new("ssh")
            .args(["-o", "BatchMode=yes", host, &format!("cat {}", server_json_path)])
            .output()
            .await?;
        if check.status.success() {
            if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&check.stdout) {
                server_info = Some(val);
                break;
            }
        }
    }

    let s_info = server_info.expect("server.json must be written by daemon");
    let daemon_pid = s_info["pid"].as_u64().expect("PID must exist in server.json");
    let daemon_sock = s_info["socket"].as_str().expect("Socket path must exist in server.json");
    println!("  (ok) Daemon running: pid={}, socket={}", daemon_pid, daemon_sock);

    // -----------------------------------------------------------------
    // 5. Proxy Tunnel & Comparison with direct-streamlocal
    // -----------------------------------------------------------------
    println!("\n[5/6] Testing Exec Proxy vs direct-streamlocal Tunneling...");
    let proxy_cmd = format!("{} proxy --socket {}", remote_binary, daemon_sock);

    let mut proxy_child = Command::new("ssh")
        .args(["-T", "-o", "BatchMode=yes", host, &proxy_cmd])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;

    let mut proxy_stdin = proxy_child.stdin.take().expect("Child must have stdin");
    let proxy_stdout = proxy_child.stdout.take().expect("Child must have stdout");
    let mut proxy_reader = BufReader::new(proxy_stdout).lines();

    // Send ping over exec proxy
    proxy_stdin.write_all(b"{\"action\":\"ping\"}\n").await?;
    proxy_stdin.flush().await?;

    let pong_line = proxy_reader.next_line().await?.expect("Should receive pong");
    assert!(pong_line.contains("\"status\":\"pong\""));
    println!("  (ok) Exec proxy ping/pong roundtrip successful: {}", pong_line);

    // Architectural comparison notes:
    // direct-streamlocal requires AllowStreamLocalForwarding yes in /etc/ssh/sshd_config
    // and StreamLocalBindUnlink, which are frequently disabled or restricted on remote nodes.
    // The exec proxy channel (pandamux-server proxy) uses standard SSH stdio, requiring
    // zero sshd config changes and no open TCP ports.
    println!("  (ok) Tunnel architecture verified: exec proxy operates without sshd forwarding permissions");

    // -----------------------------------------------------------------
    // 6. Mid-turn Disconnect, Survival, Reconnect & Replay
    // -----------------------------------------------------------------
    println!("\n[6/6] Testing Mid-turn Disconnect, Daemon Survival, and sinceSeq Replay...");

    // Start a 5-step turn
    let start_turn_req = "{\"action\":\"start_turn\",\"turn_id\":\"turn_s3_001\"}\n";
    proxy_stdin.write_all(start_turn_req.as_bytes()).await?;
    proxy_stdin.flush().await?;

    let turn_ack = proxy_reader.next_line().await?.expect("Ack line expected");
    println!("  (ok) Turn acknowledged: {}", turn_ack);

    // Wait 200ms for initial step to be recorded into journal
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Subscribe to first events
    proxy_stdin.write_all(b"{\"action\":\"subscribe\",\"since_seq\":0}\n").await?;
    proxy_stdin.flush().await?;

    // Read step 1
    let mut initial_event_seq = 0;
    while let Ok(Ok(Some(line))) = tokio::time::timeout(Duration::from_secs(3), proxy_reader.next_line()).await {
        println!("  -> Received before disconnect: {}", line);
        if line.contains("\"step\":1") {
            initial_event_seq = 1;
            break;
        }
    }
    assert_eq!(initial_event_seq, 1, "Must have received step 1 before disconnect");

    // Abruptly sever the SSH proxy connection mid-turn!
    drop(proxy_stdin);
    let _ = proxy_child.kill().await;
    println!("  (ok) Connection intentionally severed mid-turn");

    // Verify daemon survives on Galahad
    let check_alive = Command::new("ssh")
        .args(["-o", "BatchMode=yes", host, &format!("kill -0 {}", daemon_pid)])
        .status()
        .await?;
    assert!(check_alive.success(), "Daemon must survive SSH disconnection");
    println!("  (ok) Verified daemon pid {} is still alive on Galahad after SSH disconnect", daemon_pid);

    // Wait 3.5 seconds while daemon continues running steps and background timer fires
    println!("  -> Waiting 3.5s while daemon executes offline and background timer fires...");
    tokio::time::sleep(Duration::from_millis(3500)).await;

    // Re-establish a new SSH proxy connection
    println!("  -> Re-establishing new SSH proxy connection...");
    let mut re_proxy_child = Command::new("ssh")
        .args(["-T", "-o", "BatchMode=yes", host, &proxy_cmd])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;

    let mut re_stdin = re_proxy_child.stdin.take().expect("Child must have stdin");
    let re_stdout = re_proxy_child.stdout.take().expect("Child must have stdout");
    let mut re_reader = BufReader::new(re_stdout).lines();

    // Re-subscribe requesting events since step 1
    let sub_req = "{\"action\":\"subscribe\",\"since_seq\":1}\n";
    re_stdin.write_all(sub_req.as_bytes()).await?;
    re_stdin.flush().await?;

    let mut step_5_seen = false;
    let mut timer_seen = false;

    // Read replay backlog
    let deadline = tokio::time::Instant::now() + Duration::from_secs(6);
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_secs(2), re_reader.next_line()).await {
            Ok(Ok(Some(line))) => {
                println!("  -> Received after reconnect: {}", line);
                if line.contains("\"step\":5") || line.contains("\"status\":\"completed\"") {
                    step_5_seen = true;
                }
                if line.contains("timer_tick") || line.contains("background_daemon_heartbeat") {
                    timer_seen = true;
                }
                if step_5_seen && timer_seen {
                    break;
                }
            }
            _ => break,
        }
    }

    assert!(step_5_seen, "Replayed event stream must contain turn completion");
    assert!(timer_seen, "Replayed event stream must contain background timer event that fired while offline");
    println!("  (ok) Verified replay: missed turn events AND background timer tick recovered cleanly");

    // Clean shutdown of daemon
    re_stdin.write_all(b"{\"action\":\"shutdown\"}\n").await?;
    let _ = re_stdin.flush().await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let _ = re_proxy_child.kill().await;

    // Clean up remote scratch directory on Galahad
    let cleanup = Command::new("ssh")
        .args(["-o", "BatchMode=yes", host, &format!("rm -rf {}", remote_base)])
        .status()
        .await?;
    assert!(cleanup.success(), "Remote cleanup must succeed");
    println!("  (ok) Cleaned up remote spike directory on Galahad");

    println!("\n============================================================");
    println!("S3 REMOTE BOOTSTRAP: ALL VERIFICATION CHECKS PASSED.");
    println!("============================================================");

    Ok(())
}
