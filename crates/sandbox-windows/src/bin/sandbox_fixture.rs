//! Tiny fixture binary for AppContainer integration tests.

fn main() {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("echo") => {
            let msg = args.next().unwrap_or_default();
            println!("{msg}");
        }
        Some("sleep-ms") => {
            let ms: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(30_000);
            std::thread::sleep(std::time::Duration::from_millis(ms));
            println!("slept");
        }
        Some("write") => {
            let path = args.next().expect("write path");
            let body = args.next().unwrap_or_else(|| "x".to_string());
            match std::fs::write(&path, body) {
                Ok(()) => println!("wrote"),
                Err(err) => {
                    eprintln!("write failed: {err}");
                    std::process::exit(2);
                }
            }
        }
        Some("emit-bytes") => {
            let n: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(100);
            let chunk = "A".repeat(n.min(1_048_576));
            print!("{chunk}");
        }
        Some("connect-loopback") => {
            let port: u16 = args.next().and_then(|s| s.parse().ok()).unwrap_or(9);
            match std::net::TcpStream::connect_timeout(
                &std::net::SocketAddr::from(([127, 0, 0, 1], port)),
                std::time::Duration::from_millis(400),
            ) {
                Ok(_) => {
                    println!("connected");
                    std::process::exit(0);
                }
                Err(err) => {
                    eprintln!("connect failed: {err}");
                    std::process::exit(2);
                }
            }
        }
        other => {
            eprintln!("unknown mode: {other:?}");
            std::process::exit(1);
        }
    }
}
