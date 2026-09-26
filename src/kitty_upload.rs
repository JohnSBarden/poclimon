//! Out-of-band Kitty image uploads.
//!
//! ratatui-image uploads a Kitty image by prefixing the transmit sequence to
//! the symbol of the image's top-left cell the *first* time the protocol is
//! rendered, and marks it as sent at that moment. If anything drawn later in
//! the same frame covers that cell — another creature, a nameplate, the toy —
//! the transmit is overwritten and never reaches the terminal. Every later
//! render only emits placeholders, so that animation frame shows up blank for
//! the rest of the session.
//!
//! To make uploads reliable we take the transmit sequence out of the protocol
//! ourselves (by rendering it once into a scratch buffer, which is the only
//! public way to get it) and write it straight to the terminal just before the
//! frame that first displays the image. The real render then only emits
//! placeholders, which are safe to overdraw.
//!
//! Images are also deleted from the terminal when the creature that owns them
//! goes away (swap, release, reload). Without that, Kitty/Ghostty keep every
//! image ever sent until their storage quota fills, then start evicting images
//! that are still on screen.

use ratatui::{buffer::Buffer, layout::Rect, style::Color, widgets::Widget};
use ratatui_image::{Image, protocol::Protocol};

/// Unicode placeholder character that Kitty uses to position images.
const PLACEHOLDER: char = '\u{10EEEE}';

/// A Kitty image transmit sequence that still has to be written to the terminal.
#[derive(Clone, Debug)]
pub struct KittyUpload {
    pub id: u32,
    pub seq: String,
}

/// Take the pending transmit sequence out of a Kitty protocol.
///
/// Returns `None` for non-Kitty protocols or if the image was already taken.
/// After this call, rendering `proto` only produces placeholders.
pub fn take_upload(proto: &Protocol) -> Option<KittyUpload> {
    if !matches!(proto, Protocol::Kitty(_)) {
        return None;
    }
    let size = proto.size();
    if size.width == 0 || size.height == 0 {
        return None;
    }
    let area = Rect::new(0, 0, size.width, size.height);
    let mut scratch = Buffer::empty(area);
    Image::new(proto).render(area, &mut scratch);
    let symbol = scratch[(0u16, 0u16)].symbol();
    let end = symbol.find(PLACEHOLDER)?;
    if end == 0 {
        return None; // already transmitted
    }
    let seq = symbol[..end].to_string();
    let id = parse_image_id(&seq)?;
    Some(KittyUpload { id, seq })
}

/// Extract `i=<id>` from a Kitty graphics command.
fn parse_image_id(seq: &str) -> Option<u32> {
    let start = seq.find("i=")? + 2;
    let digits: String = seq[start..]
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse().ok()
}

/// Kitty command that deletes an image *and its pixel data* from the terminal.
///
/// `q=2` suppresses the terminal's reply so nothing lands on stdin.
pub fn delete_command(id: u32, is_tmux: bool) -> String {
    let cmd = format!("\x1b_Ga=d,d=I,i={id},q=2\x1b\\");
    if is_tmux {
        // tmux passthrough: wrap and double every ESC inside.
        format!("\x1bPtmux;{}\x1b\\", cmd.replace('\x1b', "\x1b\x1b"))
    } else {
        cmd
    }
}

/// Where a Kitty image was placed this frame, recovered from the buffer
/// right after rendering it (the image id is carried in the cell colour).
#[derive(Clone, Copy, Debug)]
pub struct Placement {
    pub area: Rect,
    pub fg: Color,
    pub id_extra: char,
}

/// Record the placement of a Kitty image that was just rendered at `area`.
pub fn placement_at(buf: &Buffer, area: Rect, proto: &Protocol) -> Option<Placement> {
    if !matches!(proto, Protocol::Kitty(_)) {
        return None;
    }
    let size = proto.size();
    let area = Rect::new(
        area.x,
        area.y,
        area.width.min(size.width),
        area.height.min(size.height),
    )
    .intersection(buf.area);
    let cell = buf.cell((area.x, area.y))?;
    let mut chars = cell.symbol().chars();
    if chars.next()? != PLACEHOLDER {
        return None;
    }
    // Diacritics after the placeholder: row, column, id-extra.
    let id_extra = chars.nth(2).unwrap_or(DIACRITICS[0]);
    Some(Placement {
        area,
        fg: cell.fg,
        id_extra,
    })
}

