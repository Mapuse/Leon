//! Screen geometry that themes receive through the `leon_*` context keys.
//!
//! Sources: either a `bootinfo.json` dump written by the bootloader on the
//! boot volume, or live `/sys` discovery (the active console frame buffer via
//! `virtual_size`/`stride`/`bits_per_pixel`, and the ACPI BGRT logo via
//! `/sys/firmware/acpi/bgrt`). Nothing is ever invented or assumed.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};

/// Serde mirrors of the `leon-common` ABI types, for the host-side JSON dump.
mod dump {
    use serde::Deserialize;

    #[derive(Debug, Clone, Deserialize)]
    pub struct Dump {
        pub framebuffer: Framebuffer,
        #[serde(default)]
        pub bgrt: Option<Bgrt>,
    }

    #[derive(Debug, Clone, Deserialize)]
    pub struct Framebuffer {
        pub width: u32,
        pub height: u32,
        #[serde(default)]
        pub stride: u32,
        #[serde(default)]
        pub format: String,
    }

    #[derive(Debug, Clone, Deserialize)]
    pub struct Bgrt {
        pub offset_x: i32,
        pub offset_y: i32,
        pub image_width: u32,
        pub image_height: u32,
        #[serde(default)]
        pub status: u8,
        #[serde(default)]
        pub image_type: u8,
    }
}

/// Screen geometry that themes receive through the `leon_*` context keys.
pub struct Geometry {
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub format: String,
    /// BGRT logo rect `(x0, y0, x1, y1)`, exclusive.
    logo: Option<(i64, i64, i64, i64)>,
    /// BGRT status byte (bit 0 = logo displayed).
    bgrt_status: u8,
    /// BGRT image type (0 = bitmap).
    bgrt_type: u8,
    /// Set when a BGRT exists but could not be parsed (vs. genuinely absent).
    pub logo_warning: Option<String>,
}

impl Geometry {
    pub fn load(dump: Option<&Path>) -> Result<Self> {
        match dump {
            Some(path) => Self::from_dump(path),
            None => Self::from_sysfs(),
        }
    }

