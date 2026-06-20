use chrono::{DateTime, Local, Utc};
use reqwest::blocking::Client;
use reqwest::StatusCode;
use serde_json::Value;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

const DEFAULT_BASE_URL: &str = "https://chatgpt.com/backend-api";
const RESET_CREDITS_PATH: &str = "/wham/rate-limit-reset-credits";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let args = Args::parse(env::args().skip(1).collect())?;
    let auth_path = resolve_auth_path(args.auth_path)?;
    let auth = load_codex_auth(&auth_path)?;
    let payload = fetch_reset_credits(&args.base_url, &auth)?;
    let snapshot = extract_snapshot(&payload);

    if args.json {
        print_json(&snapshot)?;
    } else if args.verbose {
        print_verbose(&snapshot, &payload, args.show_ids)?;
    } else {
        print_human(&snapshot, args.show_ids);
    }

    Ok(())
}

#[derive(Debug)]
struct Args {
    auth_path: Option<PathBuf>,
    base_url: String,
    json: bool,
    verbose: bool,
    show_ids: bool,
}

impl Args {
    fn parse(raw_args: Vec<String>) -> Result<Self, String> {
        let mut auth_path = None;
        let mut base_url = DEFAULT_BASE_URL.to_string();
        let mut json = false;
        let mut verbose = false;
        let mut show_ids = false;
        let mut i = 0;

        while i < raw_args.len() {
            match raw_args[i].as_str() {
                "--auth" => {
                    i += 1;
                    let value = raw_args
                        .get(i)
                        .ok_or_else(|| "--auth requires a path".to_string())?;
                    auth_path = Some(expand_tilde(value));
                }
                "--base-url" => {
                    i += 1;
                    let value = raw_args
                        .get(i)
                        .ok_or_else(|| "--base-url requires a URL".to_string())?;
                    base_url = value.trim_end_matches('/').to_string();
                }
                "--json" => json = true,
                "--verbose" | "-v" => verbose = true,
                "--show-ids" => show_ids = true,
                "-h" | "--help" => {
                    print_help();
                    std::process::exit(0);
                }
                unknown => return Err(format!("unknown argument: {unknown}")),
            }
            i += 1;
        }

        Ok(Self {
            auth_path,
            base_url,
            json,
            verbose,
            show_ids,
        })
    }
}

#[derive(Debug)]
struct CodexAuth {
    access_token: String,
    account_id: String,
}

#[derive(Debug, serde::Serialize)]
struct ResetSnapshot {
    available_count: Option<i64>,
    total_earned_count: Option<i64>,
    available_credits: Vec<ResetCredit>,
    oldest_expiring_credit: Option<ResetCredit>,
}

#[derive(Clone, Debug, serde::Serialize)]
struct ResetCredit {
    id: Option<String>,
    status: String,
    reset_type: Option<String>,
    granted_at: Option<String>,
    expires_at: String,
    expires_in: String,
}

fn print_help() {
    println!(
        "\
reset-bank

Usage:
  reset-bank [--auth PATH] [--verbose] [--json] [--show-ids]

Reads Codex auth from $CODEX_HOME/auth.json or ~/.codex/auth.json, then prints
available banked reset credits and their expires_at values.
"
    );
}

fn resolve_auth_path(auth_path: Option<PathBuf>) -> Result<PathBuf, String> {
    if let Some(path) = auth_path {
        return Ok(path);
    }

    if let Ok(codex_home) = env::var("CODEX_HOME") {
        if !codex_home.trim().is_empty() {
            return Ok(PathBuf::from(codex_home).join("auth.json"));
        }
    }

    let home = env::var("HOME").map_err(|_| "HOME is not set".to_string())?;
    Ok(PathBuf::from(home).join(".codex").join("auth.json"))
}

fn load_codex_auth(path: &PathBuf) -> Result<CodexAuth, String> {
    let raw = fs::read_to_string(path)
        .map_err(|err| format!("failed to read auth file {}: {err}", path.display()))?;
    let json: Value = serde_json::from_str(&raw)
        .map_err(|err| format!("auth file is not valid JSON {}: {err}", path.display()))?;

    let access_token = value_str(&json, &["access_token"])
        .or_else(|| value_str(&json, &["tokens", "access_token"]))
        .ok_or_else(|| format!("auth file has no ChatGPT access_token: {}", path.display()))?;
    let account_id = value_str(&json, &["account_id"])
        .or_else(|| value_str(&json, &["tokens", "account_id"]))
        .ok_or_else(|| format!("auth file has no ChatGPT account_id: {}", path.display()))?;

    Ok(CodexAuth {
        access_token,
        account_id,
    })
}

fn value_str(json: &Value, path: &[&str]) -> Option<String> {
    let mut current = json;
    for key in path {
        current = current.get(*key)?;
    }
    current.as_str().map(ToOwned::to_owned)
}

fn fetch_reset_credits(base_url: &str, auth: &CodexAuth) -> Result<Value, String> {
    let url = format!("{}{}", base_url.trim_end_matches('/'), RESET_CREDITS_PATH);
    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|err| format!("failed to build HTTP client: {err}"))?;

    let response = client
        .get(&url)
        .bearer_auth(&auth.access_token)
        .header("ChatGPT-Account-Id", &auth.account_id)
        .header("Accept", "application/json")
        .header("OpenAI-Beta", "codex-1")
        .header("originator", "Codex Desktop")
        .header("User-Agent", "reset-bank/0.1")
        .send()
        .map_err(|err| format!("request failed: {err}"))?;

    let status = response.status();
    let body = response
        .text()
        .map_err(|err| format!("failed to read response body: {err}"))?;

    if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
        return Err(format!(
            "auth rejected by Codex backend ({status}); run `codex login`"
        ));
    }
    if !status.is_success() {
        let detail = body.chars().take(240).collect::<String>();
        return Err(format!("Codex backend returned {status}: {detail}"));
    }

    let payload = serde_json::from_str::<Value>(&body)
        .map_err(|err| format!("unexpected response JSON: {err}"))?;
    if !payload.is_object() {
        return Err("unexpected response JSON: root is not an object".to_string());
    }
    Ok(payload)
}

