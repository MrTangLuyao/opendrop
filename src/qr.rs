//! QR code as a compact SVG (EC level M, quiet zone of 2 modules).

use std::fmt::Write as _;

use qrcode::{Color, EcLevel, QrCode};

/// Longest text accepted, in bytes (spec: 512 characters; counting bytes
/// keeps multi byte text from forcing the largest, slowest QR versions).
pub const MAX_BYTES: usize = 512;
const QUIET: usize = 2;
const DARK: &str = "#0E1A2B";

/// SVG for `text`, or `None` when it does not fit in a QR code.
pub fn svg(text: &str) -> Option<String> {
    let code = QrCode::with_error_correction_level(text.as_bytes(), EcLevel::M).ok()?;
    let width = code.width();
    let colors = code.to_colors();
    let size = width + 2 * QUIET;
    let mut path = String::with_capacity(width * width);
    for (y, row) in colors.chunks(width.max(1)).enumerate() {
        let mut x = 0;
        while x < row.len() {
            if row.get(x) == Some(&Color::Dark) {
                let start = x;
                while row.get(x) == Some(&Color::Dark) {
                    x += 1;
                }
                let run = x - start;
                let _ = write!(path, "M{} {}h{run}v1h-{run}z", start + QUIET, y + QUIET);
            } else {
                x += 1;
            }
        }
    }
    Some(format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {size} {size}\" \
         shape-rendering=\"crispEdges\"><rect width=\"{size}\" height=\"{size}\" fill=\"#fff\"/>\
         <path fill=\"{DARK}\" d=\"{path}\"/></svg>"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders() {
        let s = svg("https://drop.example.com/d/123456").expect("qr");
        assert!(s.starts_with("<svg"));
        assert!(s.contains("crispEdges"));
        assert!(s.contains(DARK));
        // version 3 (29 modules) + 2 * 2 quiet zone
        assert!(s.contains("viewBox=\"0 0 33 33\""), "{s}");
        assert!(svg(&"x".repeat(MAX_BYTES)).is_some());
        assert!(svg(&"界".repeat(MAX_BYTES / 3)).is_some());
    }
}