    /// Builds geometry from a `bootinfo.json` dump written by the bootloader.
    pub fn from_dump(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("reading dump {}", path.display()))?;
        let d: dump::Dump = serde_json::from_str(&content).context("parsing BootInfo dump")?;
        let stride = if d.framebuffer.stride == 0 {
            d.framebuffer.width
        } else {
            d.framebuffer.stride
        };
        let bgrt = d.bgrt;
        let logo = bgrt.as_ref().map(|b| {
            (
                b.offset_x as i64,
                b.offset_y as i64,
                b.offset_x as i64 + b.image_width as i64,
                b.offset_y as i64 + b.image_height as i64,
            )
        });
        Ok(Self {
            width: d.framebuffer.width,
            height: d.framebuffer.height,
            stride,
            format: d.framebuffer.format,
            logo,
            bgrt_status: bgrt.as_ref().map(|b| b.status).unwrap_or(0),
            bgrt_type: bgrt.as_ref().map(|b| b.image_type).unwrap_or(0),
            logo_warning: None,
        })
    }

    /// Live detection from `/sys`: the active console frame buffer and the
    /// ACPI BGRT logo. Errors loudly instead of inventing values.
    pub fn from_sysfs() -> Result<Self> {
        let fb = "/sys/class/graphics/fb0";
        let (width, height) = parse_virtual_size(
            &read_sysfs(&format!("{fb}/virtual_size"))
                .context("no live framebuffer (pass a BootInfo dump from the bootloader)")?,
        )?;
        let stride = read_sysfs(&format!("{fb}/stride"))
            .ok()
            .and_then(|s| s.trim().parse::<u32>().ok())
            .unwrap_or(width);
        let bpp = read_sysfs(&format!("{fb}/bits_per_pixel"))
            .ok()
            .and_then(|s| s.trim().parse::<u16>().ok())
            .unwrap_or(0);

        let (logo, bgrt_status, bgrt_type, logo_warning) = match bgrt_from_sysfs() {
            Ok(logo) => (logo.rect, logo.status, logo.image_type, None),
            Err(e) => (
                None,
                0,
                0,
                Some(format!("BGRT present but unusable: {e:#}")),
            ),
        };

        Ok(Self {
            width,
            height,
            stride,
            format: format!("{bpp}bpp"),
            logo,
            bgrt_status,
            bgrt_type,
            logo_warning,
        })
    }

    /// The `leon_*` context keys themes receive; only used by `preview`, which
    /// is Python-gated, so allow it to be dead when the python feature is off.
    #[cfg_attr(not(feature = "python"), allow(dead_code))]
    pub fn context(&self) -> HashMap<String, String> {
        let mut m = HashMap::new();
        m.insert("leon_fb_width".to_string(), self.width.to_string());
        m.insert("leon_fb_height".to_string(), self.height.to_string());
        m.insert("leon_fb_stride".to_string(), self.stride.to_string());
        m.insert("leon_fb_format".to_string(), self.format.clone());
        match self.logo {
            Some((x0, y0, x1, y1)) => {
                m.insert("leon_bgrt_rect".to_string(), format!("{x0},{y0},{x1},{y1}"));
                m.insert(
                    "leon_logo_center_x".to_string(),
                    ((x0 + x1) / 2).to_string(),
                );
                m.insert(
                    "leon_logo_center_y".to_string(),
                    ((y0 + y1) / 2).to_string(),
                );
            }
            None => {
                m.insert("leon_bgrt_rect".to_string(), "none".to_string());
                m.insert(
                    "leon_logo_center_x".to_string(),
                    (self.width / 2).to_string(),
                );
                m.insert(
                    "leon_logo_center_y".to_string(),
                    (self.height / 2).to_string(),
                );
            }
        }
        m.insert("leon_bgrt_status".to_string(), self.bgrt_status.to_string());
        m.insert("leon_bgrt_type".to_string(), self.bgrt_type.to_string());
        m
    }

    pub fn logo_text(&self) -> String {
        match self.logo {
            Some((x0, y0, x1, y1)) => format!(
                "BGRT logo {}x{} at ({},{}) [status {} type {}]",
                x1 - x0,
                y1 - y0,
                x0,
                y0,
                self.bgrt_status,
                self.bgrt_type
            ),
            None => "no BGRT logo".to_string(),
        }
    }

    pub fn report(&self) -> String {
        let mut s = format!(
            "framebuffer: {}x{} (stride {}, format {})\nlogo: {}",
            self.width,
            self.height,
            self.stride,
            self.format,
            self.logo_text()
        );
        if let Some(w) = &self.logo_warning {
            s.push_str(&format!("\nwarning: {w}"));
        }
        s
    }
}

/// Reads a sysfs attribute.
fn read_sysfs(path: &str) -> Result<String> {
    std::fs::read_to_string(path).with_context(|| format!("reading {path}"))
}

/// Parses `<width>,<height>` (or `WxH`) from a `virtual_size` attribute.
fn parse_virtual_size(s: &str) -> Result<(u32, u32)> {
    let s = s.trim();
    let (w, h) = s
        .split_once([',', 'x'])
        .ok_or_else(|| anyhow!("cannot parse virtual_size '{s}'"))?;
    let width = w
        .trim()
        .parse()
        .with_context(|| format!("bad width in '{s}'"))?;
    let height = h
        .trim()
        .parse()
        .with_context(|| format!("bad height in '{s}'"))?;
    Ok((width, height))
}

/// BGRT logo rectangle + status bytes discovered from ACPI sysfs.
struct LogoInfo {
    rect: Option<(i64, i64, i64, i64)>,
    status: u8,
    image_type: u8,
}

/// BGRT logo metadata from ACPI sysfs (offsets) + the BMP header (dimensions).
fn bgrt_from_sysfs() -> Result<LogoInfo> {
    let dir = Path::new("/sys/firmware/acpi/bgrt");
    if !dir.join("image").exists() {
        return Ok(LogoInfo {
            rect: None,
            status: 0,
            image_type: 0,
        });
    }
    let x: i32 = read_sysfs(&format!("{}/xoffset", dir.display()))?
        .trim()
        .parse()
        .context("parsing bgrt xoffset")?;
    let y: i32 = read_sysfs(&format!("{}/yoffset", dir.display()))?
        .trim()
        .parse()
        .context("parsing bgrt yoffset")?;
    let status: u8 = read_sysfs(&format!("{}/status", dir.display()))?
        .trim()
        .parse()
        .context("parsing bgrt status")?;
    let image_type: u8 = read_sysfs(&format!("{}/type", dir.display()))?
        .trim()
        .parse()
        .context("parsing bgrt type")?;
    // The logo image is raw binary BMP data, not text — `read_sysfs` would
    // choke on it.
    let image = std::fs::read(dir.join("image")).context("reading bgrt image")?;
    let (iw, ih) = bmp_dims(&image)?;
    let (x, y) = (x as i64, y as i64);
    Ok(LogoInfo {
        rect: Some((x, y, x + iw as i64, y + ih as i64)),
        status,
        image_type,
    })
}

