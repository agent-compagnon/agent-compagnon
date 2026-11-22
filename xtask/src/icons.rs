//! `cargo xtask icons`: draws the icons of the application and of the notification area into `app/icons/`.
//!
//! Simple shapes, anti-aliased by supersampling, encoded by hand (PNG with stored deflate blocks, ICO holding PNG
//! images) so that xtask keeps no dependency. Each badge has a white rim: readable on a dark taskbar as on a light
//! one, where the dark or coloured disc carries the contrast.

use std::fs;
use std::io;
use std::path::Path;

type Rgba = [u8; 4];

const WHITE: Rgba = [255, 255, 255, 255];
const NEUTRAL: Rgba = [38, 38, 38, 255];
const ATTENTION: Rgba = [242, 140, 0, 255];
const ERROR: Rgba = [217, 45, 32, 255];
const SUSPENDED: Rgba = [128, 128, 128, 255];
const INK: Rgba = [26, 26, 26, 255];

#[derive(Clone, Copy)]
enum Mark {
    /// A dot in the middle.
    Dot(Rgba),
    /// A cross.
    Cross(Rgba),
    /// Two vertical bars: paused.
    Pause(Rgba),
}

struct Badge {
    fill: Rgba,
    mark: Mark,
}

const BADGES: [(&str, Badge); 4] = [
    (
        "tray-neutral.png",
        Badge {
            fill: NEUTRAL,
            mark: Mark::Dot(WHITE),
        },
    ),
    (
        "tray-attention.png",
        Badge {
            fill: ATTENTION,
            mark: Mark::Dot(INK),
        },
    ),
    (
        "tray-error.png",
        Badge {
            fill: ERROR,
            mark: Mark::Cross(WHITE),
        },
    ),
    (
        "tray-suspended.png",
        Badge {
            fill: SUSPENDED,
            mark: Mark::Pause(WHITE),
        },
    ),
];

const TRAY_SIZE: u32 = 32;
const APP_SIZES: [u32; 4] = [16, 32, 48, 256];
const SUPERSAMPLING: u32 = 4;

