use super::ast::{Ast, CharClass, ParseState, Quantifier};
use super::charclass::{CharRange, class_digit, class_space, class_word};

pub fn parse(pattern: &str, flags: u16) -> Result<Ast, String> {
    let mut state = ParseState::new(pattern, flags);
    let ast = parse_disjunction(&mut state)?;

    if !state.is_eof() {
        return Err(format!("Unexpected character at position {}", state.pos));
    }

    for backref in &state.named_backrefs {
        if !state.named_groups.contains(backref) {
            return Err(format!("Invalid named capture referenced: {}", backref));
        }
    }

    Ok(ast.simplify())
}

fn parse_disjunction(state: &mut ParseState) -> Result<Ast, String> {
    let mut alternatives = vec![parse_alternative(state)?];

    while state.peek() == Some('|') {
        state.next();
        alternatives.push(parse_alternative(state)?);
    }

    if alternatives.len() == 1 {
        Ok(alternatives.into_iter().next().unwrap())
    } else {
        Ok(Ast::Alt(alternatives))
    }
}

fn parse_alternative(state: &mut ParseState) -> Result<Ast, String> {
    let mut terms = Vec::new();

    while let Some(c) = state.peek() {
        if c == '|' || c == ')' {
            break;
        }
        terms.push(parse_term(state)?);
    }

    if terms.is_empty() {
        Ok(Ast::Empty)
    } else if terms.len() == 1 {
        Ok(terms.into_iter().next().unwrap())
    } else {
        Ok(Ast::Concat(terms))
    }
}

fn parse_term(state: &mut ParseState) -> Result<Ast, String> {
    let atom = parse_atom(state)?;

    if let Some(q) = parse_quantifier(state)? {
        if state.is_unicode
            && matches!(atom, Ast::Lookahead(_) | Ast::NegativeLookahead(_))
        {
            return Err("Quantifiable assertion is not allowed in unicode mode".to_string());
        }
        Ok(Ast::Quant(Box::new(atom), q))
    } else {
        Ok(atom)
    }
}

fn parse_quantifier(state: &mut ParseState) -> Result<Option<Quantifier>, String> {
    let c = match state.peek() {
        Some(c) if "*+?{".contains(c) => c,
        _ => return Ok(None),
    };

    let q = match c {
        '*' => {
            state.next();
            Quantifier::star()
        }
        '+' => {
            state.next();
            Quantifier::plus()
        }
        '?' => {
            state.next();
            Quantifier::question()
        }
        '{' => {
            let start = state.pos;
            match parse_brace_quantifier(state)? {
                Some(q) => q,
                None => {
                    if state.is_unicode {
                        return Err("Incomplete quantifier".to_string());
                    }
                    state.pos = start;
                    return Ok(None);
                }
            }
        }
        _ => unreachable!(),
    };

    let q = if state.peek() == Some('?') {
        state.next();
        Quantifier { greedy: false, ..q }
    } else {
        q
    };

    Ok(Some(q))
}

fn parse_brace_quantifier(state: &mut ParseState) -> Result<Option<Quantifier>, String> {
    state.expect('{')?;

    let min = match state.parse_number() {
        Some(n) => n,
        None => return Ok(None),
    };

    let max = if state.peek() == Some(',') {
        state.next();
        if state.peek() == Some('}') {
            None
        } else {
            match state.parse_number() {
                Some(n) => Some(n),
                None => return Ok(None),
            }
        }
    } else {
        Some(min)
    };

    if state.peek() != Some('}') {
        return Ok(None);
    }
    state.next();

    if let Some(max_val) = max {
        if min > max_val {
            return Err("Invalid quantifier: min > max".to_string());
        }
    }

    Ok(Some(Quantifier::range(min, max)))
}

fn parse_atom(state: &mut ParseState) -> Result<Ast, String> {
    match state.peek() {
        None => Err("Unexpected end of pattern".to_string()),
        Some('^') => {
            state.next();
            Ok(Ast::StartOfLine)
        }
        Some('$') => {
            state.next();
            Ok(Ast::EndOfLine)
        }
        Some('.') => {
            state.next();
            if state.dot_all {
                Ok(Ast::AnyAll)
            } else {
                Ok(Ast::Any)
            }
        }
        Some('\\') => parse_escape(state),
        Some('(') => parse_group(state),
        Some('[') => parse_char_class(state),
        Some('{') if !state.is_unicode => {
            if is_braced_quantifier_shape(state) {
                Err("Nothing to repeat".to_string())
            } else {
                state.next();
                Ok(Ast::Char('{'))
            }
        }
        Some(']') | Some('}') if !state.is_unicode => {
            let c = state.next().unwrap();
            Ok(Ast::Char(c))
        }
        Some(c) if is_special_char(c) => Err(format!(
            "Unexpected special character '{}' at position {}",
            c, state.pos
        )),
        Some(_) => {
            let c = state.next().unwrap();
            Ok(Ast::Char(c))
        }
    }
}

