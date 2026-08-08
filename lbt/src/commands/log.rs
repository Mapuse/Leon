//! `lbt log`: show, tail, clear and search the Leon boot log.
//!
//! The bootloader appends to `\var\logs\leon\log.md` on the boot volume, so by
//! default these commands look at the repo's staged ESP tree (`build/esp`);
//! `--path` points at a mounted ESP's copy or anywhere else.

use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Result, bail};

use super::util;

fn log_path(path: Option<&str>) -> PathBuf {
    match path {
        Some(p) => PathBuf::from(p),
        None => {
            let staged = util::default_boot_log();
            if staged.is_file() {
                staged
            } else {
                PathBuf::from("var/logs/leon/log.md")
            }
        }
    }
}

pub fn show(path: Option<&str>) -> Result<()> {
    let p = log_path(path);
    if !p.is_file() {
        bail!(
            "no boot log at {} — boot the system once, or pass `--path`",
            p.display()
        );
    }
    println!("{}", std::fs::read_to_string(&p)?);
    Ok(())
}

pub fn tail(path: Option<&str>) -> Result<()> {
    let p = log_path(path);
    if !p.is_file() {
        bail!("no boot log at {} — boot the system once", p.display());
    }
    let mut offset = std::fs::metadata(&p)?.len();
    loop {
        let now = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
        if now < offset {
            offset = 0; // log rotated
        }
        if now > offset {
            use std::io::{Read, Seek, SeekFrom};
            let mut f = std::fs::File::open(&p)?;
            f.seek(SeekFrom::Start(offset))?;
            let mut buf = Vec::new();
            f.read_to_end(&mut buf)?;
            print!("{}", String::from_utf8_lossy(&buf));
            std::io::stdout().flush()?;
            offset = now;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

pub fn clear(path: Option<&str>) -> Result<()> {
    let p = log_path(path);
    if !p.is_file() {
        bail!("no boot log at {} — boot the system once", p.display());
    }
    std::fs::write(&p, "")?;
    println!("cleared {}", p.display());
    Ok(())
}

pub fn find(pattern: Option<&str>, path: Option<&str>) -> Result<()> {
    let pattern = pattern.ok_or_else(|| anyhow::anyhow!("`log find` needs a `--pattern`"))?;
    let p = log_path(path);
    if !p.is_file() {
        bail!("no boot log at {} — boot the system once", p.display());
    }
    let content = std::fs::read_to_string(&p)?;
    let mut hits = 0;
    for (i, line) in content.lines().enumerate() {
        if line.contains(pattern) {
            println!("{}:{}: {}", p.display(), i + 1, line);
            hits += 1;
        }
    }
    if hits == 0 {
        bail!("no matches for `{pattern}` in {}", p.display());
    }
    Ok(())
}
