//! `lbt info`: print framebuffer + BGRT logo geometry.

use std::path::PathBuf;

use anyhow::Result;

use crate::geometry::Geometry;

pub fn run(dump: Option<PathBuf>) -> Result<()> {
    let g = Geometry::load(dump.as_deref())?;
    println!("{}", g.report());
    Ok(())
}
