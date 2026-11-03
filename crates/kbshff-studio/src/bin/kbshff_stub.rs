//! Stand-in `kbshff` for studio tests. Prints canned JSON and sleeps on `run`.

use std::env;
use std::io::{self, Read};
use std::thread;
use std::time::Duration;

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let joined = args.join(" ");
    if args.first().is_some_and(|arg| arg == "run") {
        eprintln!(r#"{{"type":"message","text":"stub"}}"#);
        thread::sleep(Duration::from_secs(30));
        println!(
            r#"{{"run_id":"stub","result":"ok","stop_reason":"goal_reached","usage":{{}},"logs":{{}}}}"#
        );
        return;
    }
    if joined.contains("agent list")
        || (args.first().is_some_and(|arg| arg == "agent")
            && joined.contains("list")
            && !joined.contains("out"))
    {
        println!(r#"{{"agents":["demo"]}}"#);
        return;
    }
    if joined.contains("document") && joined.contains("get") {
        println!(
            r#"{{"ok":true,"resource":"document","action":"get","data":{{"config":{{"provider":{{"model":"m"}}}},"api_key_env_set":false,"api_key_env":"OPENAI_API_KEY"}}}}"#
        );
        return;
    }
    if joined.contains("skill") && joined.contains("body") && joined.contains("get") {
        if joined.contains("missing") {
            eprintln!("missing skill body");
            std::process::exit(2);
        }
        println!(
            r#"{{"ok":true,"resource":"skill.body","action":"get","data":{{"content":"be careful"}}}}"#
        );
        return;
    }
    if joined.contains("skill")
        && joined.contains("body")
        && (joined.contains("set") || joined.contains("clear"))
    {
        if joined.contains("set") && joined.contains("bad") {
            eprintln!("bad skill body");
            std::process::exit(2);
        }
        println!(r#"{{"ok":true,"resource":"skill.body","action":"set","data":{{"ok":true}}}}"#);
        return;
    }
    if args.first().is_some_and(|arg| arg == "init") {
        match args.get(1).map(String::as_str) {
            Some("bad") => {
                eprintln!("init failed");
                std::process::exit(2);
            }
            Some("bad-stdout") => {
                println!("plain failure");
                std::process::exit(2);
            }
            Some("bad-empty") => std::process::exit(2),
            _ => {
                println!("initialized");
                return;
            }
        }
    }
    if args.iter().any(|arg| arg == "import") {
        println!(r#"{{"ok":true,"data":{{"ok":true}}}}"#);
        return;
    }
    if joined.contains("document") && joined.contains("set") {
        println!(r#"{{"ok":true,"data":{{"config":{{}}}}}}"#);
        return;
    }
    if joined.contains("prompt") && joined.contains("get") {
        println!(r#"{{"ok":true,"data":{{"master_prompt":"sys"}}}}"#);
        return;
    }
    if joined.contains("rules") && joined.contains("get") {
        println!(r#"{{"ok":true,"data":{{"rules":"be brief"}}}}"#);
        return;
    }
    if (joined.contains("prompt") || joined.contains("rules")) && joined.contains("set") {
        println!(r#"{{"ok":true}}"#);
        return;
    }
    if joined.contains("skill") && joined.contains("list") {
        println!(r#"{{"ok":true,"data":{{"skills":[]}}}}"#);
        return;
    }
    if joined.contains("skill") && joined.contains("replace") {
        println!(r#"{{"ok":true,"data":{{"ok":true}}}}"#);
        return;
    }
    if joined.contains("secret") {
        println!(r#"{{"ok":true,"data":{{"ok":true}}}}"#);
        return;
    }
    if joined.contains("mcp") && joined.contains("discover") {
        println!(r#"{{"servers":[]}}"#);
        return;
    }
    if joined.contains("mcp") && joined.contains("call") {
        println!(r#"{{"ok":true}}"#);
        return;
    }
    if joined.contains("out") && joined.contains("show") {
        println!(r#"{{"content":"file"}}"#);
        return;
    }
    if joined.contains("out") && joined.contains("list") {
        println!(r#"{{"files":[]}}"#);
        return;
    }
    if joined.contains("check") {
        println!(
            r#"{{"config_path":"c","items":[{{"name":"config","status":"ok","detail":"ok"}}],"inventory":{{"builtins_allowed":[],"mcp_connected":[],"mcp_failed":[],"mcp_gated":[],"tools_available":[]}}}}"#
        );
        return;
    }
    let mut stdin = String::new();
    let _ = io::stdin().read_to_string(&mut stdin);
    eprintln!("stub unhandled: {joined}");
    std::process::exit(2);
}