fn is_braced_quantifier_shape(state: &ParseState) -> bool {
    let pattern = &state.pattern;
    let mut pos = state.pos + 1;

    if pos >= pattern.len() || !pattern[pos].is_ascii_digit() {
        return false;
    }
    while pos < pattern.len() && pattern[pos].is_ascii_digit() {
        pos += 1;
    }

    if pos >= pattern.len() {
        return false;
    }
    if pattern[pos] == '}' {
        return true;
    }
    if pattern[pos] != ',' {
        return false;
    }
    pos += 1;

    if pos < pattern.len() && pattern[pos] == '}' {
        return true;
    }
    if pos >= pattern.len() || !pattern[pos].is_ascii_digit() {
        return false;
    }
    while pos < pattern.len() && pattern[pos].is_ascii_digit() {
        pos += 1;
    }
    pos < pattern.len() && pattern[pos] == '}'
}

fn parse_escape(state: &mut ParseState) -> Result<Ast, String> {
    state.expect('\\')?;

    let c = state
        .next()
        .ok_or_else(|| "Unexpected end after \\".to_string())?;

    match c {
        'b' => Ok(Ast::Char('\x08')),
        'f' => Ok(Ast::Char('\x0C')),
        'n' => Ok(Ast::Char('\n')),
        'r' => Ok(Ast::Char('\r')),
        't' => Ok(Ast::Char('\t')),
        'v' => Ok(Ast::Char('\x0B')),
        'd' => Ok(Ast::Class(CharClass {
            negated: false,
            ranges: class_digit(),
        })),
        'D' => Ok(Ast::Class(CharClass {
            negated: true,
            ranges: class_digit(),
        })),
        's' => Ok(Ast::Class(CharClass {
            negated: false,
            ranges: class_space(),
        })),
        'S' => Ok(Ast::Class(CharClass {
            negated: true,
            ranges: class_space(),
        })),
        'w' => Ok(Ast::Class(CharClass {
            negated: false,
            ranges: class_word(),
        })),
        'W' => Ok(Ast::Class(CharClass {
            negated: true,
            ranges: class_word(),
        })),
        'B' => Ok(Ast::NotWordBoundary),
        'x' => parse_hex_escape(state, 2),
        'u' => parse_unicode_escape(state),
        '0' => {
            if state.is_unicode {
                if state.peek().map_or(false, |c| c.is_ascii_digit()) {
                    return Err("Invalid octal escape in unicode mode".to_string());
                }
                Ok(Ast::Char('\0'))
            } else {
                parse_octal_escape(state)
            }
        }
        '1'..='9' => {
            let start = state.pos - 1;
            let mut num = c.to_digit(10).unwrap();
            while let Some(d) = state.peek().and_then(|ch| ch.to_digit(10)) {
                num = num.wrapping_mul(10).wrapping_add(d);
                state.next();
            }

            if num > 0 && num < state.capture_count as u32 {
                Ok(Ast::BackRef(num as usize))
            } else if !state.is_unicode {
                state.pos = start;
                parse_octal_escape(state)
            } else {
                Err(format!("Invalid backreference: {}", num))
            }
        }
        'p' | 'P' if state.is_unicode => parse_unicode_property(state, c == 'P'),
        'k' => parse_named_backref(state),
        'c' => {
            if state.is_unicode {
                return Err("Invalid escape: \\c".to_string());
            }
            let next = state.next().ok_or_else(|| "Unexpected end after \\c".to_string())?;
            if next.is_ascii_alphabetic() {
                Ok(Ast::Char(((next as u32) % 32) as u8 as char))
            } else {
                Ok(Ast::Char('c'))
            }
        }
        c => {
            if state.is_unicode && !is_unicode_identity_escape(c) {
                return Err(format!("Invalid identity escape: \\{}", c));
            }
            Ok(Ast::Char(c))
        }
    }
}

