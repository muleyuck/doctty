use doctty::io::kitty_graphics::{
    DeleteCommand, KittyImagePlacement, encode_delete, encode_place, encode_support_query,
    encode_transmit_png, parse_support_reply,
};

#[test]
fn encodes_png_transfer_command() {
    let escape = encode_transmit_png(7, &[1, 2, 3, 4]);

    assert!(escape.contains("a=t"));
    assert!(escape.contains("f=100"));
    assert!(escape.contains("i=7"));
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

#[test]
fn support_query_asks_for_graphics_then_device_attributes() {
    assert_eq!(
        encode_support_query(),
        "\u{1b}_Gi=31,s=1,v=1,a=q,t=d,f=24;AAAA\u{1b}\\\u{1b}[c"
    );
}

#[test]
fn graphics_ok_before_device_attributes_means_supported() {
    assert_eq!(
        parse_support_reply(b"\x1b_Gi=31;OK\x1b\\\x1b[?62;22c"),
        Some(true)
    );
}

#[test]
fn device_attributes_alone_means_unsupported() {
    assert_eq!(parse_support_reply(b"\x1b[?62;22c"), Some(false));
    assert_eq!(parse_support_reply(b"\x1b[?1;2c"), Some(false));
}

#[test]
fn graphics_error_reply_means_unsupported() {
    assert_eq!(
        parse_support_reply(b"\x1b_Gi=31;ENOTSUPPORTED:no\x1b\\\x1b[?62c"),
        Some(false)
    );
}

#[test]
fn waits_until_device_attributes_reply_is_complete() {
    assert_eq!(parse_support_reply(b""), None);
    assert_eq!(parse_support_reply(b"\x1b_Gi=31;OK\x1b\\"), None);
    assert_eq!(parse_support_reply(b"\x1b_Gi=31;OK\x1b\\\x1b[?62;2"), None);
}

#[test]
fn ignores_replies_for_other_image_ids() {
    assert_eq!(
        parse_support_reply(b"\x1b_Gi=7;OK\x1b\\\x1b[?62c"),
        Some(false)
    );
}
