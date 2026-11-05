//! Standalone headless URL scan CLI (no Tauri desktop). Uses the same DB as the app.
//!
//! Examples:
//!   oviraptor-headless --project-id 1 --url https://example.com/
//!   oviraptor-headless --db ~/oviraptor/oviraptor.sqlite3 --project-id 1 --url https://t/ --wait
//!   oviraptor-headless --project-id 1 --url https://t/ --wait --max-wait-secs 1800 --json

use oviraptor_lib::{default_database_path, default_oviraptor_data_dir, run_url_scan_cli};
use std::{env, io::Write, path::PathBuf, process};

fn usage() -> ! {
    eprintln!(
        "Usage: oviraptor-headless --project-id ID --url URL [--task-name NAME] [--scan-mode standard|quick|deep] \\
  [--wait] [--max-wait-secs N] [--db PATH] [--data-dir PATH] [--json]"
    );
    process::exit(2);
}

fn main() {
    let mut args = env::args().skip(1);
    let mut project_id: Option<i64> = None;
    let mut url: Option<String> = None;
    let mut task_name: Option<String> = None;
    let mut scan_mode: Option<String> = None;
    let mut wait = false;
    let mut max_wait_secs: Option<u32> = None;
    let mut db_path: Option<PathBuf> = None;
    let mut data_dir: Option<PathBuf> = None;
    let mut json_out = false;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--project-id" => {
                project_id = Some(
                    args.next()
                        .unwrap_or_else(|| usage())
                        .parse()
                        .unwrap_or_else(|_| usage()),
                );
            }
            "--url" => url = Some(args.next().unwrap_or_else(|| usage())),
            "--task-name" => task_name = Some(args.next().unwrap_or_else(|| usage())),
            "--scan-mode" => scan_mode = Some(args.next().unwrap_or_else(|| usage())),
            "--wait" => wait = true,
            "--max-wait-secs" => {
                max_wait_secs = Some(
                    args.next()
                        .unwrap_or_else(|| usage())
                        .parse()
                        .unwrap_or_else(|_| usage()),
                );
            }
            "--db" => db_path = Some(PathBuf::from(args.next().unwrap_or_else(|| usage()))),
            "--data-dir" => data_dir = Some(PathBuf::from(args.next().unwrap_or_else(|| usage()))),
            "--json" => json_out = true,
            "--help" | "-h" => usage(),
            other => {
                eprintln!("Unknown argument: {other}");
                usage();
            }
        }
    }

    let project_id = project_id.unwrap_or_else(|| usage());
    let url = url.unwrap_or_else(|| usage());
    let db_path = match db_path {
        Some(path) => path,
        None => default_database_path().unwrap_or_else(|error| {
            eprintln!("{error}");
            process::exit(1);
        }),
    };
    let data_dir = match data_dir {
        Some(path) => path,
        None => default_oviraptor_data_dir().unwrap_or_else(|error| {
            eprintln!("{error}");
            process::exit(1);
        }),
    };
    let task_name = task_name.unwrap_or_else(|| url.clone());

    let result = run_url_scan_cli(
        db_path,
        data_dir,
        project_id,
        task_name,
        vec![url],
        scan_mode,
        wait,
        max_wait_secs,
    );

    match result {
        Ok(value) => {
            if json_out {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string())
                );
            } else {
                let snapshot = value.get("snapshot").cloned().unwrap_or(value.clone());
                let status = snapshot
                    .get("status")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                let findings = snapshot
                    .get("findingCount")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0);
                let scan_id = snapshot
                    .get("scanId")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let _ = writeln!(
                    std::io::stdout(),
                    "scan_id={scan_id} status={status} findings={findings}"
                );
                if value.get("timedOut").and_then(|v| v.as_bool()) == Some(true) {
                    process::exit(3);
                }
            }
        }
        Err(error) => {
            eprintln!("{error}");
            process::exit(1);
        }
    }
}
