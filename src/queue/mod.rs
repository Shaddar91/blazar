//! On-disk queue + daily counter.

use std::{
    fs::{self, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use chrono::Utc;
use fs2::FileExt;

use crate::models::Message;

pub struct DiskQueue {
    pub path: PathBuf,
}

impl DiskQueue {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn enqueue(&self, msg: &Message) -> Result<()> {
        enqueue(&self.path, msg)
    }

    pub fn flush_all(&self) -> Result<Vec<Message>> {
        flush_all(&self.path)
    }
}

/// Persist `msg` to `<queue_dir>/<id>.json`.
pub fn enqueue(queue_dir: &Path, msg: &Message) -> Result<()> {
    fs::create_dir_all(queue_dir)
        .with_context(|| format!("creating queue dir {}", queue_dir.display()))?;
    let file = queue_dir.join(format!("{}.json", msg.id));
    let json = serde_json::to_vec_pretty(msg).context("serializing queued message")?;
    fs::write(&file, json).with_context(|| format!("writing {}", file.display()))?;
    Ok(())
}

/// Read + remove every `*.json` under `queue_dir`.
pub fn flush_all(queue_dir: &Path) -> Result<Vec<Message>> {
    if !queue_dir.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in fs::read_dir(queue_dir)? {
        let entry = entry?;
        let p = entry.path();
        if p.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        // counter files end in .txt, skip just in case.
        let bytes = match fs::read(&p) {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!(path = %p.display(), error = ?e, "queue read failed");
                continue;
            }
        };
        match serde_json::from_slice::<Message>(&bytes) {
            Ok(m) => {
                out.push(m);
                if let Err(e) = fs::remove_file(&p) {
                    tracing::warn!(path = %p.display(), error = ?e, "queue unlink failed");
                }
            }
            Err(e) => {
                tracing::warn!(path = %p.display(), error = ?e, "queue parse failed — leaving file");
            }
        }
    }
    Ok(out)
}

/// Increment today's counter (atomic, flock-protected). `Ok(true)` if within cap.
pub fn check_and_increment(queue_dir: &Path, daily_cap: u32) -> Result<bool> {
    fs::create_dir_all(queue_dir)
        .with_context(|| format!("creating queue dir {}", queue_dir.display()))?;
    let today = Utc::now().format("%Y-%m-%d").to_string();
    let path = queue_dir.join(format!("counter-{today}.txt"));

    let mut f = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(&path)
        .with_context(|| format!("opening counter {}", path.display()))?;
    f.lock_exclusive().context("flock counter")?;

    let mut buf = String::new();
    f.read_to_string(&mut buf)?;
    let cur: u32 = buf.trim().parse().unwrap_or(0);
    if cur >= daily_cap {
        let _ = fs2::FileExt::unlock(&f);
        return Ok(false);
    }
    let next = cur + 1;
    f.seek(SeekFrom::Start(0))?;
    f.set_len(0)?;
    write!(f, "{next}")?;
    f.flush()?;
    let _ = fs2::FileExt::unlock(&f);

    Ok(true)
}
