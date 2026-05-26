use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::Instant;

use arrow::array::StringArray;
use arrow::record_batch::RecordBatch;
use clap::{Parser, Subcommand};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use toradb_engine::DagRunner;

#[derive(Parser)]
#[command(name = "toradb-ingest", about = "ToraDB bulk ingest and index build")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Stream source data into Parquet segments (bulk mode), then finish indexes.
    Bulk(BulkArgs),
    /// Build segment BM25 sidecars after bulk load (no active bulk session required).
    Finish(FinishArgs),
    /// Resume or rerun index build (idempotent segment skip).
    Resume(FinishArgs),
}

#[derive(Parser)]
struct BulkArgs {
    #[arg(long)]
    db: PathBuf,
    #[arg(long, default_value = "passages")]
    table: String,
    #[arg(long, default_value = "parquet")]
    source: String,
    #[arg(long)]
    path: Option<PathBuf>,
    #[arg(long)]
    jsonl: Option<PathBuf>,
    #[arg(long, default_value = "200000")]
    batch_size: usize,
    #[arg(long, default_value = "0")]
    limit: u64,
    #[arg(long, action = clap::ArgAction::SetTrue)]
    drop_table: bool,
    #[arg(long, action = clap::ArgAction::SetTrue)]
    no_finish: bool,
}

#[derive(Parser)]
struct FinishArgs {
    #[arg(long)]
    db: PathBuf,
    #[arg(long, default_value = "passages")]
    table: String,
    #[arg(long, action = clap::ArgAction::SetTrue)]
    compact: bool,
}

fn main() -> Result<(), String> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Bulk(args) => run_bulk(args),
        Commands::Finish(args) => run_index_op(args, false),
        Commands::Resume(args) => run_index_op(args, true),
    }
}

fn run_index_op(args: FinishArgs, resume: bool) -> Result<(), String> {
    let t0 = Instant::now();
    let mut dag = DagRunner::open_with_reload(&args.db, false)?;
    dag.ensure_table(&args.table);
    if resume {
        dag.resume_index_build(&args.table, args.compact)?;
        println!("resume_index_build: {:.1}s", t0.elapsed().as_secs_f64());
    } else {
        dag.resume_index_build(&args.table, args.compact)?;
        println!("finish (resume path): {:.1}s", t0.elapsed().as_secs_f64());
    }
    Ok(())
}

fn run_bulk(args: BulkArgs) -> Result<(), String> {
    if args.drop_table {
        let table_dir = args.db.join(&args.table);
        if table_dir.exists() {
            std::fs::remove_dir_all(&table_dir).map_err(|e| e.to_string())?;
            eprintln!("dropped {}", table_dir.display());
        }
    }

    let mut dag = DagRunner::open_with_reload(&args.db, false)?;
    dag.ensure_table(&args.table);
    dag.begin_bulk_ingest(&args.table);

    let t0 = Instant::now();
    let total = match args.source.as_str() {
        "parquet" => {
            let path = args.path.ok_or("--path required for parquet")?;
            ingest_parquet(&mut dag, &args.table, &path, args.limit)?
        }
        "jsonl" => {
            let path = args
                .jsonl
                .or(args.path)
                .ok_or("--jsonl or --path required for jsonl")?;
            ingest_jsonl(&mut dag, &args.table, &path, args.batch_size, args.limit)?
        }
        other => return Err(format!("unsupported source {other}")),
    };

    eprintln!(
        "ingested {total} docs in {:.1}s ({:.0} docs/s)",
        t0.elapsed().as_secs_f64(),
        total as f64 / t0.elapsed().as_secs_f64().max(0.001)
    );

    if !args.no_finish {
        eprintln!("finish: building segment BM25 + indexes…");
        let t1 = Instant::now();
        dag.finish_bulk_ingest(&args.table, false)?;
        eprintln!("finish: {:.1}s", t1.elapsed().as_secs_f64());
    } else {
        eprintln!("skipped finish (--no-finish); run: toradb-ingest finish --db … --table …");
    }
    Ok(())
}

fn ingest_parquet(
    dag: &mut DagRunner,
    table: &str,
    path: &Path,
    limit: u64,
) -> Result<u64, String> {
    let files = parquet_files(path)?;
    let mut total = 0u64;
    for file in files {
        if limit > 0 && total >= limit {
            break;
        }
        let f = File::open(&file).map_err(|e| e.to_string())?;
        let builder = ParquetRecordBatchReaderBuilder::try_new(f).map_err(|e| e.to_string())?;
        let mut reader = builder.build().map_err(|e| e.to_string())?;
        for batch in reader.by_ref() {
            if limit > 0 && total >= limit {
                break;
            }
            let batch: RecordBatch = batch.map_err(|e| e.to_string())?;
            if batch.num_rows() == 0 {
                continue;
            }
            if limit > 0 {
                let remain = (limit - total) as usize;
                if batch.num_rows() > remain {
                    let batch = batch.slice(0, remain);
                    let added = dag.ingest_record_batch(table, &batch)? as u64;
                    total += added;
                    break;
                }
            }
            let added = dag.ingest_record_batch(table, &batch)? as u64;
            total += added;
        }
    }
    Ok(total)
}

fn parquet_files(path: &Path) -> Result<Vec<PathBuf>, String> {
    if path.is_file() {
        return Ok(vec![path.to_path_buf()]);
    }
    let mut out = Vec::new();
    for entry in std::fs::read_dir(path).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let p = entry.path();
        if p.extension().and_then(|e| e.to_str()) == Some("parquet") {
            out.push(p);
        }
    }
    out.sort();
    if out.is_empty() {
        return Err(format!("no parquet files under {}", path.display()));
    }
    Ok(out)
}

fn ingest_jsonl(
    dag: &mut DagRunner,
    table: &str,
    path: &Path,
    batch_size: usize,
    limit: u64,
) -> Result<u64, String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    let reader = BufReader::new(file);
    let mut texts: Vec<String> = Vec::with_capacity(batch_size);
    let mut total = 0u64;

    for line in reader.lines() {
        if limit > 0 && total >= limit {
            break;
        }
        let line = line.map_err(|e| e.to_string())?;
        let row: serde_json::Value = serde_json::from_str(&line).map_err(|e| e.to_string())?;
        let text = row
            .get("text")
            .or_else(|| row.get("passage"))
            .or_else(|| row.get("content"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if text.is_empty() {
            continue;
        }
        texts.push(text);
        total += 1;
        if texts.len() >= batch_size {
            flush_text_batch(dag, table, &mut texts)?;
        }
    }
    if !texts.is_empty() {
        flush_text_batch(dag, table, &mut texts)?;
    }
    Ok(total)
}

fn flush_text_batch(dag: &mut DagRunner, table: &str, texts: &mut Vec<String>) -> Result<(), String> {
    if texts.is_empty() {
        return Ok(());
    }
    let schema = arrow::datatypes::Schema::new(vec![arrow::datatypes::Field::new(
        "text",
        arrow::datatypes::DataType::Utf8,
        false,
    )]);
    let arr = StringArray::from(std::mem::take(texts));
    let batch = RecordBatch::try_new(std::sync::Arc::new(schema), vec![std::sync::Arc::new(arr)])
        .map_err(|e| e.to_string())?;
    dag.ingest_record_batch(table, &batch)?;
    Ok(())
}
