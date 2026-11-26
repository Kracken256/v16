pub const SURROGATE_ESCAPE_BASE: u32 = 0x100000;
pub const SURROGATE_ESCAPE_LIMIT: u32 = SURROGATE_ESCAPE_BASE + 0x800;

#[inline]
pub fn is_surrogate_escape(c: char) -> bool {
    (SURROGATE_ESCAPE_BASE..SURROGATE_ESCAPE_LIMIT).contains(&(c as u32))
}

#[inline]
pub fn surrogate_escape_char(code_unit: u32) -> char {
    char::from_u32(SURROGATE_ESCAPE_BASE + (code_unit - 0xD800)).unwrap()
}

#[inline]
pub fn surrogate_escape_unit(c: char) -> Option<u16> {
    let value = c as u32;
    if (SURROGATE_ESCAPE_BASE..SURROGATE_ESCAPE_LIMIT).contains(&value) {
        Some((0xD800 + (value - SURROGATE_ESCAPE_BASE)) as u16)
    } else {
        None
    }
}

#[inline]
pub fn char_for_code_unit(code_unit: u32) -> char {
    if (0xD800..=0xDFFF).contains(&code_unit) {
        surrogate_escape_char(code_unit)
    } else {
        char::from_u32(code_unit).unwrap_or('\u{FFFD}')
    }
}

pub struct CodeUnits<'a> {
    chars: std::str::Chars<'a>,
    pending: Option<u16>,
}

impl Iterator for CodeUnits<'_> {
    type Item = u16;

    #[inline]
    fn next(&mut self) -> Option<u16> {
        if let Some(unit) = self.pending.take() {
            return Some(unit);
        }
        let c = self.chars.next()?;
        if let Some(unit) = surrogate_escape_unit(c) {
            return Some(unit);
        }
        let mut buf = [0u16; 2];
        let units = c.encode_utf16(&mut buf);
        if units.len() == 2 {
            self.pending = Some(units[1]);
        }
        Some(units[0])
    }
}

#[inline]
pub fn code_units(s: &str) -> CodeUnits<'_> {
    CodeUnits {
        chars: s.chars(),
        pending: None,
    }
}

#[inline]
pub fn code_unit_len(s: &str) -> usize {
    code_units(s).count()
}

#[inline]
pub fn code_unit_at(s: &str, index: usize) -> Option<u16> {
    code_units(s).nth(index)
}

pub fn canonicalize_surrogate_pairs(s: &str) -> Option<String> {
    if !s.as_bytes().contains(&0xF4) {
        return None;
    }
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if let Some(unit) = surrogate_escape_unit(c)
            && (0xD800..=0xDBFF).contains(&unit)
            && let Some(&next) = chars.peek()
            && let Some(low) = surrogate_escape_unit(next)
            && (0xDC00..=0xDFFF).contains(&low)
        {
            let code_point = 0x10000 + ((unit as u32 - 0xD800) << 10) + (low as u32 - 0xDC00);
            out.push(char::from_u32(code_point).unwrap_or('\u{FFFD}'));
            chars.next();
            continue;
        }
        out.push(c);
    }
    if out == s { None } else { Some(out) }
}

pub fn string_from_code_units(units: &[u16]) -> String {
    let mut out = String::with_capacity(units.len());
    let mut i = 0;
    while i < units.len() {
        let unit = units[i];
        if (0xD800..=0xDBFF).contains(&unit)
            && i + 1 < units.len()
            && (0xDC00..=0xDFFF).contains(&units[i + 1])
        {
            let code_point =
                0x10000 + ((unit as u32 - 0xD800) << 10) + (units[i + 1] as u32 - 0xDC00);
            out.push(char::from_u32(code_point).unwrap_or('\u{FFFD}'));
            i += 2;
        } else {
            out.push(char_for_code_unit(unit as u32));
            i += 1;
        }
    }
    out
}