/// Parses the pixel dimensions out of a BMP file header.
fn bmp_dims(bytes: &[u8]) -> Result<(u32, u32)> {
    if bytes.len() < 26 || &bytes[..2] != b"BM" {
        bail!("bgrt image is not a BMP");
    }
    let w = u32::from_le_bytes([bytes[18], bytes[19], bytes[20], bytes[21]]);
    let h = u32::from_le_bytes([bytes[22], bytes[23], bytes[24], bytes[25]]);
    if w == 0 || h == 0 {
        bail!("bgrt image has zero dimensions");
    }
    // Same sanity range the bootloader enforces for a boot logo.
    if w > 16384 || h > 16384 {
        bail!("bgrt image has implausible dimensions {w}x{h}");
    }
    Ok((w, h))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn geometry_context_has_fb_keys() {
        let json = r#"{
            "framebuffer": { "width": 1366, "height": 768, "stride": 5504, "format": "Bgrx" },
            "bgrt": null
        }"#;
        let path = std::env::temp_dir().join("lbt_test_fb.json");
        std::fs::write(&path, json).unwrap();
        let g = Geometry::from_dump(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        let ctx = g.context();
        assert_eq!(ctx.get("leon_fb_width").map(|s| s.as_str()), Some("1366"));
        assert_eq!(ctx.get("leon_fb_stride").map(|s| s.as_str()), Some("5504"));
        assert_eq!(ctx.get("leon_fb_format").map(|s| s.as_str()), Some("Bgrx"));
        assert_eq!(ctx.get("leon_bgrt_rect").map(|s| s.as_str()), Some("none"));
        assert_eq!(ctx.get("leon_bgrt_status").map(|s| s.as_str()), Some("0"));
        assert_eq!(ctx.get("leon_bgrt_type").map(|s| s.as_str()), Some("0"));
    }

    #[test]
    fn parses_virtual_size_forms() {
        assert_eq!(parse_virtual_size("1366,768\n").unwrap(), (1366, 768));
        assert_eq!(parse_virtual_size("1366x768").unwrap(), (1366, 768));
        assert!(parse_virtual_size("junk").is_err());
    }

    #[test]
    fn parses_bmp_dimensions() {
        let mut bmp = vec![0u8; 40];
        bmp[0] = b'B';
        bmp[1] = b'M';
        bmp[18..22].copy_from_slice(&548u32.to_le_bytes());
        bmp[22..26].copy_from_slice(&308u32.to_le_bytes());
        assert_eq!(bmp_dims(&bmp).unwrap(), (548, 308));
        assert!(bmp_dims(&[0u8; 10]).is_err());
        assert!(bmp_dims(&b"PNG...."[..]).is_err());
    }

    #[test]
    fn geometry_logo_rect_from_dump() {
        let json = r#"{
            "framebuffer": { "width": 3840, "height": 2160, "stride": 0, "format": "Bgrx" },
            "bgrt": { "offset_x": 1472, "offset_y": 880, "image_width": 896, "image_height": 400, "status": 1, "image_type": 0 }
        }"#;
        let path = std::env::temp_dir().join("lbt_test_dump.json");
        std::fs::write(&path, json).unwrap();
        let g = Geometry::load(Some(&path)).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(g.width, 3840);
        assert_eq!(g.stride, 3840);
        let ctx = g.context();
        assert_eq!(
            ctx.get("leon_bgrt_rect").map(|s| s.as_str()),
            Some("1472,880,2368,1280")
        );
        assert_eq!(ctx.get("leon_bgrt_status").map(|s| s.as_str()), Some("1"));
    }
}