fn extract_snapshot(payload: &Value) -> ResetSnapshot {
    let credits_array = payload
        .get("credits")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let mut available_credits = credits_array
        .iter()
        .filter_map(normalize_credit)
        .collect::<Vec<_>>();
    available_credits.sort_by(|left, right| left.expires_at.cmp(&right.expires_at));

    ResetSnapshot {
        available_count: payload.get("available_count").and_then(Value::as_i64),
        total_earned_count: payload.get("total_earned_count").and_then(Value::as_i64),
        oldest_expiring_credit: available_credits.first().cloned(),
        available_credits,
    }
}

fn normalize_credit(value: &Value) -> Option<ResetCredit> {
    let status = value.get("status")?.as_str()?.to_string();
    if !status.eq_ignore_ascii_case("available") {
        return None;
    }

    let expires_at = parse_utc(value.get("expires_at")?.as_str()?)?;
    let granted_at = value
        .get("granted_at")
        .and_then(Value::as_str)
        .and_then(parse_utc);
    let now = Utc::now();

    Some(ResetCredit {
        id: value
            .get("id")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        status,
        reset_type: value
            .get("reset_type")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        granted_at: granted_at.map(|value| value.to_rfc3339()),
        expires_at: expires_at.to_rfc3339(),
        expires_in: format_duration(expires_at.signed_duration_since(now)),
    })
}

fn parse_utc(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|parsed| parsed.with_timezone(&Utc))
}

fn print_json(snapshot: &ResetSnapshot) -> Result<(), String> {
    let rendered = serde_json::to_string_pretty(snapshot)
        .map_err(|err| format!("failed to render JSON: {err}"))?;
    println!("{rendered}");
    Ok(())
}

fn print_human(snapshot: &ResetSnapshot, show_ids: bool) {
    let count = snapshot
        .available_count
        .map(|value| value.to_string())
        .unwrap_or_else(|| "unknown".to_string());
    println!("banked reset credits: {count} available");
    if let Some(total_earned_count) = snapshot.total_earned_count {
        println!("total earned count : {total_earned_count}");
    }

    let Some(oldest) = &snapshot.oldest_expiring_credit else {
        println!("next expiry: none");
        return;
    };

    println!(
        "next expiry: {} ({})",
        format_local(&oldest.expires_at),
        oldest.expires_in
    );
    println!();
    print_credit_table(&snapshot.available_credits, show_ids);
}

fn print_verbose(snapshot: &ResetSnapshot, payload: &Value, show_ids: bool) -> Result<(), String> {
    print_human(snapshot, show_ids);
    println!();
    println!("raw reset-credit payload:");
    let rendered = serde_json::to_string_pretty(payload)
        .map_err(|err| format!("failed to render verbose JSON: {err}"))?;
    println!("{rendered}");
    Ok(())
}

fn print_credit_table(credits: &[ResetCredit], show_ids: bool) {
    if credits.is_empty() {
        println!("available credits: none");
        return;
    }

    println!("available credits:");
    println!(
        "{:<4} {:<11} {:<27} {:<27} {}",
        "#", "expires_in", "expires_at", "granted_at", "credit_id"
    );

    for (index, credit) in credits.iter().enumerate() {
        let id = credit.id.as_deref().unwrap_or("unknown");
        let display_id = if show_ids {
            id.to_string()
        } else {
            mask_credit_id(id)
        };
        let granted_at = credit
            .granted_at
            .as_deref()
            .map(format_local)
            .unwrap_or_else(|| "unknown".to_string());

        println!(
            "{:<4} {:<11} {:<27} {:<27} {}",
            index + 1,
            credit.expires_in,
            format_local(&credit.expires_at),
            granted_at,
            display_id
        );
    }
}

fn format_local(value: &str) -> String {
    DateTime::parse_from_rfc3339(value)
        .map(|parsed| {
            parsed
                .with_timezone(&Local)
                .format("%Y-%m-%d %H:%M:%S %Z")
                .to_string()
        })
        .unwrap_or_else(|_| value.to_string())
}

fn format_duration(delta: chrono::Duration) -> String {
    let seconds = delta.num_seconds();
    if seconds <= 0 {
        return "expired".to_string();
    }

    let days = seconds / 86_400;
    let hours = (seconds % 86_400) / 3_600;
    let minutes = (seconds % 3_600) / 60;

    if days > 0 {
        format!("in {days}d {hours}h")
    } else if hours > 0 {
        format!("in {hours}h {minutes}m")
    } else {
        format!("in {minutes}m")
    }
}

fn mask_credit_id(value: &str) -> String {
    let chars = value.chars().collect::<Vec<_>>();
    if chars.len() <= 12 {
        return value.to_string();
    }

    let start = chars.iter().take(8).collect::<String>();
    let end = chars
        .iter()
        .rev()
        .take(4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<String>();
    format!("{start}...{end}")
}

fn expand_tilde(value: &str) -> PathBuf {
    if value == "~" {
        return env::var("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from(value));
    }

    if let Some(rest) = value.strip_prefix("~/") {
        if let Ok(home) = env::var("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }

    PathBuf::from(value)
}
