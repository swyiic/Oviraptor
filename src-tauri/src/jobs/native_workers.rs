use base64::Engine;
use csv::{ReaderBuilder, WriterBuilder};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    fs,
    io::{BufRead, Read, Write},
    net::{IpAddr, TcpStream},
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

type Record = HashMap<String, String>;

#[path = "native_content.rs"]
mod native_content;
#[path = "native_probe_control.rs"]
mod native_probe_control;

fn read_csv(path: &Path) -> Result<(Vec<String>, Vec<Record>), String> {
    let mut reader = ReaderBuilder::new()
        .flexible(true)
        .from_path(path)
        .map_err(|error| error.to_string())?;
    let fields = reader
        .headers()
        .map_err(|error| error.to_string())?
        .iter()
        .enumerate()
        .map(|(index, value)| {
            if index == 0 {
                value.trim_start_matches('\u{feff}').to_string()
            } else {
                value.to_string()
            }
        })
        .collect::<Vec<_>>();
    let mut rows = Vec::new();
    for record in reader.records() {
        let record = record.map_err(|error| error.to_string())?;
        rows.push(
            fields
                .iter()
                .enumerate()
                .map(|(index, field)| {
                    (
                        field.clone(),
                        record.get(index).unwrap_or_default().trim().to_string(),
                    )
                })
                .collect(),
        );
    }
    Ok((fields, rows))
}
fn text(row: &Record, keys: &[&str]) -> String {
    keys.iter()
        .find_map(|k| {
            row.get(*k)
                .map(|v| v.trim())
                .filter(|v| !v.is_empty())
                .map(str::to_string)
        })
        .unwrap_or_default()
}
fn endpoint(row: &Record) -> String {
    discovery_identity(row)
}
fn number(row: &Record, keys: &[&str]) -> i64 {
    text(row, keys).parse().unwrap_or(0)
}
fn rank(row: &Record) -> (i32, i32, bool, bool) {
    let status = number(row, &["probe_status_code", "status_code"]);
    let outcome = text(row, &["probe_outcome"]);
    let url = text(row, &["probe_effective_url", "link", "host"]);
    let sr = if (200..300).contains(&status) {
        5
    } else if (300..400).contains(&status) {
        4
    } else if matches!(status, 401 | 403) {
        3
    } else if (400..500).contains(&status) {
        2
    } else {
        0
    };
    let or = match outcome.as_str() {
        "web_alive" => 6,
        "virtual_host_required" => 5,
        "web_restricted" => 4,
        "browser_render_required" => 3,
        "tcp_alive_non_http" => 2,
        "alive_clean" => 1,
        _ => 0,
    };
    (
        or,
        sr,
        url.starts_with("https://"),
        !text(row, &["probe_title", "title"]).is_empty(),
    )
}
fn reason(row: &Record) -> String {
    let status = number(row, &["probe_status_code", "status_code"]);
    match text(row, &["probe_outcome"]).as_str() {
        "web_abnormal" => {
            let v = text(row, &["probe_entry_state"]);
            format!(
                "web_abnormal:{}",
                if v.is_empty() {
                    format!("HTTP_{status}")
                } else {
                    v
                }
            )
        }
        "unreachable" => {
            let v = text(row, &["probe_entry_state"]);
            format!(
                "unreachable:{}",
                if v.is_empty() {
                    "connect_failed".into()
                } else {
                    v
                }
            )
        }
        "skipped" => {
            let v = text(row, &["probe_error"]);
            format!(
                "skipped:{}",
                if v.is_empty() {
                    "invalid_target".into()
                } else {
                    v
                }
            )
        }
        _ => String::new(),
    }
}
fn write_csv(path: &Path, fields: &[String], rows: &[Record]) -> Result<(), String> {
    let mut w = WriterBuilder::new()
        .from_path(path)
        .map_err(|e| e.to_string())?;
    w.write_record(fields).map_err(|e| e.to_string())?;
    for row in rows {
        w.write_record(
            fields
                .iter()
                .map(|f| row.get(f).map(String::as_str).unwrap_or("")),
        )
        .map_err(|e| e.to_string())?
    }
    w.flush().map_err(|e| e.to_string())
}

const ACTIVE_STATUSES: &[&str] = &[
    "200", "201", "202", "204", "206", "301", "302", "303", "307", "308", "401", "403",
];

include!("native_refinement.rs");

include!("native_discovery.rs");

include!("native_response_identity.rs");

include!("native_probe.rs");


include!("native_fofa.rs");


pub fn merge_url_inventory_with_storage(
    input: &Path,
    output: &Path,
    storage: &crate::jobs::collection_retention::CollectionStorage,
) -> Result<serde_json::Value, String> {
    merge_url_inventory_inner(input, output, Some(storage))
}