fn parse_group(state: &mut ParseState) -> Result<Ast, String> {
    state.expect('(')?;

    if state.peek() == Some('?') {
        state.next();

        match state.peek() {
            Some(':') => {
                state.next();
                let inner = parse_disjunction(state)?;
                state.expect(')')?;
                Ok(inner)
            }
            Some('=') => {
                state.next();
                let inner = parse_disjunction(state)?;
                state.expect(')')?;
                Ok(Ast::Lookahead(Box::new(inner)))
            }
            Some('!') => {
                state.next();
                let inner = parse_disjunction(state)?;
                state.expect(')')?;
                Ok(Ast::NegativeLookahead(Box::new(inner)))
            }
            Some('<') => {
                state.next();
                let name = parse_group_name(state)?;
                state.expect('>')?;

                if state.named_groups.contains(&name) {
                    return Err(format!("Duplicate capture group name: {}", name));
                }
                state.capture_count += 1;
                state.named_groups.push(name.clone());

                let inner = parse_disjunction(state)?;
                state.expect(')')?;

                Ok(Ast::Capture(Box::new(inner), Some(name)))
            }
            Some(c) => Err(format!("Unknown group extension: ?{}", c)),
            None => Err("Unexpected end after (?".to_string()),
        }
    } else {
        state.capture_count += 1;

        let inner = parse_disjunction(state)?;
        state.expect(')')?;

        Ok(Ast::Capture(Box::new(inner), None))
    }
}

fn parse_char_class(state: &mut ParseState) -> Result<Ast, String> {
    state.expect('[')?;

    let negated = if state.peek() == Some('^') {
        state.next();
        true
    } else {
        false
    };

    let mut ranges = CharRange::new();
    let mut prev_char: Option<char> = None;

    loop {
        match state.peek() {
            None => return Err("Unterminated character class".to_string()),
            Some(']') => {
                state.next();
                break;
            }
            Some('-') if prev_char.is_some() && state.peek_at(1) != Some(']') => {
                let next_is_set_escape = state.peek_at(1) == Some('\\')
                    && matches!(
                        state.peek_at(2),
                        Some('d' | 'D' | 's' | 'S' | 'w' | 'W')
                    );
                if next_is_set_escape && !state.is_unicode {
                    state.next();
                    ranges.add_char('-');
                    prev_char = None;
                } else {
                    state.next();
                    let end = parse_class_atom(state)?;
                    if let Some(start) = prev_char {
                        ranges.add_range(start as u32, end as u32);
                    }
                    prev_char = None;
                }
            }
            Some('-') => {
                state.next();
                ranges.add_char('-');
                prev_char = Some('-');
            }
            Some('\\') => {
                match parse_class_escape(state)? {
                    ClassEscape::Char(c) => {
                        ranges.add_char(c);
                        prev_char = Some(c);
                    }
                    ClassEscape::Set(set) => {
                        ranges.union(&set);
                        if state.is_unicode
                            && state.peek() == Some('-')
                            && state.peek_at(1) != Some(']')
                        {
                            return Err(
                                "Invalid class range: multi-character escape".to_string()
                            );
                        }
                        prev_char = None;
                    }
                }
            }
            Some(c) => {
                state.next();
                ranges.add_char(c);
                prev_char = Some(c);
            }
        }
    }

    if negated {
        ranges.invert();
    }

    Ok(Ast::Class(CharClass {
        negated: false,
        ranges,
    }))
}

fn parse_class_atom(state: &mut ParseState) -> Result<char, String> {
    match state.peek() {
        None => Err("Unexpected end in character class".to_string()),
        Some('\\') => match parse_class_escape(state)? {
            ClassEscape::Char(c) => Ok(c),
            ClassEscape::Set(_) => Err("Class escapes not allowed in range".to_string()),
        },
        Some(c) => {
            state.next();
            Ok(c)
        }
    }
}

enum ClassEscape {
    Char(char),
    Set(CharRange),
}

