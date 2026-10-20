use base64::{Engine as _, engine::general_purpose::STANDARD};

use doctty::io::kitty_graphics::{
    DeleteCommand, KittyImagePlacement, encode_delete, encode_place, encode_transmit_png,
};

#[test]
fn encodes_png_transfer_command() {
    let escape = encode_transmit_png(7, &[1, 2, 3, 4]);

    assert!(escape.contains("a=t"));
    assert!(escape.contains("f=100"));
    assert!(escape.contains("i=7"));
    assert!(escape.contains("m=0;AQIDBA=="));
}

#[test]
fn chunks_large_png_transfers_with_correct_continuation_markers() {
    for byte_count in [0, 1, 3072, 3073, 6144, 10_001] {
        let input: Vec<u8> = (0..byte_count).map(|index| (index % 251) as u8).collect();
        let encoded = encode_transmit_png(31, &input);
        let frames: Vec<&str> = encoded.split_terminator("\u{1b}\\").collect();

        assert!(!frames.is_empty());
        let mut reconstructed = String::new();
        for (index, frame) in frames.iter().enumerate() {
            let (control, chunk) = frame
                .strip_prefix("\u{1b}_G")
                .and_then(|value| value.split_once(';'))
                .unwrap_or_else(|| panic!("frame must be a complete Kitty APC"));
            let final_frame = index + 1 == frames.len();
            let more = if final_frame { 0 } else { 1 };

            if index == 0 {
                assert_eq!(control, format!("a=t,t=d,f=100,i=31,q=2,m={more}"));
            } else {
                assert_eq!(control, format!("m={more}"));
            }

            assert!(
                chunk.len() <= 4096,
                "Kitty payload exceeds the 4096-byte limit"
            );
            if !final_frame {
                assert_eq!(chunk.len(), 4096);
                assert_eq!(chunk.len() % 4, 0);
            }
            reconstructed.push_str(chunk);
        }

        let decoded = STANDARD
            .decode(reconstructed)
            .unwrap_or_else(|error| panic!("Base64 should decode: {error}"));
        assert_eq!(decoded, input);
    }
}

#[test]
fn encodes_image_placement_command() {
    let placement = KittyImagePlacement {
        image_id: 7,
        placement_id: 3,
        columns: 12,
        rows: 4,
        source_x_px: 11,
        source_y_px: 13,
        source_width_px: 101,
        source_height_px: 79,
        cursor_x: 2,
        cursor_y: 5,
        z_index: -1,
    };

    let escape = encode_place(&placement);

    assert!(escape.contains("a=p"));
    assert!(escape.contains("i=7"));
    assert!(escape.contains("p=3"));
    assert!(escape.contains("c=12"));
    assert!(escape.contains("r=4"));
    assert!(escape.contains("x=11"));
    assert!(escape.contains("y=13"));
    assert!(escape.contains("w=101"));
    assert!(escape.contains("h=79"));
    assert!(escape.contains("X=2"));
    assert!(escape.contains("Y=5"));
}

#[test]
fn encodes_delete_visible_placements_command() {
    let escape = encode_delete(DeleteCommand::AllVisiblePlacements);
    assert_eq!(escape, "\u{1b}_Ga=d\u{1b}\\");
}

#[test]
fn encodes_delete_specific_placement_command() {
    let escape = encode_delete(DeleteCommand::Placement {
        image_id: 7,
        placement_id: 3,
    });
    assert_eq!(escape, "\u{1b}_Ga=d,d=i,i=7,p=3\u{1b}\\");
}