/// Give explicit row/column diacritics to placeholder cells whose position
/// Kitty would otherwise infer wrongly.
///
/// ratatui-image writes the position only on the first cell of each image
/// row; Kitty infers every following cell from its left neighbour. When
/// something is drawn over the middle of a row (a nameplate, the toy, another
/// creature) the cells after the interruption have no valid neighbour and
/// Kitty shows the wrong slice of the image there.
pub fn fix_orphan_placeholders(buf: &mut Buffer, placements: &[Placement]) {
    if placements.is_empty() {
        return;
    }
    let area = buf.area;
    for y in area.top()..area.bottom() {
        let mut prev_fg: Option<Color> = None;
        for x in area.left()..area.right() {
            let cell = &buf[(x, y)];
            let sym = cell.symbol();
            if !sym.starts_with(PLACEHOLDER) {
                prev_fg = None;
                continue;
            }
            let fg = cell.fg;
            let bare = sym.chars().count() == 1;
            if bare && prev_fg != Some(fg) {
                // Topmost image at this cell with this id colour wins.
                if let Some(p) = placements
                    .iter()
                    .rev()
                    .find(|p| p.fg == fg && p.area.contains((x, y).into()))
                {
                    let row = (y - p.area.y) as usize;
                    let col = (x - p.area.x) as usize;
                    if row < DIACRITICS.len() && col < DIACRITICS.len() {
                        let mut s = String::with_capacity(16);
                        s.push(PLACEHOLDER);
                        s.push(DIACRITICS[row]);
                        s.push(DIACRITICS[col]);
                        s.push(p.id_extra);
                        buf[(x, y)].set_symbol(&s);
                    }
                }
            }
            prev_fg = Some(fg);
        }
    }
}

