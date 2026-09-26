use std::collections::VecDeque;
use std::env;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug)]
struct JournalEvent {
    seq: u64,
    json: String,
}

struct NodeState {
    journal: VecDeque<JournalEvent>,
    next_seq: u64,
}

impl NodeState {
    fn new() -> Self {
        Self {
            journal: VecDeque::new(),
            next_seq: 1,
        }
    }

    fn record_event(&mut self, event_type: &str, details: &str) -> (u64, String) {
        let seq = self.next_seq;
        self.next_seq += 1;

        let json = format!(
            "{{\"seq\":{},\"type\":\"{}\",\"data\":{}}}",
            seq, event_type, details
        );

        self.journal.push_back(JournalEvent {
            seq,
            json: json.clone(),
        });

        // Keep maximum 1000 events in journal
        if self.journal.len() > 1000 {
            self.journal.pop_front();
        }

        (seq, json)
    }

    fn get_events_since(&self, since_seq: u64) -> Vec<String> {
        self.journal
            .iter()
            .filter(|e| e.seq > since_seq)
            .map(|e| e.json.clone())
            .collect()
    }
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: pandamux-node-mock [daemon|proxy] [options]");
        std::process::exit(1);
    }

    match args[1].as_str() {
        "daemon" => run_daemon(&args[2..]),
        "proxy" => run_proxy(&args[2..]),
        other => {
            eprintln!("Unknown mode: {}", other);
            std::process::exit(1);
        }
    }
}

fn run_proxy(args: &[String]) {
    let mut socket_path = PathBuf::from("/tmp/pandamux.sock");
    let mut i = 0;
    while i < args.len() {
        if (args[i] == "-s" || args[i] == "--socket") && i + 1 < args.len() {
            socket_path = PathBuf::from(&args[i + 1]);
            i += 2;
        } else {
            i += 1;
        }
    }

    let stream = match UnixStream::connect(&socket_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Failed to connect to daemon socket {:?}: {}", socket_path, e);
            std::process::exit(1);
        }
    };

    let mut stream_reader = match stream.try_clone() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Failed to clone stream: {}", e);
            std::process::exit(1);
        }
    };
    let mut stream_writer = stream;

    // Thread 1: stdin -> socket
    let stdin_thread = thread::spawn(move || {
        let stdin = std::io::stdin();
        let mut handle = stdin.lock();
        let mut buf = [0u8; 4096];
        while let Ok(n) = handle.read(&mut buf) {
            if n == 0 {
                break;
            }
            if stream_writer.write_all(&buf[..n]).is_err() {
                break;
            }
            let _ = stream_writer.flush();
        }
    });

    // Thread 2: socket -> stdout
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    let mut buf = [0u8; 4096];
    while let Ok(n) = stream_reader.read(&mut buf) {
        if n == 0 {
            break;
        }
        if handle.write_all(&buf[..n]).is_err() {
            break;
        }
        let _ = handle.flush();
    }

    let _ = stdin_thread.join();
}

fn run_daemon(args: &[String]) {
    let mut run_dir = PathBuf::from("/tmp/pandamux-spike-run");
    let mut i = 0;
    while i < args.len() {
        if (args[i] == "-r" || args[i] == "--run-dir") && i + 1 < args.len() {
            run_dir = PathBuf::from(&args[i + 1]);
            i += 2;
        } else {
            i += 1;
        }
    }

    if let Err(e) = fs::create_dir_all(&run_dir) {
        eprintln!("Failed to create run directory {:?}: {}", run_dir, e);
        std::process::exit(1);
    }

    let socket_path = run_dir.join("server.sock");
    let server_json_path = run_dir.join("server.json");

    // Remove stale socket if exists
    let _ = fs::remove_file(&socket_path);

    let listener = match UnixListener::bind(&socket_path) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("Failed to bind unix socket {:?}: {}", socket_path, e);
            std::process::exit(1);
        }
    };

    let pid = std::process::id();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let server_info = format!(
        "{{\"pid\":{},\"version\":\"0.1.0\",\"socket\":\"{}\",\"started_at\":{}}}\n",
        pid,
        socket_path.to_string_lossy(),
        now
    );

    if let Err(e) = fs::write(&server_json_path, server_info) {
        eprintln!("Failed to write server.json: {}", e);
        std::process::exit(1);
    }

    let state = Arc::new(Mutex::new(NodeState::new()));
    let running = Arc::new(AtomicBool::new(true));

    // Spawn background timer thread: records a background timer tick every 1.5 seconds
    let timer_state = Arc::clone(&state);
    let timer_running = Arc::clone(&running);
    let timer_count = Arc::new(AtomicU64::new(0));
    let tc_clone = Arc::clone(&timer_count);

    thread::spawn(move || {
        while timer_running.load(Ordering::SeqCst) {
            thread::sleep(Duration::from_millis(1500));
            if !timer_running.load(Ordering::SeqCst) {
                break;
            }
            let c = tc_clone.fetch_add(1, Ordering::SeqCst) + 1;
            let mut s = timer_state.lock().unwrap();
            let details = format!("{{\"tick\":{},\"note\":\"background_daemon_heartbeat\"}}", c);
            s.record_event("timer_tick", &details);
        }
    });

    println!("Daemon started on {:?}, pid {}", socket_path, pid);

    for stream in listener.incoming() {
        if !running.load(Ordering::SeqCst) {
            break;
        }

        match stream {
            Ok(stream) => {
                let state_clone = Arc::clone(&state);
                let running_clone = Arc::clone(&running);
                let sock_cleanup = socket_path.clone();
                let json_cleanup = server_json_path.clone();

                thread::spawn(move || {
                    handle_client(
                        stream,
                        state_clone,
                        running_clone,
                        sock_cleanup,
                        json_cleanup,
                    );
                });
            }
            Err(_) => {
                if !running.load(Ordering::SeqCst) {
                    break;
                }
            }
        }
    }
}

