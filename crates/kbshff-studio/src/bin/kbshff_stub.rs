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
        println!(
            r#"{{"ok":true,"resource":"skill.body","action":"get","data":{{"content":"be careful"}}}}"#
        );
        return;
    }
    if joined.contains("skill")
        && joined.contains("body")
        && (joined.contains("set") || joined.contains("clear"))
    {
        println!(r#"{{"ok":true,"resource":"skill.body","action":"set","data":{{"ok":true}}}}"#);
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
