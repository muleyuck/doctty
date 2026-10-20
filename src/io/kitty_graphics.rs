use base64::{Engine as _, engine::general_purpose::STANDARD};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct KittyImagePlacement {
    pub image_id: u32,
    pub placement_id: u32,
    pub columns: u16,
    pub rows: u16,
    pub source_x_px: u32,
    pub source_y_px: u32,
    pub source_width_px: u32,
    pub source_height_px: u32,
    pub cursor_x: u16,
    pub cursor_y: u16,
    pub z_index: i32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeleteCommand {
    AllVisiblePlacements,
    Placement { image_id: u32, placement_id: u32 },
}

// The Kitty protocol limits Base64 payloads to 4096 bytes per APC command.
// Large single-APC uploads can be discarded by terminal multiplexers.
const MAX_GRAPHICS_CHUNK_BYTES: usize = 4096;

#[must_use]
pub fn encode_transmit_png(image_id: u32, png_bytes: &[u8]) -> String {
    let payload = STANDARD.encode(png_bytes);
    if payload.is_empty() {
        return format!("\u{1b}_Ga=t,t=d,f=100,i={image_id},q=2,m=0;\u{1b}\\");
    }

    let mut commands = String::with_capacity(payload.len() + 64 * payload.len().div_ceil(4096));
    for (index, start) in (0..payload.len())
        .step_by(MAX_GRAPHICS_CHUNK_BYTES)
        .enumerate()
    {
        let end = start.saturating_add(MAX_GRAPHICS_CHUNK_BYTES).min(payload.len());
        let more = u8::from(end < payload.len());

        if index == 0 {
            commands.push_str(&format!(
                "\u{1b}_Ga=t,t=d,f=100,i={image_id},q=2,m={more};"
            ));
        } else {
            commands.push_str(&format!("\u{1b}_Gm={more};"));
        }
        // STANDARD Base64 is ASCII, so its byte boundaries are valid UTF-8 boundaries.
        commands.push_str(&payload[start..end]);
        commands.push_str("\u{1b}\\");
    }
    commands
}

#[must_use]
pub fn encode_place(placement: &KittyImagePlacement) -> String {
    format!(
        "\u{1b}_Ga=p,i={},p={},c={},r={},x={},y={},w={},h={},X={},Y={},z={},q=2,C=1\u{1b}\\",
        placement.image_id,
        placement.placement_id,
        placement.columns,
        placement.rows,
        placement.source_x_px,
        placement.source_y_px,
        placement.source_width_px,
        placement.source_height_px,
        placement.cursor_x,
        placement.cursor_y,
        placement.z_index,
    )
}

#[must_use]
pub fn encode_delete(command: DeleteCommand) -> String {
    match command {
        DeleteCommand::AllVisiblePlacements => "\u{1b}_Ga=d\u{1b}\\".to_string(),
        DeleteCommand::Placement {
            image_id,
            placement_id,
        } => {
            format!("\u{1b}_Ga=d,d=i,i={image_id},p={placement_id}\u{1b}\\")
        }
    }
}