/// Distance from `p` to the segment `a`–`b`.
fn segment_distance(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let t = (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
    let (x, y) = (a.0 + t * dx, a.1 + t * dy);
    ((p.0 - x).powi(2) + (p.1 - y).powi(2)).sqrt()
}

/// Colour of one point of the unit badge (coordinates from -1 to 1), or `None` outside.
fn sample(badge: &Badge, x: f64, y: f64) -> Option<Rgba> {
    let r = (x * x + y * y).sqrt();
    if r > 0.96 {
        return None;
    }
    if r > 0.80 {
        return Some(WHITE);
    }
    let inside = match badge.mark {
        Mark::Dot(colour) => (r <= 0.32).then_some(colour),
        Mark::Cross(colour) => {
            let arm = 0.36;
            let d = segment_distance((x, y), (-arm, -arm), (arm, arm)).min(segment_distance(
                (x, y),
                (-arm, arm),
                (arm, -arm),
            ));
            (d <= 0.13).then_some(colour)
        }
        Mark::Pause(colour) => {
            let bar = |centre: f64| (x - centre).abs() <= 0.1 && y.abs() <= 0.38;
            (bar(-0.19) || bar(0.19)).then_some(colour)
        }
    };
    Some(inside.unwrap_or(badge.fill))
}

fn render(badge: &Badge, size: u32) -> Vec<u8> {
    let mut pixels = Vec::with_capacity((size * size * 4) as usize);
    let n = f64::from(size * SUPERSAMPLING);
    for py in 0..size {
        for px in 0..size {
            let mut sum = [0u32; 4];
            for sy in 0..SUPERSAMPLING {
                for sx in 0..SUPERSAMPLING {
                    let x = (f64::from(px * SUPERSAMPLING + sx) + 0.5) / n * 2.0 - 1.0;
                    let y = (f64::from(py * SUPERSAMPLING + sy) + 0.5) / n * 2.0 - 1.0;
                    if let Some(colour) = sample(badge, x, y) {
                        // Colours of the covered samples, averaged below.
                        for (total, value) in sum.iter_mut().zip(colour) {
                            *total += u32::from(value);
                        }
                    }
                }
            }
            let samples = SUPERSAMPLING * SUPERSAMPLING;
            let covered = sum[3] / 255;
            for channel in &sum[..3] {
                pixels.push(channel.checked_div(covered).unwrap_or(0) as u8);
            }
            pixels.push((sum[3] / samples) as u8);
        }
    }
    pixels
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &b in bytes {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

fn adler32(bytes: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in bytes {
        a = (a + u32::from(byte)) % 65_521;
        b = (b + a) % 65_521;
    }
    (b << 16) | a
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let crc = crc32(&out[start..]);
    out.extend_from_slice(&crc.to_be_bytes());
}

/// An RGBA PNG, zlib stream of stored (uncompressed) blocks.
pub fn png(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    let mut raw = Vec::with_capacity(rgba.len() + height as usize);
    for row in rgba.chunks(width as usize * 4) {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    let mut zlib = vec![0x78, 0x01];
    let blocks: Vec<&[u8]> = raw.chunks(65_535).collect();
    for (i, block) in blocks.iter().enumerate() {
        zlib.push(u8::from(i + 1 == blocks.len()));
        let len = block.len() as u16;
        zlib.extend_from_slice(&len.to_le_bytes());
        zlib.extend_from_slice(&(!len).to_le_bytes());
        zlib.extend_from_slice(block);
    }
    zlib.extend_from_slice(&adler32(&raw).to_be_bytes());

    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut header = Vec::new();
    header.extend_from_slice(&width.to_be_bytes());
    header.extend_from_slice(&height.to_be_bytes());
    header.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &header);
    chunk(&mut out, b"IDAT", &zlib);
    chunk(&mut out, b"IEND", &[]);
    out
}

/// An ICO file whose images are PNGs (accepted by Windows since Vista).
fn ico(images: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let mut out = vec![0, 0, 1, 0];
    out.extend_from_slice(&(images.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * images.len() as u32;
    for (size, data) in images {
        let side = if *size >= 256 { 0 } else { *size as u8 };
        out.extend_from_slice(&[side, side, 0, 0]);
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&32u16.to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset += data.len() as u32;
    }
    for (_, data) in images {
        out.extend_from_slice(data);
    }
    out
}

/// Writes every icon into `dir`.
pub fn write_all(dir: &Path) -> io::Result<Vec<String>> {
    fs::create_dir_all(dir)?;
    let mut written = Vec::new();
    for (name, badge) in &BADGES {
        fs::write(dir.join(name), png(TRAY_SIZE, TRAY_SIZE, &render(badge, TRAY_SIZE)))?;
        written.push((*name).to_owned());
    }
    let neutral = &BADGES[0].1;
    let images: Vec<(u32, Vec<u8>)> = APP_SIZES
        .iter()
        .map(|&size| (size, png(size, size, &render(neutral, size))))
        .collect();
    fs::write(dir.join("icon.ico"), ico(&images))?;
    written.push("icon.ico".to_owned());
    if let Some((_, data)) = images.iter().find(|(size, _)| *size == TRAY_SIZE) {
        fs::write(dir.join("icon.png"), data)?;
        written.push("icon.png".to_owned());
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_structure() {
        let pixels = render(&BADGES[2].1, 8);
        assert_eq!(pixels.len(), 8 * 8 * 4);
        let file = png(8, 8, &pixels);
        assert!(file.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert_eq!(&file[12..16], b"IHDR");
        assert!(file.ends_with(&[0xae, 0x42, 0x60, 0x82]), "IEND with its CRC");
        assert_eq!(crc32(b"IEND"), 0xae42_6082);
        // Corners are transparent, the centre is opaque.
        assert_eq!(pixels[3], 0);
        let centre = ((4 * 8 + 4) * 4 + 3) as usize;
        assert_eq!(pixels[centre], 255);
    }

    #[test]
    fn generated_icons_are_up_to_date() {
        let dir = std::env::temp_dir().join(format!("mcp-proxy-icons-{}", std::process::id()));
        let names = write_all(&dir).unwrap();
        let committed = Path::new(env!("CARGO_MANIFEST_DIR")).join("../app/icons");
        for name in names {
            let fresh = fs::read(dir.join(&name)).unwrap();
            let current = fs::read(committed.join(&name)).unwrap_or_default();
            assert!(fresh == current, "app/icons/{name} is stale: run `cargo xtask icons`");
        }
        let _ = fs::remove_dir_all(&dir);
    }
}
