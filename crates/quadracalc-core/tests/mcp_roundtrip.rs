#![cfg(unix)]
//! Black-box acceptance test for quadracalc's agent-control wiring
//! (`crate::agent`): a real MCP client, connected over a Unix-domain
//! socket, drives a headless [`CalcApp`] through `initialize` →
//! `tools/list` → `set_cell` (twice) → `run_command` (a formula) →
//! `get_range`, and checks both the engine's own state and the TUI
//! screen a keyboard-driven user would see, the same two-angle
//! assertion `quadraui-mcp`'s own `mcp_tui_roundtrip.rs` makes.
//!
//! Same three-thread shape as that test: this test's own thread plays
//! the "runner" role (servicing the control port, the headless stand-in
//! for a live runner's per-iteration `pump`), a background thread runs
//! [`quadraui_mcp::serve`], and a second background thread is the "MCP
//! client" — a from-scratch line-delimited JSON-RPC connection, no MCP
//! SDK, proving the wire format rather than trusting a client library.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use quadraui::Backend;
use quadraui::control::channel;
use quadraui::tui::testing::TuiDriver;
use quadraui_mcp::Transport;
use serde_json::{Value, json};

use quadracalc_core::CalcApp;

/// One JSON-RPC round trip over `stream`: writes `request` as a line,
/// reads one line back, parses it as JSON.
fn call(stream: &mut UnixStream, reader: &mut BufReader<UnixStream>, request: &Value) -> Value {
    let mut line = serde_json::to_string(request).expect("request serializes");
    line.push('\n');
    stream.write_all(line.as_bytes()).expect("write request");
    stream.flush().expect("flush request");

    let mut response_line = String::new();
    reader.read_line(&mut response_line).expect("read response");
    assert!(
        !response_line.is_empty(),
        "server closed the connection before answering {request}"
    );
    serde_json::from_str(&response_line)
        .unwrap_or_else(|e| panic!("response {response_line:?} is not valid JSON: {e}"))
}

