//! The reason a staff member states for a change (`x-audit-reason`, decoded), as every end of the
//! admin surface checks it: the admin partition before it signs it into the Staff context, the
//! Staff context itself ([`super::SignedContextIdentity::reason`]) and Core admin-gateway, which
//! records it.
//!
//! A reason is shown in audit pages, written to logs and handed to plugins, so it may hold no
//! character that changes how the text around it renders: control characters (Unicode `Cc`),
//! format characters (`Cf`: bidirectional embeddings, overrides and isolates, zero-width spaces
//! and joiners, the byte order mark, tag characters) and the line and paragraph separators
//! (`Zl`, `Zp`). Such a reason is refused, never repaired. Surrounding whitespace is trimmed
//! first; a reason with nothing visible left (only whitespace, combining marks, variation
//! selectors or blank fillers) states nothing and counts as missing.

/// Bytes of a stated reason (decoded UTF-8, before trimming).
pub const MAX_STATED_REASON_BYTES: usize = 512;

/// Why a stated reason is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatedReasonError {
    /// Nothing visible: empty, blank, or only characters that render as nothing on their own.
    Missing,
    /// Longer than [`MAX_STATED_REASON_BYTES`] or holding a control, format or separator
    /// character ([`hidden_character`]).
    Invalid,
}

/// Unicode format characters (general category `Cf`).
const FORMAT: &[(char, char)] = &[
    ('\u{00AD}', '\u{00AD}'),
    ('\u{0600}', '\u{0605}'),
    ('\u{061C}', '\u{061C}'),
    ('\u{06DD}', '\u{06DD}'),
    ('\u{070F}', '\u{070F}'),
    ('\u{0890}', '\u{0891}'),
    ('\u{08E2}', '\u{08E2}'),
    ('\u{180E}', '\u{180E}'),
    ('\u{200B}', '\u{200F}'),
    ('\u{202A}', '\u{202E}'),
    ('\u{2060}', '\u{2064}'),
    ('\u{2066}', '\u{206F}'),
    ('\u{FEFF}', '\u{FEFF}'),
    ('\u{FFF9}', '\u{FFFB}'),
    ('\u{110BD}', '\u{110BD}'),
    ('\u{110CD}', '\u{110CD}'),
    ('\u{13430}', '\u{1343F}'),
    ('\u{1BCA0}', '\u{1BCA3}'),
    ('\u{1D173}', '\u{1D17A}'),
    ('\u{E0001}', '\u{E0001}'),
    ('\u{E0020}', '\u{E007F}'),
];

/// Characters that render as nothing on their own: combining marks, variation selectors, the
/// combining grapheme joiner and the blank fillers.
const BLANK: &[(char, char)] = &[
    ('\u{0300}', '\u{036F}'),
    ('\u{115F}', '\u{1160}'),
    ('\u{1AB0}', '\u{1AFF}'),
    ('\u{1DC0}', '\u{1DFF}'),
    ('\u{20D0}', '\u{20FF}'),
    ('\u{2800}', '\u{2800}'),
    ('\u{3164}', '\u{3164}'),
    ('\u{FE00}', '\u{FE0F}'),
    ('\u{FE20}', '\u{FE2F}'),
    ('\u{FFA0}', '\u{FFA0}'),
    ('\u{E0100}', '\u{E01EF}'),
];

fn within(ranges: &[(char, char)], c: char) -> bool {
    ranges.iter().any(|(low, high)| (*low..=*high).contains(&c))
}

/// Whether `c` may not appear in a stated reason: a control character (`Cc`), a format
/// character (`Cf`) or the line or paragraph separator (`Zl`, `Zp`).
pub fn hidden_character(c: char) -> bool {
    c.is_control() || matches!(c, '\u{2028}' | '\u{2029}') || within(FORMAT, c)
}

fn visible(c: char) -> bool {
    !c.is_whitespace() && !hidden_character(c) && !within(BLANK, c)
}

/// The stated reason `text` trimmed of surrounding whitespace, when it is one: at most
/// [`MAX_STATED_REASON_BYTES`] bytes, no [`hidden_character`] once trimmed, at least one visible
/// character.
pub fn stated_reason(text: &str) -> Result<&str, StatedReasonError> {
    if text.len() > MAX_STATED_REASON_BYTES {
        return Err(StatedReasonError::Invalid);
    }
    let trimmed = text.trim();
    if trimmed.chars().any(hidden_character) {
        return Err(StatedReasonError::Invalid);
    }
    if !trimmed.chars().any(visible) {
        return Err(StatedReasonError::Missing);
    }
    Ok(trimmed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reason_is_visible_text_without_control_format_or_separator_characters() {
        for (text, expected) in [
            ("清理违规内容 ticket-9", "清理违规内容 ticket-9"),
            ("  spam wave \t", "spam wave"),
            ("e\u{0301}", "e\u{0301}"),
            ("\u{3000}关闭\u{3000}", "关闭"),
            ("close\n", "close"),
        ] {
            assert_eq!(stated_reason(text), Ok(expected), "{text:?}");
        }
        let longest = "r".repeat(MAX_STATED_REASON_BYTES);
        assert_eq!(stated_reason(&longest), Ok(longest.as_str()));
        assert_eq!(
            stated_reason(&format!("{longest}r")),
            Err(StatedReasonError::Invalid)
        );
        for hidden in [
            '\u{0000}',
            '\u{0001}',
            '\n',
            '\r',
            '\u{001B}',
            '\u{007F}',
            '\u{0085}',
            '\u{00AD}',
            '\u{061C}',
            '\u{180E}',
            '\u{200B}',
            '\u{200C}',
            '\u{200D}',
            '\u{200E}',
            '\u{200F}',
            '\u{202A}',
            '\u{202B}',
            '\u{202C}',
            '\u{202D}',
            '\u{202E}',
            '\u{2028}',
            '\u{2029}',
            '\u{2060}',
            '\u{2066}',
            '\u{2067}',
            '\u{2068}',
            '\u{2069}',
            '\u{FEFF}',
            '\u{E0001}',
            '\u{E0041}',
        ] {
            assert_eq!(
                stated_reason(&format!("close poll {hidden}9")),
                Err(StatedReasonError::Invalid),
                "{hidden:?}"
            );
        }
        // Reversing the rendered text around it is refused, not repaired.
        assert_eq!(
            stated_reason("refund \u{202E}001 resu"),
            Err(StatedReasonError::Invalid)
        );
    }

    #[test]
    fn a_reason_with_nothing_visible_is_missing() {
        for text in [
            "",
            " ",
            "\t \u{3000}",
            "\r\n",
            "\u{2028}",
            "\u{0301}\u{0302}",
            "\u{FE0F}",
            "\u{3164}\u{115F}",
            "\u{2800} ",
            "\u{034F}",
        ] {
            assert_eq!(
                stated_reason(text),
                Err(StatedReasonError::Missing),
                "{text:?}"
            );
        }
        // A zero-width "reason" is refused outright.
        assert_eq!(stated_reason("\u{200B}"), Err(StatedReasonError::Invalid));
    }
}