fn parse_class_escape(state: &mut ParseState) -> Result<ClassEscape, String> {
    state.expect('\\')?;

    let c = state
        .next()
        .ok_or_else(|| "Unexpected end after \\".to_string())?;

    match c {
        'b' => Ok(ClassEscape::Char('\x08')),
        'f' => Ok(ClassEscape::Char('\x0C')),
        'n' => Ok(ClassEscape::Char('\n')),
        'r' => Ok(ClassEscape::Char('\r')),
        't' => Ok(ClassEscape::Char('\t')),
        'v' => Ok(ClassEscape::Char('\x0B')),
        'x' => Ok(ClassEscape::Char(
            parse_hex_escape_char(state, 2)?.ok_or("Invalid hex escape")?,
        )),
        'u' => Ok(ClassEscape::Char(
            parse_unicode_escape_char(state)?.ok_or("Invalid unicode escape")?,
        )),
        '0' if state.is_unicode => {
            if state.peek().map_or(false, |c| c.is_ascii_digit()) {
                Err("Invalid octal escape".to_string())
            } else {
                Ok(ClassEscape::Char('\0'))
            }
        }

        'd' => Ok(ClassEscape::Set(class_digit())),
        'D' => {
            let mut set = class_digit();
            set.invert();
            Ok(ClassEscape::Set(set))
        }
        's' => Ok(ClassEscape::Set(class_space())),
        'S' => {
            let mut set = class_space();
            set.invert();
            Ok(ClassEscape::Set(set))
        }
        'w' => Ok(ClassEscape::Set(class_word())),
        'W' => {
            let mut set = class_word();
            set.invert();
            Ok(ClassEscape::Set(set))
        }
        'c' => {
            if state.is_unicode {
                return Err("Invalid class escape: \\c".to_string());
            }
            let next = state.next().ok_or_else(|| "Unexpected end after \\c".to_string())?;
            if next.is_ascii_alphabetic() {
                Ok(ClassEscape::Char(((next as u32) % 32) as u8 as char))
            } else {
                Ok(ClassEscape::Char('c'))
            }
        }
        c => {
            if state.is_unicode && !is_unicode_identity_escape(c) {
                return Err(format!("Invalid identity escape: \\{}", c));
            }
            Ok(ClassEscape::Char(c))
        }
    }
}

fn parse_hex_escape(state: &mut ParseState, digits: usize) -> Result<Ast, String> {
    let c = parse_hex_escape_char(state, digits)?.ok_or("Invalid hex escape")?;
    Ok(Ast::Char(c))
}

fn parse_hex_escape_char(state: &mut ParseState, digits: usize) -> Result<Option<char>, String> {
    let mut val: u32 = 0;
    for _ in 0..digits {
        let c = state
            .next()
            .ok_or_else(|| "Unexpected end in hex escape".to_string())?;
        let digit = c
            .to_digit(16)
            .ok_or_else(|| format!("Invalid hex digit: {}", c))?;
        val = val * 16 + digit;
    }
    if (0xD800..=0xDFFF).contains(&val) {
        return Ok(Some(crate::util::utf16::char_for_code_unit(val)));
    }
    char::from_u32(val)
        .map(Some)
        .ok_or_else(|| "Invalid Unicode code point".to_string())
}

fn parse_unicode_escape(state: &mut ParseState) -> Result<Ast, String> {
    let c = parse_unicode_escape_char(state)?;
    Ok(Ast::Char(c.unwrap_or('\u{FFFD}')))
}

fn parse_unicode_escape_char(state: &mut ParseState) -> Result<Option<char>, String> {
    if state.peek() == Some('{') {
        state.next();
        let mut val: u32 = 0;
        loop {
            match state.next() {
                Some('}') => break,
                Some(c) => {
                    let digit = c
                        .to_digit(16)
                        .ok_or_else(|| format!("Invalid hex digit: {}", c))?;
                    val = val * 16 + digit;
                    if val > 0x10FFFF {
                        return Err("Unicode code point out of range".to_string());
                    }
                }
                None => return Err("Unterminated Unicode escape".to_string()),
            }
        }
        char::from_u32(val)
            .map(Some)
            .ok_or_else(|| "Invalid Unicode code point".to_string())
    } else {
        let parsed = parse_hex_escape_char(state, 4)?;
        if state.is_unicode
            && let Some(high) = parsed
            && let Some(combined) = try_combine_surrogate_pair(state, high)
        {
            return Ok(Some(combined));
        }
        Ok(parsed)
    }
}

fn try_combine_surrogate_pair(state: &mut ParseState, high: char) -> Option<char> {
    let unit = crate::util::utf16::surrogate_escape_unit(high)?;
    if !(0xD800..=0xDBFF).contains(&unit) {
        return None;
    }
    if state.peek() != Some('\\') || state.peek_at(1) != Some('u') {
        return None;
    }
    let mut low: u32 = 0;
    for offset in 0..4 {
        let digit = state.peek_at(2 + offset)?.to_digit(16)?;
        low = low * 16 + digit;
    }
    if !(0xDC00..=0xDFFF).contains(&low) {
        return None;
    }
    for _ in 0..6 {
        state.next();
    }
    let code_point = 0x10000 + ((unit as u32 - 0xD800) << 10) + (low - 0xDC00);
    char::from_u32(code_point)
}

