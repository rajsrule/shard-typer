use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Debug)]
pub struct PreparedText {
    pub text: String,
    pub offsets: Vec<usize>,
}

impl PreparedText {
    pub fn new(raw: &str) -> Self {
        let text = raw
            .replace("\r\n", "\n")
            .replace('\r', "\n")
            .replace('\t', "    ");
        let mut offsets: Vec<_> = text.grapheme_indices(true).map(|(i, _)| i).collect();
        offsets.push(text.len());
        Self { text, offsets }
    }

    pub fn len(&self) -> usize {
        self.offsets.len() - 1
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn grapheme(&self, index: usize) -> Option<&str> {
        self.offsets
            .get(index + 1)
            .map(|end| &self.text[self.offsets[index]..*end])
    }
    pub fn remaining(&self, index: usize) -> &str {
        &self.text[self.offsets[index.min(self.len())]..]
    }
    pub fn preview(&self, index: usize) -> String {
        self.remaining(index)
            .split_whitespace()
            .take(6)
            .collect::<Vec<_>>()
            .join(" ")
    }
}

pub fn decode_text(bytes: &[u8]) -> Result<String, String> {
    if bytes.starts_with(&[0xff, 0xfe]) || bytes.starts_with(&[0xfe, 0xff]) {
        if !(bytes.len() - 2).is_multiple_of(2) {
            return Err("This UTF-16 file has an incomplete character.".into());
        }
        let le = bytes[0] == 0xff;
        let units: Vec<_> = bytes[2..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| {
                if le {
                    u16::from_le_bytes([c[0], c[1]])
                } else {
                    u16::from_be_bytes([c[0], c[1]])
                }
            })
            .collect();
        String::from_utf16(&units).map_err(|_| "The file contains invalid UTF-16 text.".into())
    } else {
        String::from_utf8(
            bytes
                .strip_prefix(&[0xef, 0xbb, 0xbf])
                .unwrap_or(bytes)
                .to_vec(),
        )
        .map_err(|_| "Please save this file as UTF-8 or UTF-16 text.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn prepares_lines_tabs_and_graphemes() {
        let p = PreparedText::new("a\r\nb\rc\t👩‍💻e\u{301}");
        assert_eq!(p.text, "a\nb\nc    👩‍💻e\u{301}");
        assert_eq!(p.len(), 11);
        assert_eq!(p.grapheme(9), Some("👩‍💻"));
        assert_eq!(p.grapheme(10), Some("e\u{301}"));
        assert_eq!(p.preview(9), "👩‍💻e\u{301}");
        assert_eq!(p.grapheme(11), None);
    }
    #[test]
    fn imports_bom_encodings() {
        assert_eq!(
            decode_text(&[0xff, 0xfe, 0x3d, 0xd8, 0x00, 0xde]).unwrap(),
            "😀"
        );
        assert_eq!(decode_text(&[0xfe, 0xff, 0, 65]).unwrap(), "A");
        assert_eq!(decode_text(b"\xef\xbb\xbfhello").unwrap(), "hello");
        assert!(decode_text(&[0xff, 0xfe, 0]).is_err());
        assert!(decode_text(&[0x80]).is_err());
    }
}
