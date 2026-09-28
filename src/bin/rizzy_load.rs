//! Independent HTTP load client for the scoped API comparison.
//! Credentials and target paths are supplied through the environment, so tokens
//! do not appear in the process command line or benchmark result file.

use std::{
    collections::BTreeMap,
    env,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use reqwest::{Client, redirect::Policy};
use serde_json::{Value, json};
use tokio::time::Instant;

fn required(name: &str) -> Result<String, Box<dyn std::error::Error>> {
    Ok(env::var(name).map_err(|_| format!("{name} is required"))?)
}

fn percentile(sorted: &[f64], fraction: f64) -> Option<f64> {
    if sorted.is_empty() {
        return None;
    }
    let index = ((sorted.len() - 1) as f64 * fraction) as usize;
    Some((sorted[index] * 1000.0 * 100.0).round() / 100.0)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = required("LOAD_URL")?.trim_end_matches('/').to_owned();
    let roots: Vec<String> = match env::var("LOAD_URLS_JSON") {
        Ok(value) => serde_json::from_str(&value)?,
        Err(env::VarError::NotPresent) => vec![root],
        Err(error) => return Err(error.into()),
    };
    if roots.is_empty() || roots.iter().any(|url| url.is_empty()) {
        return Err("LOAD_URLS_JSON must contain at least one server URL".into());
    }
    let account = required("LOAD_ACCOUNT")?;
    let token = required("LOAD_TOKEN")?;
    let paths: Vec<String> = serde_json::from_str(&required("LOAD_PATHS_JSON")?)?;
    if paths.is_empty() || paths.iter().any(|path| !path.starts_with('/')) {
        return Err("LOAD_PATHS_JSON must contain absolute application paths".into());
    }
    let concurrency: usize = required("LOAD_CONCURRENCY")?.parse()?;
    let duration: f64 = required("LOAD_SECONDS")?.parse()?;
    if concurrency == 0 || !duration.is_finite() || duration <= 0.0 {
        return Err("LOAD_CONCURRENCY and LOAD_SECONDS must be positive".into());
    }
    let urls: Arc<Vec<Vec<String>>> = Arc::new(
        roots
            .iter()
            .map(|root| {
                paths
                    .iter()
                    .map(|path| format!("{}/{account}{path}", root.trim_end_matches('/')))
                    .collect()
            })
            .collect(),
    );
    let client = Client::builder()
        .pool_max_idle_per_host(concurrency)
        .timeout(Duration::from_secs(15))
        .redirect(Policy::none())
        .build()?;
    let token = Arc::new(token);
    let sequence = Arc::new(AtomicUsize::new(0));
    let started = Instant::now();
    let deadline = started + Duration::from_secs_f64(duration);
    let mut tasks = Vec::with_capacity(concurrency);
    for _ in 0..concurrency {
        let client = client.clone();
        let urls = urls.clone();
        let token = token.clone();
        let sequence = sequence.clone();
        tasks.push(tokio::spawn(async move {
            let mut samples = Vec::new();
            let mut errors = BTreeMap::<String, u64>::new();
            let mut bytes_read = 0_u64;
            let mut path_requests = vec![0_u64; urls[0].len()];
            let mut path_bytes = vec![0_u64; urls[0].len()];
            let mut server_requests = vec![0_u64; urls.len()];
            while Instant::now() < deadline {
                let request_index = sequence.fetch_add(1, Ordering::Relaxed);
                let path_index = request_index % urls[0].len();
                let server_index = request_index / urls[0].len() % urls.len();
                let began = Instant::now();
                let result = client
                    .get(&urls[server_index][path_index])
                    .bearer_auth(token.as_str())
                    .header(reqwest::header::ACCEPT, "application/json")
                    .send()
                    .await;
                match result {
                    Ok(response) if response.status().is_success() => {
                        match response.bytes().await {
                            Ok(body) => {
                                bytes_read += body.len() as u64;
                                path_requests[path_index] += 1;
                                path_bytes[path_index] += body.len() as u64;
                                server_requests[server_index] += 1;
                                samples.push(began.elapsed().as_secs_f64());
                            }
                            Err(error) => {
                                let key = if error.is_timeout() {
                                    "timeout"
                                } else {
                                    "body_error"
                                };
                                *errors.entry(key.to_owned()).or_default() += 1;
                            }
                        }
                    }
                    Ok(response) => {
                        *errors
                            .entry(format!("status_{}", response.status().as_u16()))
                            .or_default() += 1;
                    }
                    Err(error) => {
                        let key = if error.is_timeout() {
                            "timeout"
                        } else {
                            "transport_error"
                        };
                        *errors.entry(key.to_owned()).or_default() += 1;
                    }
                }
            }
            (
                samples,
                errors,
                bytes_read,
                path_requests,
                path_bytes,
                server_requests,
            )
        }));
    }
    let mut samples = Vec::new();
    let mut errors = BTreeMap::<String, u64>::new();
    let mut bytes_read = 0_u64;
    let mut path_requests = vec![0_u64; urls[0].len()];
    let mut path_bytes = vec![0_u64; urls[0].len()];
    let mut server_requests = vec![0_u64; urls.len()];
    for task in tasks {
        let (latencies, failures, bytes, counts, sizes, servers) = task.await?;
        samples.extend(latencies);
        bytes_read += bytes;
        for (index, count) in counts.into_iter().enumerate() {
            path_requests[index] += count;
        }
        for (index, size) in sizes.into_iter().enumerate() {
            path_bytes[index] += size;
        }
        for (index, count) in servers.into_iter().enumerate() {
            server_requests[index] += count;
        }
        for (key, value) in failures {
            *errors.entry(key).or_default() += value;
        }
    }
    let elapsed = started.elapsed().as_secs_f64();
    samples.sort_by(f64::total_cmp);
    let output: Value = json!({
        "concurrency": concurrency,
        "seconds": (elapsed * 100.0).round() / 100.0,
        "requests": samples.len(),
        "errors": errors,
        "rps": (samples.len() as f64 / elapsed * 10.0).round() / 10.0,
        "p50_ms": percentile(&samples, 0.5),
        "p95_ms": percentile(&samples, 0.95),
        "p99_ms": percentile(&samples, 0.99),
        "response_bytes": bytes_read,
        "path_requests": path_requests,
        "path_response_bytes": path_bytes,
        "server_requests": server_requests,
    });
    println!("{}", serde_json::to_string(&output)?);
    Ok(())
}