fn parse_octal_escape(state: &mut ParseState) -> Result<Ast, String> {
    let mut val: u32 = 0;
    let mut count = 0;

    while count < 3 {
        match state.peek() {
            Some(c) if c >= '0' && c <= '7' => {
                state.next();
                val = val * 8 + (c as u32 - '0' as u32);
                count += 1;
            }
            _ => break,
        }
    }

    if count == 0 {
        Ok(Ast::Char('\0'))
    } else {
        char::from_u32(val)
            .map(Ast::Char)
            .ok_or_else(|| "Invalid octal escape".to_string())
    }
}

fn parse_unicode_property(state: &mut ParseState, negated: bool) -> Result<Ast, String> {
    state.expect('{')?;

    let mut name = String::new();
    loop {
        match state.next() {
            Some('}') => break,
            Some(c) => name.push(c),
            None => return Err("Unterminated property name".to_string()),
        }
    }

    let ranges = match name.as_str() {
        "ASCII" => CharRange::from_range('\0', '\x7F'),
        "ASCII_Hex_Digit" => {
            let mut r = CharRange::from_range('0', '9');
            r.add_range('A' as u32, 'F' as u32);
            r.add_range('a' as u32, 'f' as u32);
            r
        }
        _ => CharRange::new(),
    };

    Ok(Ast::Class(CharClass { negated, ranges }))
}

fn parse_named_backref(state: &mut ParseState) -> Result<Ast, String> {
    state.expect('<')?;
    let name = parse_group_name(state)?;
    state.expect('>')?;

    state.named_backrefs.push(name.clone());

    Ok(Ast::NamedBackRef(name))
}

fn parse_group_name(state: &mut ParseState) -> Result<String, String> {
    let mut name = String::new();
    let mut first = true;

    loop {
        match state.peek() {
            Some('>') | Some(')') => break,
            Some(c) => {
                let valid = if first {
                    crate::compiler::lexer::Lexer::is_identifier_start(c)
                } else {
                    crate::compiler::lexer::Lexer::is_identifier_part(c)
                };
                if !valid {
                    return Err(format!("Invalid character in group name: {}", c));
                }
                state.next();
                name.push(c);
                first = false;
            }
            None => return Err("Unexpected end in group name".to_string()),
        }
    }

    if name.is_empty() {
        Err("Empty group name".to_string())
    } else {
        Ok(name)
    }
}

fn is_unicode_identity_escape(c: char) -> bool {
    matches!(
        c,
        '^' | '$' | '\\' | '.' | '*' | '+' | '?' | '(' | ')' | '[' | ']' | '{' | '}' | '|' | '/'
    )
}

fn is_special_char(c: char) -> bool {
    matches!(
        c,
        '^' | '$' | '\\' | '.' | '*' | '+' | '?' | '(' | ')' | '[' | ']' | '{' | '}' | '|'
    )
}

trait PeekAt {
    fn peek_at(&self, offset: usize) -> Option<char>;
}

impl PeekAt for ParseState {
    fn peek_at(&self, offset: usize) -> Option<char> {
        self.pattern.get(self.pos + offset).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple() {
        let ast = parse("abc", 0).unwrap();
        match ast {
            Ast::Concat(nodes) => {
                assert_eq!(nodes.len(), 3);
            }
            _ => panic!("Expected concat"),
        }
    }

    #[test]
    fn test_parse_alt() {
        let ast = parse("a|b|c", 0).unwrap();
        match ast {
            Ast::Alt(nodes) => {
                assert_eq!(nodes.len(), 3);
            }
            _ => panic!("Expected alt"),
        }
    }

    #[test]
    fn test_parse_quantifier() {
        let ast = parse("a*", 0).unwrap();
        match ast {
            Ast::Quant(_inner, q) => {
                assert_eq!(q.min, 0);
                assert_eq!(q.max, None);
                assert!(q.greedy);
            }
            _ => panic!("Expected quant"),
        }
    }

    #[test]
    fn test_parse_capture() {
        let ast = parse("(ab)+", 0).unwrap();
        match ast {
            Ast::Quant(inner, _) => match *inner {
                Ast::Capture(_, None) => {}
                _ => panic!("Expected capture"),
            },
            _ => panic!("Expected quant"),
        }
    }

    #[test]
    fn test_parse_char_class() {
        let ast = parse("[a-z]", 0).unwrap();
        match ast {
            Ast::Class(cc) => {
                assert!(!cc.negated);
                assert!(cc.ranges.contains_char('a'));
                assert!(cc.ranges.contains_char('z'));
                assert!(!cc.ranges.contains_char('A'));
            }
            _ => panic!("Expected class"),
        }
    }
}
