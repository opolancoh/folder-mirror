use std::fs::File;
use std::io::{BufReader, BufWriter, Write};
use std::path::Path;

use crate::features::plan::SyncPlan;
use crate::shared::error::{MirrorError, Result};

pub fn write_plan(plan: &SyncPlan, path: &Path) -> Result<()> {
    let mut writer = BufWriter::new(File::create(path)?);
    serde_json::to_writer_pretty(&mut writer, plan).map_err(|e| MirrorError::Report(e.to_string()))?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    Ok(())
}

pub fn read_plan(path: &Path) -> Result<SyncPlan> {
    let file = File::open(path)
        .map_err(|e| MirrorError::PlanFile(format!("cannot open {}: {e}", path.display())))?;
    serde_json::from_reader(BufReader::new(file))
        .map_err(|e| MirrorError::PlanFile(format!("{} is not a valid plan: {e}", path.display())))
}