fn handle_client(
    mut stream: UnixStream,
    state: Arc<Mutex<NodeState>>,
    running: Arc<AtomicBool>,
    socket_path: PathBuf,
    server_json_path: PathBuf,
) {
    let reader_stream = match stream.try_clone() {
        Ok(s) => s,
        Err(_) => return,
    };
    let mut reader = BufReader::new(reader_stream);
    let mut line = String::new();

    while let Ok(n) = reader.read_line(&mut line) {
        if n == 0 {
            break;
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            line.clear();
            continue;
        }

        if trimmed.contains("\"action\":\"ping\"") || trimmed == "ping" {
            let resp = "{\"status\":\"pong\",\"version\":\"0.1.0\"}\n";
            let _ = stream.write_all(resp.as_bytes());
            let _ = stream.flush();
        } else if trimmed.contains("\"action\":\"start_turn\"") {
            // Parse turn parameters
            let turn_id = if let Some(pos) = trimmed.find("\"turn_id\":") {
                let sub = &trimmed[pos + 10..];
                let end = sub.find(',').or_else(|| sub.find('}')).unwrap_or(sub.len());
                sub[..end].trim().trim_matches('"').to_string()
            } else {
                "turn_default".to_string()
            };

            // Spawn background task executing turn steps
            let turn_state = Arc::clone(&state);
            let t_id = turn_id.clone();
            thread::spawn(move || {
                for step in 1..=5 {
                    {
                        let mut s = turn_state.lock().unwrap();
                        let payload = format!(
                            "{{\"turn_id\":\"{}\",\"step\":{},\"status\":\"running\"}}",
                            t_id, step
                        );
                        s.record_event("turn_progress", &payload);
                    }
                    thread::sleep(Duration::from_millis(800));
                }
                thread::sleep(Duration::from_millis(800));
                let mut s = turn_state.lock().unwrap();
                let completion = format!(
                    "{{\"turn_id\":\"{}\",\"status\":\"completed\",\"tokens\":1450}}",
                    t_id
                );
                s.record_event("turn_completed", &completion);
            });

            let resp = format!(
                "{{\"status\":\"started\",\"turn_id\":\"{}\"}}\n",
                turn_id
            );
            let _ = stream.write_all(resp.as_bytes());
            let _ = stream.flush();
        } else if trimmed.contains("\"action\":\"subscribe\"") {
            // Extract since_seq
            let since_seq: u64 = if let Some(pos) = trimmed.find("\"since_seq\":") {
                let sub = &trimmed[pos + 12..];
                let end = sub.find(',').or_else(|| sub.find('}')).unwrap_or(sub.len());
                sub[..end].trim().parse().unwrap_or(0)
            } else {
                0
            };

            let events = {
                let s = state.lock().unwrap();
                s.get_events_since(since_seq)
            };

            for ev in events {
                let _ = stream.write_all(ev.as_bytes());
                let _ = stream.write_all(b"\n");
            }
            let _ = stream.flush();
        } else if trimmed.contains("\"action\":\"shutdown\"") {
            let resp = "{\"status\":\"shutting_down\"}\n";
            let _ = stream.write_all(resp.as_bytes());
            let _ = stream.flush();

            running.store(false, Ordering::SeqCst);
            let _ = fs::remove_file(&socket_path);
            let _ = fs::remove_file(&server_json_path);
            std::process::exit(0);
        }

        line.clear();
    }
}
