use good_tools_app::hsr_server::{allowed_origin, serve, JobState};
use serde_json::{json, Value};
use std::{
    io::{Read, Write},
    net::TcpStream,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use yas::cancel::{CancelToken, StopReason};

fn request(address: &str, method: &str, path: &str, body: &str, origin: &str) -> (u16, Value) {
    let mut stream = TcpStream::connect(address).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    write!(stream, "{method} {path} HTTP/1.1\r\nHost: {address}\r\nOrigin: {origin}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
    let mut reply = String::new();
    stream.read_to_string(&mut reply).unwrap();
    let (head, body) = reply.split_once("\r\n\r\n").unwrap();
    let code = head.split_whitespace().nth(1).unwrap().parse().unwrap();
    (code, serde_json::from_str(body).unwrap_or(Value::Null))
}

#[test]
fn loopback_jobs_keep_status_responsive_cancel_safely_and_deduplicate_results() {
    let server = tiny_http::Server::http(("127.0.0.1", 0)).unwrap();
    let address = server.server_addr().to_string();
    let cancel = CancelToken::new();
    let stop = cancel.clone();
    let calls = Arc::new(AtomicUsize::new(0));
    let runs = calls.clone();
    let thread = std::thread::spawn(move || {
        serve(
            server,
            stop,
            Arc::new(Mutex::new(JobState::default())),
            move |_, job_cancel| {
                let run = runs.fetch_add(1, Ordering::SeqCst);
                if run == 0 {
                    let deadline = Instant::now() + Duration::from_secs(4);
                    while !job_cancel.is_cancelled() && Instant::now() < deadline {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(json!({"zh":"已停止", "en":"Stopped", "details":"cancel retained journal"}))
                } else {
                    Ok(json!({"kind":"manage", "verified":1}))
                }
            },
            || json!({"steps":[]}),
        )
        .unwrap()
    });
    let origin = "https://hsr.ggartifact.com";
    let body = include_str!("../../experimental/hsr/tests/fixtures/manager_instructions_v1.json");
    assert_eq!(
        request(
            &address,
            "POST",
            "/manage",
            body,
            "https://hsr.ggartifact.com.evil.test"
        )
        .0,
        403
    );
    assert_eq!(request(&address, "POST", "/manage", "{}", origin).0, 400);
    let (code, first) = request(&address, "POST", "/manage", body, origin);
    assert_eq!(code, 202);
    let id = first["jobId"].as_str().unwrap();
    assert_eq!(request(&address, "POST", "/manage", body, origin).1, first);
    assert_eq!(
        request(&address, "POST", "/scan", r#"{"relics":true}"#, origin).0,
        409
    );
    let status = request(&address, "GET", "/status", "", origin).1;
    assert_eq!(status["game"], "star-rail");
    assert_eq!(status["count"], 1);
    assert_eq!(
        request(&address, "GET", &format!("/result?jobId={id}"), "", origin).0,
        409
    );
    assert_eq!(
        request(&address, "POST", "/cancel?jobId=other", "{}", origin).0,
        404
    );
    assert_eq!(
        request(
            &address,
            "POST",
            &format!("/cancel?jobId={id}"),
            "{}",
            origin
        )
        .0,
        202
    );
    let deadline = Instant::now() + Duration::from_secs(4);
    loop {
        if request(&address, "GET", "/status", "", origin).1["phase"] == "failed" {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    let result = request(&address, "GET", &format!("/result?jobId={id}"), "", origin).1;
    assert_eq!(result["error"]["en"], "Stopped");
    assert_eq!(request(&address, "GET", "/health", "", origin).0, 200);
    // Explicit retry of a failed request is allowed. A completed retry isn't executed again.
    let second = request(&address, "POST", "/manage", body, origin).1;
    assert_ne!(first, second);
    while request(&address, "GET", "/status", "", origin).1["phase"] != "completed" {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(request(&address, "POST", "/manage", body, origin).1, second);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(
        request(&address, "GET", &format!("/result?jobId={id}"), "", origin).0,
        404
    );
    cancel.cancel(StopReason::UserAbort);
    thread.join().unwrap();
}

#[test]
fn origins_are_exact_and_game_specific() {
    for origin in [
        "https://hsr.ggartifact.com",
        "http://localhost:5173",
        "http://127.0.0.1:5173",
    ] {
        assert!(allowed_origin(origin));
    }
    for origin in [
        "https://ggartifact.com",
        "http://hsr.ggartifact.com",
        "https://evilggartifact.com",
        "https://hsr.ggartifact.com/path",
        "null",
        "https://user@hsr.ggartifact.com",
    ] {
        assert!(!allowed_origin(origin));
    }
}
