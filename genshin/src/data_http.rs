//! Bounded downloads for the hosted scanner and capture catalogs.
use std::time::Duration;

pub(crate) fn client() -> Result<reqwest::blocking::Client, reqwest::Error> {
    with_timeout(Duration::from_secs(30))
}

fn with_timeout(timeout: Duration) -> Result<reqwest::blocking::Client, reqwest::Error> {
    reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(timeout)
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

    #[test]
    fn stalled_headers_and_body_both_time_out() {
        for send_headers in [false, true] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let url = format!(
                "http://{}/good/data_cache.json",
                listener.local_addr().unwrap()
            );
            let server = thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                stream.read(&mut [0; 2048]).unwrap();
                if send_headers {
                    stream
                        .write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\n",
                        )
                        .unwrap();
                }
                thread::sleep(Duration::from_millis(500));
            });
            let error = with_timeout(Duration::from_millis(200))
                .unwrap()
                .get(url)
                .send()
                .and_then(|response| response.text())
                .unwrap_err();
            assert!(error.is_timeout(), "send_headers={send_headers}: {error}");
            server.join().unwrap();
        }
    }
}
