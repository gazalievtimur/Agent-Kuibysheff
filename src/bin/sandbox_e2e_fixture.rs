//! Minimal executable used by HomeFs / agent E2E sandbox tests.

fn main() {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("echo") => {
            let msg = args.next().unwrap_or_default();
            println!("{msg}");
        }
        Some("write") => {
            let path = args.next().expect("path");
            let body = args.next().unwrap_or_else(|| "x".to_string());
            if let Err(err) = std::fs::write(&path, body) {
                eprintln!("write failed: {err}");
                std::process::exit(2);
            }
            println!("wrote");
        }
        other => {
            eprintln!("unknown mode: {other:?}");
            std::process::exit(1);
        }
    }
}
