use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::PathBuf;

use anyhow::Result;
use polars::prelude::*;
use rand::prelude::*;
use tikv_jemallocator::Jemalloc;

mod download;

#[global_allocator]
static GLOBAL: Jemalloc = Jemalloc;

const CRAWL: &str = "CC-MAIN-2020-34";
const SAMPLE_FILES: usize = 8; // how many of the ~300 index part-files to actually fetch
const MAX_URL_SAMPLE: usize = 5_000_000; // final cap on the URL list; usize::MAX to keep everything
const OUTPUT_PATH: &str = "urls.txt";

#[tokio::main]
async fn main() -> Result<()> {
    let data_dir = PathBuf::from("cc-index-data");
    fs::create_dir_all(&data_dir)?;

    if std::env::var("USE_LOCAL").ok().is_none() {
        println!("Downloading index files from Common Crawl...");

        let paths = crate::download::download_paths(CRAWL, "cc-index-table", &["warc"]).await?;
        println!("manifest lists {} index files total", paths.len());

        let sampled_paths: Vec<_> = paths
            .sample(&mut rand::rng(), SAMPLE_FILES)
            .map(|s| s.as_str())
            .collect();

        dbg!(&sampled_paths);

        crate::download::download(&sampled_paths, &data_dir, false).await?;
    }

    let glob_pattern = data_dir.join("**/*.parquet");

    let mut df = LazyFrame::scan_parquet(
        glob_pattern.to_str().expect("bad path").into(),
        Default::default(),
    )?
    .filter(col("fetch_status").eq(lit(200)))
    .filter(col("content_languages").str().contains_literal(lit("eng")))
    .filter(col("content_mime_type").eq(lit("text/html")))
    .select([col("url")])
    .unique(None, UniqueKeepStrategy::Any)
    .collect()?;

    if df.height() > MAX_URL_SAMPLE {
        df = df.sample_n_literal(MAX_URL_SAMPLE, false, None, Some(42))?;
    }

    let urls = df.column("url")?.str()?;
    let mut writer = BufWriter::new(File::create(OUTPUT_PATH)?);
    for url in urls.no_null_iter() {
        writeln!(writer, "{url}")?;
    }

    println!("wrote {} urls to {OUTPUT_PATH}", df.height());
    Ok(())
}
