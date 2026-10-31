//! Low-resolution, non-WebKit preview images for the renderer viewport.
//!
//! A dedicated worker turns a bounded DOM outline into small SVG/PNG tiles.
//! Kitty retains at most a few decoded tiles; scrolling only re-places already
//! uploaded images. Neither SVG rasterization nor PNG transfer runs on the
//! terminal input thread.
use std::{
    collections::HashMap,
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, OnceLock,
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread,
};

use anyhow::{Context, Result, anyhow};
use base64::{Engine, engine::general_purpose::STANDARD};
use resvg::{
    tiny_skia::{IntSize, Pixmap, PixmapPaint, Transform},
    usvg::{self, Options},
};

pub(crate) const FIRST_PREVIEW_ID: u32 = 94_001;
pub(crate) const LAST_PREVIEW_ID: u32 = 94_004;
pub(crate) const MAX_PREVIEW_CACHE: usize = 3;
const SCALE: f64 = 0.5;
const CHUNK_SIZE: usize = 4096;

#[derive(Clone, Debug, Default)]
pub(crate) struct StyledItem {
    pub(crate) y: f64,
    pub(crate) x: f64,
    pub(crate) width: f64,
    pub(crate) height: f64,
    pub(crate) font_size: f64,
    pub(crate) line_height: f64,
    pub(crate) weight: u16,
    pub(crate) color: String,
    pub(crate) background: String,
    pub(crate) tag: String,
    pub(crate) text: String,
    /// Code block lines with their original breaks and indentation.
    pub(crate) lines: Vec<String>,
    pub(crate) padding_left: f64,
    pub(crate) padding_top: f64,
    /// Per code line, syntax colour runs as (character count, colour).
    pub(crate) runs: Vec<Vec<(usize, String)>>,
    /// Table cells, drawn as a grid instead of one flattened line.
    pub(crate) cells: Vec<TableCell>,
    /// Image source URL; only local `file://` images are drawn.
    pub(crate) src: String,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct TableCell {
    pub(crate) x: f64,
    pub(crate) y: f64,
    pub(crate) width: f64,
    pub(crate) height: f64,
    pub(crate) text: String,
    pub(crate) header: bool,
    pub(crate) padding_left: f64,
    /// CSS text-align: "center", "right"/"end", or anything else for left.
    pub(crate) align: String,
    pub(crate) background: String,
    pub(crate) border: String,
}

#[derive(Clone, Debug)]
pub(crate) struct Scene {
    pub(crate) background: String,
    pub(crate) items: Vec<StyledItem>,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Tile {
    pub(crate) id: u32,
    pub(crate) origin: f64,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

impl Tile {
    pub(crate) fn covers(self, position: f64, viewport_height: u32) -> bool {
        position >= self.origin - 0.5
            && position + f64::from(viewport_height)
                <= self.origin + f64::from(viewport_height) * 2.0 + 0.5
    }

    pub(crate) fn source_y(self, position: f64, viewport_height: u32) -> u32 {
        ((position - self.origin) * SCALE)
            .round()
            .max(0.0)
            .min(f64::from(self.height) - (f64::from(viewport_height) * SCALE).round())
            as u32
    }

    pub(crate) fn viewport_height_px(viewport_height: u32) -> u32 {
        (f64::from(viewport_height) * SCALE).round().max(1.0) as u32
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Request {
    pub(crate) origin: f64,
    pub(crate) image_id: u32,
    pub(crate) requested_at: f64,
}

pub(crate) enum Response {
    Ready(Tile),
    Skipped,
    Failed(String),
}

pub(crate) fn tile_origin(position: f64, viewport_height: u32, document_height: f64) -> f64 {
    let step = f64::from(viewport_height.max(1));
    let max_origin = (document_height - step * 2.0).max(0.0);
    ((position / step).floor() * step).clamp(0.0, max_origin)
}

pub(crate) fn select(cached: &[Tile], position: f64, viewport_height: u32) -> Option<Tile> {
    cached
        .iter()
        .copied()
        .find(|tile| tile.covers(position, viewport_height))
}

pub(crate) fn free_id(cached: &[Tile]) -> Option<u32> {
    (FIRST_PREVIEW_ID..=LAST_PREVIEW_ID).find(|id| cached.iter().all(|tile| tile.id != *id))
}

pub(crate) fn evict(
    cached: &mut Vec<Tile>,
    position: f64,
    viewport_height: u32,
    placed_id: Option<u32>,
) -> Option<Tile> {
    let index = cached
        .iter()
        .enumerate()
        .filter(|(_, tile)| Some(tile.id) != placed_id && !tile.covers(position, viewport_height))
        .max_by(|(_, a), (_, b)| {
            (a.origin - position)
                .abs()
                .total_cmp(&(b.origin - position).abs())
        })
        .map(|(index, _)| index)?;
    Some(cached.remove(index))
}

pub(crate) fn spawn(
    scene: Scene,
    viewport_width: u32,
    viewport_height: u32,
    latest_position: Arc<AtomicU64>,
    read_root: PathBuf,
) -> (Sender<Request>, Receiver<Response>, thread::JoinHandle<()>) {
    let (requests, incoming) = mpsc::channel::<Request>();
    let (outgoing, responses) = mpsc::channel::<Response>();
    let worker = thread::spawn(move || {
        let mut thumbnails = Thumbnails::new(&read_root);
        while let Ok(request) = incoming.recv() {
            let obsolete = || {
                let current = f64::from_bits(latest_position.load(Ordering::Relaxed));
                (current - request.requested_at).abs() > f64::from(viewport_height)
                    && !(current >= request.origin
                        && current + f64::from(viewport_height)
                            <= request.origin + f64::from(viewport_height) * 2.0)
            };
            let reply = if obsolete() {
                Response::Skipped
            } else {
                match render_png(
                    &scene,
                    request.origin,
                    viewport_width,
                    viewport_height,
                    &mut thumbnails,
                ) {
                    Ok((png, width, height)) => {
                        if obsolete() {
                            Response::Skipped
                        } else {
                            let uploaded = (|| -> Result<()> {
                                let mut out = io::stdout().lock();
                                upload_png(&mut out, request.image_id, &png)?;
                                out.flush()?;
                                Ok(())
                            })();
                            match uploaded {
                                Ok(()) => Response::Ready(Tile {
                                    id: request.image_id,
                                    origin: request.origin,
                                    width,
                                    height,
                                }),
                                Err(error) => Response::Failed(error.to_string()),
                            }
                        }
                    }
                    Err(error) => Response::Failed(error.to_string()),
                }
            };
            if outgoing.send(reply).is_err() {
                break;
            }
        }
    });
    (requests, responses, worker)
}

fn safe_color(input: &str, fallback: &str) -> String {
    if input.len() == 7
        && input.starts_with('#')
        && input[1..].chars().all(|c| c.is_ascii_hexdigit())
    {
        input.to_owned()
    } else {
        fallback.to_owned()
    }
}

fn xml_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c if c.is_control() => {}
            other => out.push(other),
        }
    }
    out
}

// Keep words together where possible and treat full-width script characters
// as wider than Latin characters. This remains an approximation of WebKit
// shaping, but avoids mid-word breaks and overcrowded Japanese preview lines.
fn approximate_char_units(ch: char) -> usize {
    if ch.is_ascii() { 1 } else { 2 }
}

fn preview_wrap_lines(text: &str, max_units: usize, max_lines: usize) -> Vec<String> {
    if max_units == 0 || max_lines == 0 {
        return Vec::new();
    }

    let chars: Vec<char> = text.chars().take(160).collect();
    let mut lines = Vec::new();
    let mut start = 0;
    while start < chars.len() && lines.len() < max_lines {
        while start < chars.len() && chars[start].is_whitespace() {
            start += 1;
        }
        if start >= chars.len() {
            break;
        }

        let mut end = start;
        let mut used_units = 0;
        while end < chars.len() {
            let char_units = approximate_char_units(chars[end]);
            if used_units + char_units > max_units {
                break;
            }
            used_units += char_units;
            end += 1;
        }
        if end == start {
            // Always make progress, even for a wide character in a tiny box.
            end += 1;
        }
        if end < chars.len() && !chars[end].is_whitespace() {
            if let Some(space) = (start + 1..end)
                .rev()
                .find(|&index| chars[index].is_whitespace())
            {
                end = space;
            }
        }
        let line: String = chars[start..end].iter().collect();
        let line = line.trim_end();
        if !line.is_empty() {
            lines.push(line.to_owned());
        }
        start = end;
    }
    lines
}

fn preview_svg(scene: &Scene, origin: f64, width: u32, height: u32) -> String {
    let full_height = height.saturating_mul(2);
    let background = safe_color(&scene.background, "#ffffff");
    let mut svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{full_height}" viewBox="0 0 {width} {full_height}"><rect width="{width}" height="{full_height}" fill="{background}"/>"#
    );
    let end = origin + f64::from(full_height);
    // A very long document can contain 12k outline entries. Only rasterize
    // the visible region, and cap the work for pathologically dense markup.
    for item in scene
        .items
        .iter()
        .filter(|item| item.y < end && item.y + item.height.max(1.0) >= origin)
        .take(240)
    {
        let x = item.x.clamp(0.0, f64::from(width.saturating_sub(1)));
        let y = item.y - origin;
        let item_width = item.width.clamp(1.0, f64::from(width) - x);
        let font_size = item.font_size.clamp(8.0, 42.0);
        let line_height = item.line_height.max(font_size * 1.15).min(70.0);
        let item_height = item.height.clamp(line_height, 1000.0);
        let color = safe_color(&item.color, "#1f2328");
        let fill = safe_color(&item.background, &background);
        if item.tag == "pre" && !item.lines.is_empty() {
            // Code is drawn to the block's real height, so its background
            // must cover the whole visible part rather than the 1000 px clamp.
            let top = y.max(-1.0);
            let bottom = (y + item.height).min(f64::from(full_height) + 1.0);
            let visible = (bottom - top).max(0.0);
            svg.push_str(&format!(
                r#"<rect x="{x:.1}" y="{top:.1}" width="{item_width:.1}" height="{visible:.1}" rx="4" fill="{fill}"/>"#
            ));
        } else if item.tag == "table" && !item.cells.is_empty() {
            // Cells paint their own backgrounds and borders.
        } else if item.tag == "pre" || item.tag == "table" {
            svg.push_str(&format!(
                r#"<rect x="{x:.1}" y="{y:.1}" width="{item_width:.1}" height="{item_height:.1}" rx="4" fill="{fill}"/>"#
            ));
        } else if item.tag == "img" {
            svg.push_str(&format!(
                r#"<rect x="{x:.1}" y="{y:.1}" width="{item_width:.1}" height="{item_height:.1}" rx="4" fill="{fill}" stroke="{color}" stroke-opacity="0.25"/>"#
            ));
            continue;
        }

        if item.tag == "pre" && !item.lines.is_empty() {
            push_code_lines(
                &mut svg,
                item,
                x,
                y,
                item_width,
                f64::from(full_height),
                &color,
            );
            continue;
        }
        if item.tag == "table" && !item.cells.is_empty() {
            push_table_cells(&mut svg, item, origin, f64::from(full_height), &color);
            continue;
        }

        let weight = item.weight.clamp(300, 800);
        let font = if item.tag == "pre" {
            "Menlo"
        } else {
            "Helvetica"
        };
        let clean = item.text.trim_start_matches('#').trim();
        if clean.is_empty() {
            continue;
        }
        let max_lines = ((item_height / line_height).ceil() as usize).clamp(1, 8);
        let per_line = (item_width / (font_size * 0.52)).floor().max(8.0) as usize;
        for (index, line) in preview_wrap_lines(clean, per_line, max_lines)
            .into_iter()
            .enumerate()
        {
            let baseline = y + font_size + f64::from(index as u32) * line_height;
            if baseline > f64::from(full_height) {
                break;
            }
            let text = xml_escape(&line);
            svg.push_str(&format!(
                r#"<text x="{x:.1}" y="{baseline:.1}" fill="{color}" font-family="{font}" font-weight="{weight}" font-size="{font_size:.1}">{text}</text>"#
            ));
        }
    }
    svg.push_str("</svg>");
    svg
}

// Monospaced lines are drawn unwrapped and clipped at the block edge, as
// WebKit scrolls an overlong <pre> line instead of wrapping it.
fn push_code_lines(
    svg: &mut String,
    item: &StyledItem,
    x: f64,
    y: f64,
    width: f64,
    full_height: f64,
    color: &str,
) {
    let font_size = item.font_size.clamp(8.0, 42.0);
    let line_height = item.line_height.max(font_size * 1.15).min(70.0);
    let padding_left = item.padding_left.clamp(0.0, width / 2.0);
    let top = y + item.padding_top.clamp(0.0, 200.0);
    // Menlo advances about 0.6 em per character.
    let max_chars = ((width - 2.0 * padding_left) / (font_size * 0.6))
        .floor()
        .max(1.0) as usize;
    for (index, line) in item.lines.iter().enumerate() {
        let line_top = top + index as f64 * line_height;
        if line_top >= full_height || line_top >= y + item.height {
            break;
        }
        if line_top + line_height <= 0.0 || line.trim().is_empty() {
            continue;
        }
        let baseline = line_top + (line_height + font_size * 0.7) / 2.0;
        svg.push_str(&format!(
            r#"<text x="{:.1}" y="{baseline:.1}" fill="{color}" font-family="Menlo" font-size="{font_size:.1}" xml:space="preserve">"#,
            x + padding_left
        ));
        let mut chars = line.chars().take(max_chars);
        for (count, run_color) in item.runs.get(index).into_iter().flatten() {
            let segment: String = chars.by_ref().take(*count).collect();
            if segment.is_empty() {
                break;
            }
            if run_color.is_empty() {
                svg.push_str(&xml_escape(&segment));
            } else {
                svg.push_str(&format!(
                    r#"<tspan fill="{}">{}</tspan>"#,
                    safe_color(run_color, color),
                    xml_escape(&segment)
                ));
            }
        }
        svg.push_str(&xml_escape(&chars.collect::<String>()));
        svg.push_str("</text>");
    }
}

fn push_table_cells(
    svg: &mut String,
    item: &StyledItem,
    origin: f64,
    full_height: f64,
    color: &str,
) {
    let font_size = item.font_size.clamp(8.0, 42.0);
    for cell in &item.cells {
        let y = cell.y - origin;
        if y >= full_height || y + cell.height <= 0.0 {
            continue;
        }
        // A transparent border arrives empty: draw no grid line for it.
        let border = if cell.border.is_empty() {
            "none".to_owned()
        } else {
            safe_color(&cell.border, "#3d444d")
        };
        let fill = if cell.background.is_empty() {
            "none".to_owned()
        } else {
            safe_color(&cell.background, "#0d1117")
        };
        svg.push_str(&format!(
            r#"<rect x="{:.1}" y="{y:.1}" width="{:.1}" height="{:.1}" fill="{fill}" stroke="{border}"/>"#,
            cell.x, cell.width, cell.height
        ));
        let padding = cell.padding_left.clamp(0.0, cell.width / 2.0);
        // Helvetica averages about 0.45 em per character in table text.
        let max_chars = ((cell.width - 2.0 * padding) / (font_size * 0.45))
            .floor()
            .max(1.0) as usize;
        let clipped: String = cell.text.chars().take(max_chars).collect();
        if clipped.is_empty() {
            continue;
        }
        let baseline = y + (cell.height + font_size * 0.7) / 2.0;
        let weight = if cell.header { 600 } else { 400 };
        let (anchor, text_x) = match cell.align.as_str() {
            "center" => ("middle", cell.x + cell.width / 2.0),
            "right" | "end" => ("end", cell.x + cell.width - padding),
            _ => ("start", cell.x + padding),
        };
        svg.push_str(&format!(
            r#"<text x="{text_x:.1}" y="{baseline:.1}" fill="{color}" font-family="Helvetica" font-weight="{weight}" font-size="{font_size:.1}" text-anchor="{anchor}">{}</text>"#,
            xml_escape(&clipped)
        ));
    }
}

/// Downscaled local images, decoded once per source and size. Re-decoding a
/// photo or a large GIF for every preview tile would dominate its render time.
pub(crate) struct Thumbnails {
    // Only files WebKit itself may read are decoded, so a document cannot
    // surface a private image through the preview.
    root: Option<PathBuf>,
    cache: HashMap<(String, u32, u32), Option<Pixmap>>,
    cached_bytes: usize,
}

const MAX_THUMBNAIL_SOURCE_BYTES: u64 = 32 * 1024 * 1024;
const MAX_THUMBNAIL_CACHE_BYTES: usize = 64 * 1024 * 1024;

impl Thumbnails {
    pub(crate) fn new(read_root: &Path) -> Self {
        Self {
            root: std::fs::canonicalize(read_root).ok(),
            cache: HashMap::new(),
            cached_bytes: 0,
        }
    }