#[test]
fn mcp_client_builds_a_summary_through_set_cell_run_command_and_get_range() {
    let mut driver = TuiDriver::new(CalcApp::new(), 40, 10);

    let waker = driver.backend().waker();
    let (handle, port) = channel(waker);

    // `sockaddr_un::sun_path` is capped at 104 bytes on macOS (`SUN_LEN`,
    // smaller still on some other unixes), and `std::env::temp_dir()` is
    // `$TMPDIR` on macOS, e.g. `/var/folders/sr/<hash>/T/`, which alone can
    // run ~50 bytes — long enough that this test's own file name pushes it
    // over the limit. Prefer the short, fixed `/tmp` whenever the file name
    // would still fit under it; only fall back to `temp_dir()` (and let the
    // connect loop below report the `SUN_LEN` error) if `/tmp` itself isn't
    // short enough, which isn't expected on any unix this test runs on.
    let file_name = format!(
        "qc-mcp-{}-{}.sock",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    const SUN_LEN_BUDGET: usize = 100;
    let short_dir = std::path::Path::new("/tmp");
    let socket_path = if short_dir.join(&file_name).as_os_str().len() <= SUN_LEN_BUDGET {
        short_dir.join(&file_name)
    } else {
        std::env::temp_dir().join(&file_name)
    };
    let _ = std::fs::remove_file(&socket_path);

    let server_socket_path = socket_path.clone();
    let server =
        thread::spawn(move || quadraui_mcp::serve(handle, Transport::Unix(server_socket_path)));

    // `TuiDriver` is not `Send` (it owns an `Rc`-based ratatui terminal),
    // so it stays on this test's own thread, which plays the runner's
    // role: service the control port (and tick, to pick up the deferred
    // view refresh — see `CalcApp::tick`'s own comment) in a loop until
    // the client thread reports it's done.
    let client_done = Arc::new(AtomicBool::new(false));
    let worker_done = client_done.clone();
    let socket_path_for_client = socket_path.clone();
    let client = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut stream = loop {
            match UnixStream::connect(&socket_path_for_client) {
                Ok(stream) => break stream,
                Err(e) => {
                    assert!(
                        Instant::now() < deadline,
                        "never connected to the MCP socket: {e}"
                    );
                    thread::sleep(Duration::from_millis(1));
                }
            }
        };
        let mut reader = BufReader::new(stream.try_clone().expect("clone the connected stream"));

        let init = call(
            &mut stream,
            &mut reader,
            &json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {"protocolVersion": "2024-11-05"},
            }),
        );
        assert_eq!(init["result"]["serverInfo"]["name"], json!("quadraui-mcp"));

        let listed = call(
            &mut stream,
            &mut reader,
            &json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
        );
        let tool_names: Vec<&str> = listed["result"]["tools"]
            .as_array()
            .expect("tools array")
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert_eq!(
            tool_names,
            vec![
                "snapshot",
                "act",
                "key",
                "type",
                "screenshot",
                "set_cell",
                "get_range",
                "run_command",
            ],
            "the three Level 2 app commands should follow the five built-ins"
        );

        let set_a1 = call(
            &mut stream,
            &mut reader,
            &json!({
                "jsonrpc": "2.0",
                "id": 3,
                "method": "tools/call",
                "params": {"name": "set_cell", "arguments": {"cell": "A1", "input": "10"}},
            }),
        );
        assert_eq!(
            set_a1["result"]["isError"],
            json!(false),
            "set_cell A1 failed: {set_a1:?}"
        );

        let set_a2 = call(
            &mut stream,
            &mut reader,
            &json!({
                "jsonrpc": "2.0",
                "id": 4,
                "method": "tools/call",
                "params": {"name": "set_cell", "arguments": {"cell": "A2", "input": "20"}},
            }),
        );
        assert_eq!(
            set_a2["result"]["isError"],
            json!(false),
            "set_cell A2 failed: {set_a2:?}"
        );

        let run_formula = call(
            &mut stream,
            &mut reader,
            &json!({
                "jsonrpc": "2.0",
                "id": 5,
                "method": "tools/call",
                "params": {
                    "name": "run_command",
                    "arguments": {"id": "cell.set", "params": {"cell": "A3", "input": "=A1+A2"}},
                },
            }),
        );
        assert_eq!(
            run_formula["result"]["isError"],
            json!(false),
            "run_command cell.set A3 failed: {run_formula:?}"
        );

        let range = call(
            &mut stream,
            &mut reader,
            &json!({
                "jsonrpc": "2.0",
                "id": 6,
                "method": "tools/call",
                "params": {"name": "get_range", "arguments": {"range": "A1:A3"}},
            }),
        );
        assert_eq!(
            range["result"]["isError"],
            json!(false),
            "get_range A1:A3 failed: {range:?}"
        );
        let range_value: Value =
            serde_json::from_str(range["result"]["content"][0]["text"].as_str().unwrap())
                .expect("get_range content is JSON");
        assert_eq!(range_value["values"], json!([[10.0], [20.0], [30.0]]));
        assert_eq!(
            range_value["formulas"],
            json!([[10.0], [20.0], ["=A1+A2"]]),
            "only A3 should come back as a formula string"
        );

        worker_done.store(true, Ordering::SeqCst);
    });

    let deadline = Instant::now() + Duration::from_secs(10);
    while !client_done.load(Ordering::SeqCst) {
        driver.service_control(&port);
        driver.tick();
        assert!(Instant::now() < deadline, "MCP client never finished");
        thread::sleep(Duration::from_millis(1));
    }
    client.join().expect("client thread panicked");

    // The engine's own state really changed, not just the MCP JSON.
    assert_eq!(driver.app_mut().cell_text("A3"), "30");

    // ...and it painted on the live screen the same way a keyboard edit
    // would have, through `CalcApp`'s own `View`/`CellGrid` refresh path.
    assert!(driver.screen_contains("30"), "{}", driver.screen());

    let _ = std::fs::remove_file(&socket_path);
    // `server` is an infinite accept loop with no shutdown signal — its
    // thread leaks for the rest of this test binary's process, same as
    // `quadraui-mcp`'s own round-trip test.
    drop(server);
}
