use std::path::PathBuf;

use clap::Args;

use super::{emit, run_data_audit_over};

#[derive(Args)]
pub(crate) struct DataAuditArgs {
    /// Data root to audit: a local path, or `s3://bucket[/prefix]`.
    #[arg(long)]
    data: String,
    #[arg(long, default_value_t = 20000101)]
    from: i32,
    #[arg(long, default_value_t = 99991231)]
    to: i32,
    /// Emit the full report as JSON instead of the human table.
    #[arg(long)]
    json: bool,
    /// Output file (default: stdout).
    #[arg(long)]
    out: Option<PathBuf>,
}

pub(crate) fn run(args: DataAuditArgs) -> Result<(), Box<dyn std::error::Error>> {
    let DataAuditArgs {
        data,
        from,
        to,
        json,
        out,
    } = args;
    let report = run_data_audit_over(&data, from, to)?;
    let overall = report.overall;
    let body = if json {
        serde_json::to_string_pretty(&report)?
    } else {
        pomelo_audit::render_table(&report)
    };
    emit(&out, body)?;
    // Non-zero exit on a FAIL so the audit can gate CI / a nightly job.
    if overall == pomelo_audit::Status::Fail {
        std::process::exit(2);
    }
    Ok(())
}
