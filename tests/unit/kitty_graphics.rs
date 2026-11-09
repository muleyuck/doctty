use doctty::io::kitty_graphics::{encode_support_query, parse_support_reply};

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
