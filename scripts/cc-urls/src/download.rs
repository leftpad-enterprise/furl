use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use async_compression::futures::bufread::GzipDecoder;
use futures::{AsyncReadExt, TryStreamExt};
use ratelimit::{Ratelimiter, TryWaitError};
use reqwest::{Client, Url};
use reqwest_middleware::{ClientBuilder, ClientWithMiddleware};
use reqwest_retry::policies::ExponentialBackoff;
use reqwest_retry::{Jitter, RetryTransientMiddleware};
use tokio::io::{AsyncWriteExt, BufWriter};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

const MAX_RETRIES: u32 = 64;
const THREADS: usize = 8;

const BASE_URL: &str = "https://data.commoncrawl.org/";

static APP_USER_AGENT: &str = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"),);

fn new_client() -> Result<ClientWithMiddleware> {
    let retry_policy = ExponentialBackoff::builder()
        .retry_bounds(Duration::from_secs(1), Duration::from_secs(3600))
        .jitter(Jitter::Bounded)
        .base(2)
        .build_with_max_retries(MAX_RETRIES);

    let client_base = Client::builder().user_agent(APP_USER_AGENT).build()?;

    Ok(ClientBuilder::new(client_base)
        .with(RetryTransientMiddleware::new_with_policy(retry_policy))
        .build())
}

/// Downloads the paths index file for a Common Crawl snapshot and data type.
pub async fn download_paths(
    snapshot: &str,
    data_type: &str,
    subsets: &[&str],
) -> Result<Vec<String>> {
    let paths = format!("{}crawl-data/{}/{}.paths.gz", BASE_URL, snapshot, data_type);
    println!("Downloading paths from: {paths}");
    let url = Url::parse(&paths)?;

    let client = new_client()?;

    client.head(url.clone()).send().await?.error_for_status()?;

    let request = client.get(url);

    let reader = request
        .send()
        .await?
        .bytes_stream()
        .map_err(std::io::Error::other)
        .into_async_read();
    let mut decoder = GzipDecoder::new(futures::io::BufReader::new(reader));

    let mut rel_paths = String::new();
    decoder.read_to_string(&mut rel_paths).await?;

    let filtered = rel_paths
        .lines()
        .filter(|l| {
            subsets.is_empty() || subsets.iter().any(|s| l.contains(&format!("subset={s}")))
        })
        .map(ToOwned::to_owned)
        .collect();

    Ok(filtered)
}

// Based on: https://github.com/benkay86/async-applied/blob/master/indicatif-reqwest-tokio/src/bin/indicatif-reqwest-tokio-multi.rs
async fn download_task(
    client: ClientWithMiddleware,
    path: &Url,
    mut dst: PathBuf,
    files_only: bool,
    limiter: Arc<Ratelimiter>,
) -> Result<()> {
    // Parse the filename from the given URL
    let filename = if files_only {
        path.path_segments()
            .and_then(|mut segments| segments.next_back())
            .unwrap_or("file.download")
    } else {
        path.path().strip_prefix("/").unwrap_or("file.download")
    };

    dst.push(filename);

    // Create the directory if it doesn't exist
    if let Some(parent) = dst.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    // Acquire a permit from the rate limiter before making the request.
    loop {
        match limiter.try_wait() {
            Ok(()) => break,
            Err(TryWaitError::Insufficient(wait)) => tokio::time::sleep(wait).await,
            Err(TryWaitError::ExceedsCapacity) => unreachable!("max_tokens > 0"),
            Err(_) => unreachable!(),
        }
    }

    println!("Downloading: {path}");
    let mut download = client.get(path.as_str()).send().await?;

    // stream the response to the file
    let mut outfile = BufWriter::new(tokio::fs::File::create(&dst).await?);
    while let Some(chunk) = download.chunk().await? {
        outfile.write_all(&chunk).await?;
    }
    outfile.flush().await?;

    println!("Downloaded file to: {}", dst.display());

    Ok(())
}

/// Downloads every file listed in `paths` relative to the CC base URL.
pub async fn download(paths: &[&str], dst: &Path, files_only: bool) -> Result<()> {
    let base = Url::parse(BASE_URL).expect("bad");

    let paths: Vec<Url> = paths
        .iter()
        .map(|line| base.join(line))
        .collect::<Result<_, _>>()?;

    let client = new_client()?;

    let rate_limiter = Arc::new(
        Ratelimiter::builder(1499)
            .period(Duration::from_secs(300))
            .build()
            .expect("invalid rate limit config"),
    );

    let semaphore = Arc::new(Semaphore::new(THREADS));
    let mut set = JoinSet::new();

    for path in paths.into_iter() {
        let client = client.clone();
        let dst = dst.to_path_buf();
        let semaphore = semaphore.clone();
        let rate_limiter = rate_limiter.clone();
        set.spawn(async move {
            let _permit = semaphore.acquire().await;

            download_task(client, &path, dst, files_only, rate_limiter).await
        });
    }

    // Wait for the tasks to finish.
    while let Some(result) = set.join_next().await {
        match result {
            Ok(Ok(())) => {}
            Ok(Err(e)) => eprintln!("Error: {e:?}"),
            Err(e) => eprintln!("Error: {e:?}"),
        }
    }

    println!("All downloads completed");
    Ok(())
}