/// Row/column diacritics from Kitty's rowcolumn-diacritics.txt (same table
/// ratatui-image uses; it isn't exported).
static DIACRITICS: [char; 297] = [
    '\u{305}',
    '\u{30D}',
    '\u{30E}',
    '\u{310}',
    '\u{312}',
    '\u{33D}',
    '\u{33E}',
    '\u{33F}',
    '\u{346}',
    '\u{34A}',
    '\u{34B}',
    '\u{34C}',
    '\u{350}',
    '\u{351}',
    '\u{352}',
    '\u{357}',
    '\u{35B}',
    '\u{363}',
    '\u{364}',
    '\u{365}',
    '\u{366}',
    '\u{367}',
    '\u{368}',
    '\u{369}',
    '\u{36A}',
    '\u{36B}',
    '\u{36C}',
    '\u{36D}',
    '\u{36E}',
    '\u{36F}',
    '\u{483}',
    '\u{484}',
    '\u{485}',
    '\u{486}',
    '\u{487}',
    '\u{592}',
    '\u{593}',
    '\u{594}',
    '\u{595}',
    '\u{597}',
    '\u{598}',
    '\u{599}',
    '\u{59C}',
    '\u{59D}',
    '\u{59E}',
    '\u{59F}',
    '\u{5A0}',
    '\u{5A1}',
    '\u{5A8}',
    '\u{5A9}',
    '\u{5AB}',
    '\u{5AC}',
    '\u{5AF}',
    '\u{5C4}',
    '\u{610}',
    '\u{611}',
    '\u{612}',
    '\u{613}',
    '\u{614}',
    '\u{615}',
    '\u{616}',
    '\u{617}',
    '\u{657}',
    '\u{658}',
    '\u{659}',
    '\u{65A}',
    '\u{65B}',
    '\u{65D}',
    '\u{65E}',
    '\u{6D6}',
    '\u{6D7}',
    '\u{6D8}',
    '\u{6D9}',
    '\u{6DA}',
    '\u{6DB}',
    '\u{6DC}',
    '\u{6DF}',
    '\u{6E0}',
    '\u{6E1}',
    '\u{6E2}',
    '\u{6E4}',
    '\u{6E7}',
    '\u{6E8}',
    '\u{6EB}',
    '\u{6EC}',
    '\u{730}',
    '\u{732}',
    '\u{733}',
    '\u{735}',
    '\u{736}',
    '\u{73A}',
    '\u{73D}',
    '\u{73F}',
    '\u{740}',
    '\u{741}',
    '\u{743}',
    '\u{745}',
    '\u{747}',
    '\u{749}',
    '\u{74A}',
    '\u{7EB}',
    '\u{7EC}',
    '\u{7ED}',
    '\u{7EE}',
    '\u{7EF}',
    '\u{7F0}',
    '\u{7F1}',
    '\u{7F3}',
    '\u{816}',
    '\u{817}',
    '\u{818}',
    '\u{819}',
    '\u{81B}',
    '\u{81C}',
    '\u{81D}',
    '\u{81E}',
    '\u{81F}',
    '\u{820}',
    '\u{821}',
    '\u{822}',
    '\u{823}',
    '\u{825}',
    '\u{826}',
    '\u{827}',
    '\u{829}',
    '\u{82A}',
    '\u{82B}',
    '\u{82C}',
    '\u{82D}',
    '\u{951}',
    '\u{953}',
    '\u{954}',
    '\u{F82}',
    '\u{F83}',
    '\u{F86}',
    '\u{F87}',
    '\u{135D}',
    '\u{135E}',
    '\u{135F}',
    '\u{17DD}',
    '\u{193A}',
    '\u{1A17}',
    '\u{1A75}',
    '\u{1A76}',
    '\u{1A77}',
    '\u{1A78}',
    '\u{1A79}',
    '\u{1A7A}',
    '\u{1A7B}',
    '\u{1A7C}',
    '\u{1B6B}',
    '\u{1B6D}',
    '\u{1B6E}',
    '\u{1B6F}',
    '\u{1B70}',
    '\u{1B71}',
    '\u{1B72}',
    '\u{1B73}',
    '\u{1CD0}',
    '\u{1CD1}',
    '\u{1CD2}',
    '\u{1CDA}',
    '\u{1CDB}',
    '\u{1CE0}',
    '\u{1DC0}',
    '\u{1DC1}',
    '\u{1DC3}',
    '\u{1DC4}',
    '\u{1DC5}',
    '\u{1DC6}',
    '\u{1DC7}',
    '\u{1DC8}',
    '\u{1DC9}',
    '\u{1DCB}',
    '\u{1DCC}',
    '\u{1DD1}',
    '\u{1DD2}',
    '\u{1DD3}',
    '\u{1DD4}',
    '\u{1DD5}',
    '\u{1DD6}',
    '\u{1DD7}',
    '\u{1DD8}',
    '\u{1DD9}',
    '\u{1DDA}',
    '\u{1DDB}',
    '\u{1DDC}',
    '\u{1DDD}',
    '\u{1DDE}',
    '\u{1DDF}',
    '\u{1DE0}',
    '\u{1DE1}',
    '\u{1DE2}',
    '\u{1DE3}',
    '\u{1DE4}',
    '\u{1DE5}',
    '\u{1DE6}',
    '\u{1DFE}',
    '\u{20D0}',
    '\u{20D1}',
    '\u{20D4}',
    '\u{20D5}',
    '\u{20D6}',
    '\u{20D7}',
    '\u{20DB}',
    '\u{20DC}',
    '\u{20E1}',
    '\u{20E7}',
    '\u{20E9}',
    '\u{20F0}',
    '\u{2CEF}',
    '\u{2CF0}',
    '\u{2CF1}',
    '\u{2DE0}',
    '\u{2DE1}',
    '\u{2DE2}',
    '\u{2DE3}',
    '\u{2DE4}',
    '\u{2DE5}',
    '\u{2DE6}',
    '\u{2DE7}',
    '\u{2DE8}',
    '\u{2DE9}',
    '\u{2DEA}',
    '\u{2DEB}',
    '\u{2DEC}',
    '\u{2DED}',
    '\u{2DEE}',
    '\u{2DEF}',
    '\u{2DF0}',
    '\u{2DF1}',
    '\u{2DF2}',
    '\u{2DF3}',
    '\u{2DF4}',
    '\u{2DF5}',
    '\u{2DF6}',
    '\u{2DF7}',
    '\u{2DF8}',
    '\u{2DF9}',
    '\u{2DFA}',
    '\u{2DFB}',
    '\u{2DFC}',
    '\u{2DFD}',
    '\u{2DFE}',
    '\u{2DFF}',
    '\u{A66F}',
    '\u{A67C}',
    '\u{A67D}',
    '\u{A6F0}',
    '\u{A6F1}',
    '\u{A8E0}',
    '\u{A8E1}',
    '\u{A8E2}',
    '\u{A8E3}',
    '\u{A8E4}',
    '\u{A8E5}',
    '\u{A8E6}',
    '\u{A8E7}',
    '\u{A8E8}',
    '\u{A8E9}',
    '\u{A8EA}',
    '\u{A8EB}',
    '\u{A8EC}',
    '\u{A8ED}',
    '\u{A8EE}',
    '\u{A8EF}',
    '\u{A8F0}',
    '\u{A8F1}',
    '\u{AAB0}',
    '\u{AAB2}',
    '\u{AAB3}',
    '\u{AAB7}',
    '\u{AAB8}',
    '\u{AABE}',
    '\u{AABF}',
    '\u{AAC1}',
    '\u{FE20}',
    '\u{FE21}',
    '\u{FE22}',
    '\u{FE23}',
    '\u{FE24}',
    '\u{FE25}',
    '\u{FE26}',
    '\u{10A0F}',
    '\u{10A38}',
    '\u{1D185}',
    '\u{1D186}',
    '\u{1D187}',
    '\u{1D188}',
    '\u{1D189}',
    '\u{1D1AA}',
    '\u{1D1AB}',
    '\u{1D1AC}',
    '\u{1D1AD}',
    '\u{1D242}',
    '\u{1D243}',
    '\u{1D244}',
];

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui_image::{
        Resize,
        picker::{Picker, ProtocolType},
    };

    fn kitty_proto() -> Protocol {
        let mut picker = Picker::halfblocks();
        picker.set_protocol_type(ProtocolType::Kitty);
        let img = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            8,
            8,
            image::Rgba([255, 0, 0, 255]),
        ));
        picker
            .new_protocol(img, ratatui::layout::Size::new(4, 2), Resize::Scale(None))
            .expect("kitty protocol")
    }

    #[test]
    fn take_upload_returns_transmit_once() {
        let proto = kitty_proto();
        let up = take_upload(&proto).expect("first take yields the transmit");
        assert!(
            up.seq.contains("a=T"),
            "transmit sequence: {:?}",
            &up.seq[..40]
        );
        assert!(up.seq.contains(&format!("i={}", up.id)));
        assert!(take_upload(&proto).is_none(), "second take is empty");

        // After the take, a real render carries only placeholders — nothing
        // that could be lost by overdrawing.
        let area = Rect::new(0, 0, 4, 2);
        let mut buf = Buffer::empty(area);
        Image::new(&proto).render(area, &mut buf);
        assert!(buf[(0u16, 0u16)].symbol().starts_with(PLACEHOLDER));
    }

    #[test]
    fn non_kitty_protocols_have_no_upload() {
        let picker = Picker::halfblocks();
        let img = image::DynamicImage::ImageRgba8(image::RgbaImage::new(4, 4));
        let proto = picker
            .new_protocol(img, ratatui::layout::Size::new(2, 2), Resize::Scale(None))
            .unwrap();
        assert!(take_upload(&proto).is_none());
    }

    #[test]
    fn orphaned_cells_get_explicit_positions() {
        let proto = kitty_proto(); // 4x2 cells
        take_upload(&proto);
        let mut buf = Buffer::empty(Rect::new(0, 0, 10, 4));
        let area = Rect::new(2, 1, 4, 2);
        Image::new(&proto).render(area, &mut buf);
        let p = placement_at(&buf, area, &proto).expect("placement");
        // Overdraw the second column of the first row, like a nameplate would.
        buf[(3u16, 1u16)].set_symbol("X").set_fg(Color::Reset);
        fix_orphan_placeholders(&mut buf, &[p]);
        let cell = buf[(4u16, 1u16)].symbol().to_string();
        let chars: Vec<char> = cell.chars().collect();
        assert_eq!(chars[0], PLACEHOLDER);
        assert_eq!(chars[1], DIACRITICS[0], "row 0");
        assert_eq!(chars[2], DIACRITICS[2], "col 2 (x=4 minus area.x=2)");
        // A cell with an intact left neighbour stays bare (Kitty infers it).
        assert_eq!(buf[(5u16, 1u16)].symbol().chars().count(), 1);
    }

    #[test]
    fn delete_command_formats() {
        assert_eq!(delete_command(42, false), "\x1b_Ga=d,d=I,i=42,q=2\x1b\\");
        let t = delete_command(42, true);
        assert!(t.starts_with("\x1bPtmux;\x1b\x1b_G") && t.ends_with("\x1b\x1b\\\x1b\\"));
    }
}