fn merge_url_inventory_inner(
    input: &Path,
    output: &Path,
    storage: Option<&crate::jobs::collection_retention::CollectionStorage>,
) -> Result<serde_json::Value, String> {
    let (_, rows) = read_csv(input)?;
    let mut best = HashMap::<String, Record>::new();
    for mut row in rows {
        let raw = text(&row, &["link", "host", "ip"]);
        if raw.is_empty() {
            continue;
        }
        let identity = discovery_identity(&row);
        let mut url = build_probe_url(&row).unwrap_or_else(|| {
            if raw.contains("://") {
                raw
            } else {
                format!("http://{raw}")
            }
        });
        if let Ok(mut parsed) = reqwest::Url::parse(&url) {
            parsed.set_fragment(None);
            url = parsed.to_string();
        }
        let decision = text(&row, &["decision", "人工结论", "结论"]).to_ascii_lowercase();
        let bucket = if [
            "confirmed",
            "confirm",
            "yes",
            "true",
            "1",
            "确认",
            "已确认",
            "是",
        ]
        .contains(&decision.as_str())
        {
            "confirmed"
        } else if ["rejected", "reject", "no", "false", "0", "排除", "否"]
            .contains(&decision.as_str())
        {
            "rejected"
        } else {
            "pending"
        };
        row.insert("normalized_url".into(), url.clone());
        row.insert("inventory_identity".into(), identity.clone());
        row.insert("inventory_bucket".into(), bucket.into());
        let replace = best
            .get(&identity)
            .map(|old| number(&row, &["score"]) > number(old, &["score"]))
            .unwrap_or(true);
        if replace {
            best.insert(identity, row);
        }
    }
    let mut merged = best.into_values().collect::<Vec<_>>();
    merged.sort_by(|left, right| {
        text(left, &["normalized_url"])
            .cmp(&text(right, &["normalized_url"]))
            .then_with(|| {
                text(left, &["inventory_identity"]).cmp(&text(right, &["inventory_identity"]))
            })
    });
    let mut fields = vec![
        "normalized_url".into(),
        "inventory_identity".into(),
        "inventory_bucket".into(),
    ];
    for row in &merged {
        for field in row.keys() {
            if !fields.contains(field) {
                fields.push(field.clone());
            }
        }
    }
    write_collection_csv(output, &fields, &merged, storage)?;
    let parent = output.parent().unwrap_or_else(|| Path::new("."));
    for (bucket, name) in [
        ("confirmed", "confirmed_urls.txt"),
        ("pending", "pending_urls.txt"),
        ("rejected", "rejected_urls.txt"),
    ] {
        let values = merged
            .iter()
            .filter(|row| text(row, &["inventory_bucket"]) == bucket)
            .map(|row| text(row, &["normalized_url"]))
            .collect::<Vec<_>>()
            .join("\n");
        let content = if values.is_empty() {
            values
        } else {
            format!("{values}\n")
        };
        if let Some(storage) = storage {
            storage.write(
                Path::new(name),
                content.as_bytes(),
                crate::jobs::collection_retention::ArtifactKind::Inventory,
                None,
                chrono::Utc::now().timestamp(),
            )?;
        } else {
            native_collection_resume::atomic_write(&parent.join(name), content.as_bytes())?;
        }
    }
    Ok(
        json!({"input": merged.len(), "unique": merged.len(), "confirmed": merged.iter().filter(|row| text(row, &["inventory_bucket"]) == "confirmed").count(), "pending": merged.iter().filter(|row| text(row, &["inventory_bucket"]) == "pending").count(), "rejected": merged.iter().filter(|row| text(row, &["inventory_bucket"]) == "rejected").count()}),
    )
}
pub fn optimize_src_assets(input: &Path, output: &Path) -> Result<serde_json::Value, String> {
    fs::create_dir_all(output).map_err(|e| e.to_string())?;
    let accepted = [
        "web_alive",
        "web_restricted",
        "browser_render_required",
        "virtual_host_required",
        "web_abnormal",
        "tcp_alive_non_http",
        "alive_clean",
        "blocked_content",
        "unreachable",
        "skipped",
    ];
    let mut paths = fs::read_dir(input)
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.extension().and_then(|v| v.to_str()) == Some("csv")
                && accepted.iter().any(|v| {
                    p.file_name()
                        .and_then(|v| v.to_str())
                        .unwrap_or("")
                        .contains(v)
                })
        })
        .collect::<Vec<_>>();
    paths.sort();
    let (mut rows, mut fields) = (Vec::<Record>::new(), Vec::<String>::new());
    for path in paths {
        let mut r = ReaderBuilder::new()
            .flexible(true)
            .from_path(path)
            .map_err(|e| e.to_string())?;
        let h = r
            .headers()
            .map_err(|e| e.to_string())?
            .iter()
            .map(str::to_string)
            .collect::<Vec<_>>();
        for v in &h {
            if !fields.contains(v) {
                fields.push(v.clone())
            }
        }
        for rec in r.records() {
            let rec = rec.map_err(|e| e.to_string())?;
            rows.push(
                h.iter()
                    .enumerate()
                    .map(|(i, k)| (k.clone(), rec.get(i).unwrap_or("").into()))
                    .collect(),
            )
        }
    }
    rows.sort_by(|left, right| {
        endpoint(left)
            .cmp(&endpoint(right))
            .then_with(|| rank(right).cmp(&rank(left)))
            .then_with(|| {
                let left = left.iter().collect::<BTreeMap<_, _>>();
                let right = right.iter().collect::<BTreeMap<_, _>>();
                serde_json::to_string(&left)
                    .expect("CSV records serialize")
                    .cmp(&serde_json::to_string(&right).expect("CSV records serialize"))
            })
    });
    // Stage/journal output can contain the same requested endpoint more than
    // once. Import its best current result once, retaining all source evidence;
    // a later duplicate must not overwrite a healthy result with quarantine.
    let mut best = BTreeMap::<String, usize>::new();
    for (i, row) in rows.iter().enumerate() {
        let key = endpoint(row);
        best.entry(key).or_insert(i);
    }
    for i in 0..rows.len() {
        let selected = best[&endpoint(&rows[i])];
        if selected != i {
            let incoming = rows[i].clone();
            merge_discovery_evidence(&mut rows[selected], &incoming);
        }
    }
    let selected = best.values().copied().collect::<Vec<_>>();
    let eligible = selected
        .iter()
        .copied()
        .filter(|&i| reason(&rows[i]).is_empty())
        .collect::<Vec<_>>();
    let response_summary = annotate_exact_response_groups(&mut rows, &eligible);
    let (mut keep, mut excluded, mut response_duplicates, mut all) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for (i, row) in rows.iter().enumerate() {
        let is_selected = best.get(&endpoint(row)).copied() == Some(i);
        let why = if is_selected {
            reason(row)
        } else {
            "duplicate_endpoint".into()
        };
        let mut out = row.clone();
        if why.is_empty() {
            out.insert("auto_excluded".into(), "0".into());
            out.insert("exclude_reason".into(), String::new());
            if text(&out, &["response_duplicate_of"]).is_empty() {
                keep.push(out.clone());
            } else {
                response_duplicates.push(out.clone());
            }
        } else {
            out.insert("auto_excluded".into(), "1".into());
            out.insert("exclude_reason".into(), why);
            excluded.push(out.clone());
        }
        if is_selected {
            all.push(out);
        }
    }
    for v in ["auto_excluded", "exclude_reason"]
        .into_iter()
        .chain(RESPONSE_GROUP_FIELDS.iter().copied())
    {
        if !fields.iter().any(|x| x == v) {
            fields.push(v.into())
        }
    }
    // Legacy input may not have provenance columns. Evidence union adds them
    // in memory; export those generated fields too rather than silently
    // dropping the original endpoint's second discovery source at the CSV edge.
    let generated_fields = rows
        .iter()
        .flat_map(|row| row.keys().cloned())
        .collect::<BTreeSet<_>>();
    for field in generated_fields {
        if !fields.contains(&field) {
            fields.push(field);
        }
    }
    write_csv(&output.join("optimized_assets.csv"), &fields, &keep)?;
    write_csv(&output.join("auto_excluded.csv"), &fields, &excluded)?;
    write_csv(
        &output.join("response_duplicates.csv"),
        &fields,
        &response_duplicates,
    )?;
    write_csv(&output.join("all_probe_results.csv"), &fields, &all)?;
    let summary = json!({"input":rows.len(),"importable":all.len(),"kept":keep.len(),"excluded":excluded.len(),"response_groups":response_summary.groups,"response_duplicates":response_summary.duplicates,"endpoint_duplicates":rows.len()-all.len(),"web_alive":rows.iter().filter(|r|matches!(text(r,&["probe_outcome"]).as_str(),"web_alive"|"web_restricted"|"browser_render_required"|"virtual_host_required")).count(),"tcp_non_web":rows.iter().filter(|r|text(r,&["probe_outcome"])=="tcp_alive_non_http").count(),"web_abnormal":rows.iter().filter(|r|text(r,&["probe_outcome"])=="web_abnormal").count(),"unreachable":rows.iter().filter(|r|reason(r).starts_with("unreachable:")).count()});
    fs::write(
        output.join("summary.json"),
        serde_json::to_vec_pretty(&summary).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    Ok(summary)
}