    fn get(&mut self, src: &str, width: u32, height: u32) -> Option<&Pixmap> {
        let key = (src.to_owned(), width, height);
        if !self.cache.contains_key(&key) {
            let thumbnail = self
                .root
                .as_deref()
                .and_then(|root| load_thumbnail(root, src, width, height));
            let size = thumbnail.as_ref().map_or(0, |pixmap| pixmap.data().len());
            if self.cached_bytes + size > MAX_THUMBNAIL_CACHE_BYTES {
                self.cache.clear();
                self.cached_bytes = 0;
            }
            self.cached_bytes += size;
            self.cache.insert(key.clone(), thumbnail);
        }
        self.cache.get(&key)?.as_ref()
    }
}

fn local_file(src: &str) -> Option<PathBuf> {
    let url = src.strip_prefix("file://")?;
    let url = url.strip_prefix("localhost").unwrap_or(url);
    let bytes = url.split(['?', '#']).next()?.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let hex = std::str::from_utf8(bytes.get(index + 1..index + 3)?).ok()?;
            decoded.push(u8::from_str_radix(hex, 16).ok()?);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    Some(PathBuf::from(String::from_utf8(decoded).ok()?))
}

fn load_thumbnail(root: &Path, src: &str, width: u32, height: u32) -> Option<Pixmap> {
    let path = std::fs::canonicalize(local_file(src)?).ok()?;
    let metadata = std::fs::metadata(&path).ok()?;
    // A FIFO or device would block the worker; outside the root is off limits.
    if !path.starts_with(root) || !metadata.is_file() || metadata.len() > MAX_THUMBNAIL_SOURCE_BYTES
    {
        return None;
    }
    let image = image::ImageReader::open(&path)
        .ok()?
        .with_guessed_format()
        .ok()?
        .decode()
        .ok()?;
    let mut pixels = image
        .resize_exact(width, height, image::imageops::FilterType::Triangle)
        .into_rgba8()
        .into_raw();
    // tiny-skia stores premultiplied alpha.
    for pixel in pixels.as_chunks_mut::<4>().0 {
        let alpha = u16::from(pixel[3]);
        for channel in &mut pixel[..3] {
            *channel = ((u16::from(*channel) * alpha + 127) / 255) as u8;
        }
    }
    Pixmap::from_vec(pixels, IntSize::from_wh(width, height)?)
}

fn font_database() -> Arc<usvg::fontdb::Database> {
    static DATABASE: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();
    Arc::clone(DATABASE.get_or_init(|| {
        let mut database = usvg::fontdb::Database::new();
        database.load_system_fonts();
        Arc::new(database)
    }))
}

/// Pixel size of one preview tile for a viewport, shared with the renderer's
/// terminal image budget.
pub(crate) fn raster_size(width: u32, height: u32) -> (u32, u32) {
    (width.div_ceil(2).clamp(1, 2048), height.clamp(1, 4096))
}

fn render_png(
    scene: &Scene,
    origin: f64,
    width: u32,
    height: u32,
    thumbnails: &mut Thumbnails,
) -> Result<(Vec<u8>, u32, u32)> {
    let (out_width, out_height) = raster_size(width, height);
    let svg = preview_svg(scene, origin, width, height);
    let options = Options {
        font_family: "Helvetica".to_string(),
        fontdb: font_database(),
        ..Options::default()
    };
    let tree = usvg::Tree::from_str(&svg, &options).context("invalid preview SVG")?;
    let mut pixmap = Pixmap::new(out_width, out_height)
        .ok_or_else(|| anyhow!("cannot allocate preview image"))?;
    resvg::render(
        &tree,
        Transform::from_scale(
            out_width as f32 / width.max(1) as f32,
            out_height as f32 / height.max(1).saturating_mul(2) as f32,
        ),
        &mut pixmap.as_mut(),
    );
    let scale_x = f64::from(out_width) / f64::from(width.max(1));
    let scale_y = f64::from(out_height) / f64::from(height.max(1).saturating_mul(2));
    let end = origin + f64::from(height.saturating_mul(2));
    for item in scene
        .items
        .iter()
        .filter(|item| item.tag == "img" && item.y < end && item.y + item.height > origin)
    {
        let thumb_width = (item.width * scale_x)
            .round()
            .clamp(1.0, f64::from(out_width)) as u32;
        let thumb_height = (item.height * scale_y).round().clamp(1.0, 4096.0) as u32;
        if let Some(thumbnail) = thumbnails.get(&item.src, thumb_width, thumb_height) {
            pixmap.draw_pixmap(
                (item.x * scale_x).round() as i32,
                ((item.y - origin) * scale_y).round() as i32,
                thumbnail.as_ref(),
                &PixmapPaint::default(),
                Transform::identity(),
                None,
            );
        }
    }
    Ok((encode_png(&pixmap)?, out_width, out_height))
}

// The preview is opaque (the page background is painted first), so its
// premultiplied pixels are already straight RGBA. Fast compression keeps
// photos from dominating the render time.
fn encode_png(pixmap: &Pixmap) -> Result<Vec<u8>> {
    use image::{
        ExtendedColorType, ImageEncoder,
        codecs::png::{CompressionType, FilterType, PngEncoder},
    };
    let mut png = Vec::new();
    PngEncoder::new_with_quality(&mut png, CompressionType::Fast, FilterType::Adaptive)
        .write_image(
            pixmap.data(),
            pixmap.width(),
            pixmap.height(),
            ExtendedColorType::Rgba8,
        )?;
    Ok(png)
}

fn upload_png(out: &mut impl Write, id: u32, png: &[u8]) -> Result<()> {
    let data = STANDARD.encode(png);
    let chunks = data.len().div_ceil(CHUNK_SIZE);
    for (index, chunk) in data.as_bytes().chunks(CHUNK_SIZE).enumerate() {
        let more = u8::from(index + 1 < chunks);
        if index == 0 {
            write!(out, "\x1b_Ga=t,t=d,f=100,i={id},q=2,m={more};")?;
        } else {
            write!(out, "\x1b_Gm={more};")?;
        }
        out.write_all(chunk)?;
        out.write_all(b"\x1b\\")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chooses_bounded_preview_tiles_and_protects_displayed_tile() {
        let mut tiles = vec![
            Tile {
                id: FIRST_PREVIEW_ID,
                origin: 0.0,
                width: 400,
                height: 1000,
            },
            Tile {
                id: FIRST_PREVIEW_ID + 1,
                origin: 1000.0,
                width: 400,
                height: 1000,
            },
            Tile {
                id: FIRST_PREVIEW_ID + 2,
                origin: 2000.0,
                width: 400,
                height: 1000,
            },
        ];
        let removed = evict(&mut tiles, 1700.0, 1000, Some(FIRST_PREVIEW_ID));
        assert_eq!(removed.map(|t| t.origin), Some(2000.0));
        assert_eq!(tiles.len(), 2);
        assert!(tiles.iter().any(|t| t.id == FIRST_PREVIEW_ID));
    }

    #[test]
    fn tile_geometry_stays_within_the_uploaded_image() {
        let viewport = 1000;
        let tile = Tile {
            id: FIRST_PREVIEW_ID,
            origin: 1000.0,
            width: 400,
            height: 1000,
        };
        assert!(!tile.covers(999.0, viewport));
        assert!(tile.covers(1000.0, viewport));
        assert!(tile.covers(2000.0, viewport));
        assert!(!tile.covers(2001.0, viewport));
        assert_eq!(tile.source_y(1000.0, viewport), 0);
        assert_eq!(tile.source_y(1500.0, viewport), 250);
        assert_eq!(tile.source_y(2000.0, viewport), 500);
        assert_eq!(Tile::viewport_height_px(viewport), 500);
        assert_eq!(tile_origin(49000.0, viewport, 50000.0), 48000.0);
        assert_eq!(tile_origin(0.0, viewport, 1000.0), 0.0);
    }

    #[test]
    fn preview_cache_remains_bounded_across_repeated_long_scrolls() {
        use std::collections::BTreeSet;

        let viewport = 1000;
        let document_height = 50000.0;
        let mut cached = vec![Tile {
            id: FIRST_PREVIEW_ID,
            origin: 0.0,
            width: 400,
            height: 1000,
        }];
        let mut displayed_id = Some(FIRST_PREVIEW_ID);
        let mut evicted_count = 0;

        // Model instantaneous worker completion. Repeated bidirectional
        // sweeps exercise cache admission, eviction, and image-ID reuse.
        for _ in 0..3 {
            for step in (0..=196).chain((0..=196).rev()) {
                let position = f64::from(step * 250);
                if let Some(tile) = select(&cached, position, viewport) {
                    displayed_id = Some(tile.id);
                } else {
                    if cached.len() == MAX_PREVIEW_CACHE {
                        if let Some(old) = evict(&mut cached, position, viewport, displayed_id) {
                            assert_ne!(Some(old.id), displayed_id);
                            assert!(!old.covers(position, viewport));
                            evicted_count += 1;
                        }
                    }
                    if cached.len() < MAX_PREVIEW_CACHE {
                        let id = free_id(&cached)
                            .unwrap_or_else(|| panic!("preview image ID exhausted"));
                        let origin = tile_origin(position, viewport, document_height);
                        assert!(
                            cached.iter().all(|tile| tile.origin != origin),
                            "duplicate preview tile"
                        );
                        cached.push(Tile {
                            id,
                            origin,
                            width: 400,
                            height: 1000,
                        });
                    }
                }

                assert!(cached.len() <= MAX_PREVIEW_CACHE);
                let ids: BTreeSet<_> = cached.iter().map(|tile| tile.id).collect();
                assert_eq!(ids.len(), cached.len(), "duplicate Kitty IDs");
                assert!(
                    ids.iter()
                        .all(|id| { (FIRST_PREVIEW_ID..=LAST_PREVIEW_ID).contains(id) })
                );
            }
        }
        assert!(evicted_count > 10, "stress test must exercise eviction");
    }

    #[test]
    fn long_outline_rasterizes_only_nearby_document_items() {
        let items = (0..12000)
            .map(|index| StyledItem {
                y: f64::from(index) * 24.0,
                x: 8.0,
                width: 600.0,
                height: 20.0,
                font_size: 14.0,
                line_height: 20.0,
                text: format!("entry-{index:05}"),
                ..StyledItem::default()
            })
            .collect();
        let scene = Scene {
            background: "#0d1117".to_owned(),
            items,
        };
        let svg = preview_svg(&scene, 120000.0, 800, 480);
        assert!(svg.contains("entry-05000"));
        assert!(!svg.contains("entry-00000"));
        assert!(!svg.contains("entry-11999"));
        assert!(svg.matches("<text ").count() <= 42);
        assert!(
            svg.len() < 20000,
            "SVG size should not grow with document length"
        );
    }

    #[test]
    fn preview_wrapping_prefers_word_boundaries_and_wide_text() {
        assert_eq!(
            preview_wrap_lines("Hello world from WebKit", 11, 3),
            vec!["Hello world", "from WebKit"]
        );
        assert_eq!(
            preview_wrap_lines("日本語の文章", 6, 3),
            vec!["日本語", "の文章"]
        );
        assert_eq!(
            preview_wrap_lines("Supercalifragilistic", 6, 2),
            vec!["Superc", "alifra"]
        );
        assert!(preview_wrap_lines("hello", 0, 2).is_empty());
        assert_eq!(preview_wrap_lines("日本語", 1, 3), vec!["日", "本", "語"]);
    }

    #[test]
    fn code_preview_keeps_lines_and_indentation_and_stops_at_the_block() {
        let item = StyledItem {
            y: 100.0,
            x: 20.0,
            width: 400.0,
            height: 16.0 + 3.0 * 20.0,
            font_size: 13.6,
            line_height: 20.0,
            tag: "pre".to_owned(),
            text: "[code] fn main() {}".to_owned(),
            lines: vec![
                "fn main() {".to_owned(),
                String::new(),
                "    let x = 1 < 2;".to_owned(),
                "}".to_owned(),
                "hidden".to_owned(),
            ],
            padding_left: 16.0,
            padding_top: 16.0,
            ..StyledItem::default()
        };
        let scene = Scene {
            background: "#0d1117".to_owned(),
            items: vec![item],
        };
        let svg = preview_svg(&scene, 0.0, 800, 400);
        assert!(svg.contains(r#"xml:space="preserve">    let x = 1 &lt; 2;</text>"#));
        assert_eq!(
            svg.matches("<text").count(),
            2,
            "blank and overflowing lines are skipped"
        );
        assert!(!svg.contains("hidden"));
        assert!(
            svg.contains(r#"x="36.0""#),
            "text starts after the block padding"
        );
    }

    #[test]
    fn code_preview_colors_each_syntax_run() {
        let item = StyledItem {
            y: 0.0,
            x: 0.0,
            width: 600.0,
            height: 60.0,
            font_size: 13.6,
            line_height: 20.0,
            tag: "pre".to_owned(),
            text: "[code] fn main".to_owned(),
            lines: vec!["fn main() {}".to_owned()],
            runs: vec![vec![
                (2, "#ff7b72".to_owned()),
                (1, String::new()),
                (4, "#d2a8ff".to_owned()),
                (5, String::new()),
            ]],
            padding_top: 16.0,
            ..StyledItem::default()
        };
        let scene = Scene {
            background: "#0d1117".to_owned(),
            items: vec![item],
        };
        let svg = preview_svg(&scene, 0.0, 800, 400);
        assert!(svg.contains(
            r##"xml:space="preserve"><tspan fill="#ff7b72">fn</tspan> <tspan fill="#d2a8ff">main</tspan>() {}</text>"##
        ));
    }

    #[test]
    fn code_color_runs_escape_markup_and_stop_at_the_block_width() {
        let item = StyledItem {
            y: 0.0,
            x: 0.0,
            // 0.6 em * 10 px = 6 px per character: 60 px holds 10 characters.
            width: 60.0,
            height: 60.0,
            font_size: 10.0,
            line_height: 20.0,
            tag: "pre".to_owned(),
            text: "[code] a<b".to_owned(),
            lines: vec!["a<b && c_overflowing".to_owned()],
            runs: vec![vec![
                (3, "#ff7b72".to_owned()),
                (40, "not-a-color".to_owned()),
            ]],
            ..StyledItem::default()
        };
        let scene = Scene {
            background: "#0d1117".to_owned(),
            items: vec![item],
        };
        let svg = preview_svg(&scene, 0.0, 800, 400);
        assert!(svg.contains(r##"<tspan fill="#ff7b72">a&lt;b</tspan>"##));
        assert!(svg.contains(r##"<tspan fill="#1f2328"> &amp;&amp; c_o</tspan></text>"##));
        assert!(!svg.contains("verflowing"));
    }

    #[test]
    fn code_color_runs_keep_alignment_for_short_missing_and_wide_runs() {
        let item = StyledItem {
            y: 0.0,
            x: 0.0,
            width: 600.0,
            height: 100.0,
            font_size: 10.0,
            line_height: 20.0,
            tag: "pre".to_owned(),
            text: "[code] x".to_owned(),
            lines: vec![
                "abcdef".to_owned(),
                "日本🙂ab".to_owned(),
                "no runs here".to_owned(),
            ],
            // The third line has no run list at all.
            runs: vec![
                vec![(2, "#ff7b72".to_owned())],
                vec![
                    (2, "#d2a8ff".to_owned()),
                    (1, String::new()),
                    (2, "#79c0ff".to_owned()),
                ],
            ],
            ..StyledItem::default()
        };
        let scene = Scene {
            background: "#0d1117".to_owned(),
            items: vec![item],
        };
        let svg = preview_svg(&scene, 0.0, 800, 400);
        assert!(
            svg.contains(r##"<tspan fill="#ff7b72">ab</tspan>cdef</text>"##),
            "characters past a short run list keep the text colour"
        );
        assert!(
            svg.contains(
                r##"<tspan fill="#d2a8ff">日本</tspan>🙂<tspan fill="#79c0ff">ab</tspan></text>"##
            ),
            "runs count characters, not bytes"
        );
        assert!(svg.contains(r#"xml:space="preserve">no runs here</text>"#));
    }

    #[test]
    fn tall_code_block_background_covers_every_visible_line() {
        let item = StyledItem {
            y: 0.0,
            x: 8.0,
            width: 600.0,
            height: 32.0 + 100.0 * 20.0,
            font_size: 13.6,
            line_height: 20.0,
            background: "#161b22".to_owned(),
            tag: "pre".to_owned(),
            text: "[code] long".to_owned(),
            lines: (0..100).map(|index| format!("line {index}")).collect(),
            padding_left: 16.0,
            padding_top: 16.0,
            ..StyledItem::default()
        };
        let scene = Scene {
            background: "#0d1117".to_owned(),
            items: vec![item],
        };
        // The 1200 px tile shows lines well past the old 1000 px rect clamp.
        let svg = preview_svg(&scene, 0.0, 800, 600);
        assert!(svg.contains("line 58"));
        assert!(svg.contains(r#"y="0.0" width="600.0" height="1201.0""#));
    }

    #[test]
    fn table_preview_draws_each_cell_with_its_alignment() {
        let cell = |x: f64, text: &str, align: &str| TableCell {
            x,
            y: 40.0,
            width: 100.0,
            height: 36.0,
            text: text.to_owned(),
            padding_left: 13.0,
            align: align.to_owned(),
            border: "#3d444d".to_owned(),
            ..TableCell::default()
        };
        let scene = Scene {
            background: "#0d1117".to_owned(),
            items: vec![StyledItem {
                y: 40.0,
                x: 0.0,
                width: 300.0,
                height: 36.0,
                font_size: 16.0,
                tag: "table".to_owned(),
                text: "[table] a 1 b".to_owned(),
                cells: vec![
                    cell(0.0, "a", ""),
                    cell(100.0, "1", "right"),
                    cell(200.0, "b", "center"),
                ],
                ..StyledItem::default()
            }],
        };
        let svg = preview_svg(&scene, 0.0, 400, 200);
        assert_eq!(svg.matches(r##"stroke="#3d444d""##).count(), 3);
        assert!(svg.contains(r#"x="13.0""#) && svg.contains(r#"text-anchor="start">a<"#));
        assert!(svg.contains(r#"x="187.0""#) && svg.contains(r#"text-anchor="end">1<"#));
        assert!(svg.contains(r#"x="250.0""#) && svg.contains(r#"text-anchor="middle">b<"#));
        assert!(
            !svg.contains("[table]"),
            "cells replace the flattened table text"
        );
    }

    #[test]
    fn local_image_is_drawn_from_a_cached_thumbnail() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("red dot.png");
        image::RgbaImage::from_pixel(4, 4, image::Rgba([255, 0, 0, 255])).save(&path)?;
        let src = format!("file://{}", path.display()).replace(' ', "%20");
        assert_eq!(local_file(&src).as_deref(), Some(path.as_path()));
        assert_eq!(local_file("https://example.com/a.png"), None);

        let scene = Scene {
            background: "#ffffff".to_owned(),
            items: vec![StyledItem {
                y: 0.0,
                x: 0.0,
                width: 200.0,
                height: 200.0,
                tag: "img".to_owned(),
                text: "[image] dot".to_owned(),
                src,
                ..StyledItem::default()
            }],
        };
        let mut thumbnails = Thumbnails::new(dir.path());
        let (png, _, _) = render_png(&scene, 0.0, 400, 200, &mut thumbnails)?;
        let decoded = image::load_from_memory(&png)?.into_rgba8();
        assert_eq!(decoded.get_pixel(10, 10).0, [255, 0, 0, 255]);
        assert_eq!(thumbnails.cache.len(), 1);
        render_png(&scene, 0.0, 400, 200, &mut thumbnails)?;
        assert_eq!(thumbnails.cache.len(), 1, "the thumbnail is decoded once");

        // The same image is off limits once it lies outside the read root.
        let other_root = tempfile::tempdir()?;
        let mut confined = Thumbnails::new(other_root.path());
        let (png, _, _) = render_png(&scene, 0.0, 400, 200, &mut confined)?;
        let decoded = image::load_from_memory(&png)?.into_rgba8();
        assert_ne!(decoded.get_pixel(10, 10).0, [255, 0, 0, 255]);
        Ok(())
    }

    #[test]
    fn outline_svg_escapes_markup_and_matches_palette() {
        let scene = Scene {
            background: "#0d1117".to_owned(),
            items: vec![StyledItem {
                x: 16.0,
                y: 50.0,
                width: 300.0,
                height: 48.0,
                font_size: 24.0,
                line_height: 28.0,
                weight: 700,
                color: "#f0f6fc".to_owned(),
                tag: "h1".to_owned(),
                text: "Hello <Rust> & WebKit".to_owned(),
                ..StyledItem::default()
            }],
        };
        let xml = preview_svg(&scene, 0.0, 400, 1000);
        assert!(xml.contains("fill=\"#0d1117\""));
        assert!(xml.contains("fill=\"#f0f6fc\""));
        assert!(xml.contains("&lt;Rust&gt; &amp; WebKit"));
        assert_eq!(tile_origin(1800.0, 1000, 10_000.0), 1000.0);
    }
}
