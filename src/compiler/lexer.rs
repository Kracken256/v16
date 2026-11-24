#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenType {
    Eof,
    Identifier,
    Keyword,
    Number,
    BigInt,
    String,
    Punctuator,
    Template,
    Regex,
    Hashbang,
    PrivateIdentifier,
    Error,

    NoSubstitutionTemplate,

    TemplateHead,

    TemplateMiddle,

    TemplateTail,
}

#[derive(Debug, Clone)]
pub struct Token {
    pub token_type: TokenType,
    pub value: String,
    pub raw: String,
    pub line: u32,
    pub column: u32,
    pub line_terminator_before: bool,
    pub had_escape: bool,
    pub had_legacy_escape: bool,
    pub invalid_escape: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LastTokenKind {
    None,

    Dividend,

    RegexPrefix,
}

#[derive(Debug, Clone)]
struct PeekCache {
    pre_pos: usize,
    pre_line: u32,
    pre_column: u32,
    pre_last_token_kind: LastTokenKind,
    post_pos: usize,
    post_line: u32,
    post_column: u32,
    post_last_token_kind: LastTokenKind,
    token: Token,
}

pub struct Lexer {
    pub source: Vec<char>,
    pub pos: usize,
    pub line: u32,
    pub column: u32,

    last_token_kind: LastTokenKind,

    cached_peek: Option<PeekCache>,

    pub last_string_had_escape: bool,
    pub error: Option<String>,

    pending_line_terminator: bool,
}

impl Lexer {
    pub fn new(source: &str) -> Self {
        Lexer {
            source: source.chars().collect(),
            pos: 0,
            line: 1,
            column: 1,
            last_token_kind: LastTokenKind::None,
            cached_peek: None,
            last_string_had_escape: false,
            error: None,
            pending_line_terminator: false,
        }
    }

    pub fn next_token(&mut self) -> Option<Token> {
        if let Some(message) = self.error.take() {
            return Some(self.error_token(message));
        }
        if let Some(cache) = self.cached_peek.take() {
            if cache.pre_pos == self.pos
                && cache.pre_line == self.line
                && cache.pre_column == self.column
                && cache.pre_last_token_kind == self.last_token_kind
            {
                self.pos = cache.post_pos;
                self.line = cache.post_line;
                self.column = cache.post_column;
                self.last_token_kind = cache.post_last_token_kind;
                return Some(cache.token);
            }
        }

        let result = self.next_token_uncached();
        if let Some(message) = self.error.take() {
            return Some(self.error_token(message));
        }
        result
    }

    fn fail(&mut self, message: &str) {
        if self.error.is_none() {
            self.error = Some(message.to_string());
        }
    }

    fn error_token(&self, message: String) -> Token {
        Token {
            token_type: TokenType::Error,
            value: message,
            line: self.line,
            column: self.column,
            line_terminator_before: false,
            raw: String::new(),
            had_escape: false,
            had_legacy_escape: false,
            invalid_escape: false,
        }
    }

    fn check_number_end(&mut self) {
        if self.pos < self.source.len() {
            let c = self.source[self.pos];
            if Self::is_identifier_start(c) || c.is_ascii_digit() || c == '\\' {
                self.fail("Identifier directly after number");
            }
        }
    }

    fn next_token_uncached(&mut self) -> Option<Token> {
        let mut token = self.next_token_uncached_inner()?;
        token.line_terminator_before = token.line_terminator_before || self.pending_line_terminator;
        self.pending_line_terminator = false;
        Some(token)
    }

    fn next_token_uncached_inner(&mut self) -> Option<Token> {
        self.skip_whitespace()?;
        if self.pos >= self.source.len() {
            return None;
        }

        if self.pos == 0 && self.source[self.pos] == '#' {
            if self.pos + 1 < self.source.len() && self.source[self.pos + 1] == '!' {
                let token = self.read_hashbang();
                self.last_token_kind = LastTokenKind::RegexPrefix;
                return Some(token);
            }
        }

        let c = self.source[self.pos];

        if c.is_ascii_digit() {
            let token = self.read_number();
            self.last_token_kind = LastTokenKind::Dividend;
            return Some(token);
        }

        if c == '.'
            && self.pos + 1 < self.source.len()
            && self.source[self.pos + 1].is_ascii_digit()
        {
            let token = self.read_number();
            self.last_token_kind = LastTokenKind::Dividend;
            return Some(token);
        }

        if c == '"' || c == '\'' {
            let token = self.read_string(c);
            self.last_token_kind = LastTokenKind::Dividend;
            return Some(token);
        }

        if c == '`' {
            self.advance();
            let (value, raw, terminated_by_interp, invalid_escape) =
                self.scan_template_segment();
            self.last_token_kind = LastTokenKind::Dividend;
            let token_type = if terminated_by_interp {
                TokenType::TemplateHead
            } else {
                TokenType::NoSubstitutionTemplate
            };
            return Some(Token {
                token_type,
                value,
                raw,
                line: self.line,
                column: self.column,
                line_terminator_before: false,
                had_escape: false,
                had_legacy_escape: false,
                invalid_escape,
            });
        }

        if Self::is_identifier_start(c) || c == '\\' {
            let token = self.read_identifier();
            self.last_token_kind = if matches!(
                token.value.as_str(),
                "return"
                    | "throw"
                    | "case"
                    | "typeof"
                    | "void"
                    | "new"
                    | "delete"
                    | "in"
                    | "instanceof"
                    | "yield"
            ) {
                LastTokenKind::RegexPrefix
            } else {
                LastTokenKind::Dividend
            };
            return Some(token);
        }

        if c == '#' && self.pos > 0 {
            if self.pos + 1 < self.source.len() {
                let next = self.source[self.pos + 1];
                if Self::is_identifier_start(next) || next == '\\' {
                    let token = self.read_private_identifier();
                    self.last_token_kind = LastTokenKind::Dividend;
                    return Some(token);
                }
            }
        }

        if c == '/' {
            let token = self.read_comment_or_regex();
            if token.token_type == TokenType::Eof {
                return None;
            }
            return Some(token);
        }

        let token = self.read_punctuator();
        self.last_token_kind = match token.value.as_str() {
            ")" | "]" | "}" | "++" | "--" => LastTokenKind::Dividend,
            _ => LastTokenKind::RegexPrefix,
        };
        Some(token)
    }

    pub fn set_pos(&mut self, pos: usize) {
        self.pos = pos;

        self.cached_peek = None;
        self.last_token_kind = LastTokenKind::None;
        self.pending_line_terminator = false;
    }

    pub fn pos(&self) -> usize {
        self.pos
    }

    pub fn column(&self) -> u32 {
        self.column
    }

    pub fn set_column(&mut self, col: u32) {
        self.column = col;
    }

    pub fn last_token_kind(&self) -> LastTokenKind {
        self.last_token_kind
    }

    pub fn set_last_token_kind(&mut self, kind: LastTokenKind) {
        self.last_token_kind = kind;
    }

    pub fn line(&self) -> u32 {
        self.line
    }

    pub fn set_line(&mut self, line: u32) {
        self.line = line;
    }

    pub fn get_current_line(&self) -> String {
        let mut line_start = self.pos;
        while line_start > 0 && self.source[line_start - 1] != '\n' {
            line_start -= 1;
        }

        let mut line_end = self.pos;
        while line_end < self.source.len() && self.source[line_end] != '\n' {
            line_end += 1;
        }

        self.source[line_start..line_end].iter().collect()
    }

    pub fn peek(&mut self) -> Option<Token> {
        if let Some(cache) = self.cached_peek.as_ref() {
            if cache.pre_pos == self.pos
                && cache.pre_line == self.line
                && cache.pre_column == self.column
                && cache.pre_last_token_kind == self.last_token_kind
            {
                return Some(cache.token.clone());
            }
        }

        let old_pos = self.pos;
        let old_line = self.line;
        let old_column = self.column;
        let old_last_token_kind = self.last_token_kind;
        let old_pending_line_terminator = self.pending_line_terminator;

        self.cached_peek = None;
        let result = self.next_token_uncached();

        let new_pos = self.pos;
        let new_line = self.line;
        let new_column = self.column;
        let new_last_token_kind = self.last_token_kind;

        self.pos = old_pos;
        self.line = old_line;
        self.column = old_column;
        self.last_token_kind = old_last_token_kind;
        self.pending_line_terminator = old_pending_line_terminator;

        if let Some(token) = result {
            self.cached_peek = Some(PeekCache {
                pre_pos: old_pos,
                pre_line: old_line,
                pre_column: old_column,
                pre_last_token_kind: old_last_token_kind,
                post_pos: new_pos,
                post_line: new_line,
                post_column: new_column,
                post_last_token_kind: new_last_token_kind,
                token: token.clone(),
            });
            return Some(token);
        }

        None
    }

    fn skip_whitespace(&mut self) -> Option<()> {
        while self.pos < self.source.len() {
            match self.source[self.pos] {
                '\n' | '\r' | '\u{2028}' | '\u{2029}' => {
                    self.line += 1;
                    self.column = 1;
                    self.pos += 1;
                    self.pending_line_terminator = true;
                }
                ' ' | '\t' | '\u{000B}' | '\u{000C}' | '\u{00A0}' | '\u{1680}' | '\u{202F}'
                | '\u{205F}' | '\u{3000}' | '\u{FEFF}' => {
                    self.column += 1;
                    self.pos += 1;
                }
                c if (c as u32) >= 0x2000 && (c as u32) <= 0x200A => {
                    self.column += 1;
                    self.pos += 1;
                }
                _ => break,
            }
        }
        Some(())
    }

    fn advance(&mut self) {
        if self.pos < self.source.len() {
            if self.source[self.pos] == '\n' || self.source[self.pos] == '\r' {
                self.line += 1;
                self.column = 1;
            } else {
                self.column += 1;
            }
            self.pos += 1;
        }
    }

    fn read_number(&mut self) -> Token {
        let start = self.pos;

        if self.source[self.pos] == '0'
            && self.pos + 1 < self.source.len()
            && matches!(self.source[self.pos + 1], 'x' | 'X' | 'o' | 'O' | 'b' | 'B')
        {
            let radix = self.source[self.pos + 1];
            self.advance();
            self.advance();
            let digits_start = self.pos;
            while self.pos < self.source.len()
                && (self.source[self.pos] == '_' || is_radix_digit(radix, self.source[self.pos]))
            {
                self.advance();
            }
            let digits: String = self.source[digits_start..self.pos].iter().collect();
            let mut is_bigint = false;
            if self.pos < self.source.len() && self.source[self.pos] == 'n' {
                is_bigint = true;
            }
            let value_end = self.pos;
            if is_bigint {
                self.advance();
            }
            let value: String = self.source[start..value_end].iter().collect();
            let mut valid = valid_digit_run(&digits);
            if is_bigint && !valid {
                valid = false;
            }
            if valid {
                self.check_number_end();
            }
            return Token {
                token_type: if !valid {
                    TokenType::Punctuator
                } else if is_bigint {
                    TokenType::BigInt
                } else {
                    TokenType::Number
                },
                value,
                line: self.line,
                column: self.column,
                line_terminator_before: false,
                raw: String::new(),
                had_escape: false,
                had_legacy_escape: false,
                invalid_escape: false,
            };
        }

        let int_start = self.pos;
        while self.pos < self.source.len()
            && (self.source[self.pos].is_ascii_digit() || self.source[self.pos] == '_')
        {
            self.advance();
        }
        let int_part: String = self.source[int_start..self.pos].iter().collect();

        let mut has_dot = false;
        let mut frac_part = String::new();
        if self.pos < self.source.len() && self.source[self.pos] == '.' {
            has_dot = true;
            self.advance();
            let frac_start = self.pos;
            while self.pos < self.source.len()
                && (self.source[self.pos].is_ascii_digit() || self.source[self.pos] == '_')
            {
                self.advance();
            }
            frac_part = self.source[frac_start..self.pos].iter().collect();
        }

        let mut has_exp = false;
        let mut exp_part = String::new();
        if self.pos < self.source.len()
            && (self.source[self.pos] == 'e' || self.source[self.pos] == 'E')
        {
            has_exp = true;
            self.advance();
            if self.pos < self.source.len()
                && (self.source[self.pos] == '+' || self.source[self.pos] == '-')
            {
                self.advance();
            }
            let exp_start = self.pos;
            while self.pos < self.source.len()
                && (self.source[self.pos].is_ascii_digit() || self.source[self.pos] == '_')
            {
                self.advance();
            }
            exp_part = self.source[exp_start..self.pos].iter().collect();
        }

        let mut is_bigint = false;
        if !has_dot && !has_exp && self.pos < self.source.len() && self.source[self.pos] == 'n' {
            is_bigint = true;
        }
        let value_end = self.pos;
        if is_bigint {
            self.advance();
        }

        let value: String = self.source[start..value_end].iter().collect();

        let mut valid = if has_dot {
            !(int_part.is_empty() && frac_part.is_empty())
                && (int_part.is_empty() || valid_digit_run(&int_part))
                && (frac_part.is_empty() || valid_digit_run(&frac_part))
        } else {
            valid_digit_run(&int_part)
        };
        if has_exp && !valid_digit_run(&exp_part) {
            valid = false;
        }
        let int_bytes = int_part.as_bytes();
        if int_bytes.len() > 1 && int_bytes[0] == b'0' {
            if int_part.contains('_') {
                valid = false;
            } else if is_bigint {
                valid = false;
            } else if has_dot || has_exp {
                if !int_part.bytes().any(|c| c == b'8' || c == b'9') {
                    valid = false;
                }
            }
        }
        if valid {
            self.check_number_end();
        }

        Token {
            token_type: if !valid {
                TokenType::Punctuator
            } else if is_bigint {
                TokenType::BigInt
            } else {
                TokenType::Number
            },
            value,
            line: self.line,
            column: self.column,
            line_terminator_before: false,
            raw: String::new(),
            had_escape: false,
            had_legacy_escape: false,
            invalid_escape: false,
        }
    }

    pub fn get_context(&self, chars: usize) -> String {
        let start = self.pos.saturating_sub(chars);
        let end = (self.pos + chars).min(self.source.len());
        self.source[start..end].iter().collect()
    }

    fn read_string(&mut self, quote: char) -> Token {
        let token_line = self.line;
        let token_column = self.column;
        self.last_string_had_escape = false;
        self.advance();
        let mut value = String::new();
        let mut had_escape = false;
        let mut had_legacy_escape = false;
        let mut terminated = false;
        while self.pos < self.source.len() {
            let ch = self.source[self.pos];
            if ch == quote {
                self.advance();
                terminated = true;
                break;
            }
            if ch == '\n' || ch == '\r' {
                self.fail("Unterminated string literal");
                break;
            }
            if ch != '\\' {
                value.push(ch);
                self.advance();
                continue;
            }

            self.advance();
            if self.pos >= self.source.len() {
                self.fail("Unterminated string literal");
                break;
            }

            had_escape = true;
            self.last_string_had_escape = true;
            let esc = self.source[self.pos];
            match esc {
                '\n' | '\u{2028}' | '\u{2029}' => {}
                '\r' => {
                    if self.pos + 1 < self.source.len() && self.source[self.pos + 1] == '\n' {
                        self.advance();
                    }
                }
                'n' => value.push('\n'),
                'r' => value.push('\r'),
                't' => value.push('\t'),
                'b' => value.push('\x08'),
                'f' => value.push('\x0c'),
                'v' => value.push('\x0b'),
                '0'..='7' => {
                    had_legacy_escape = true;
                    let first_val = esc as u32 - '0' as u32;
                    if first_val == 0
                        && (self.pos + 1 >= self.source.len()
                            || self.source[self.pos + 1] < '0'
                            || self.source[self.pos + 1] > '7')
                    {
                        value.push('\0');
                    } else {
                        let mut code = first_val;
                        let mut count = 1u32;
                        let mut lookahead = 1;
                        while self.pos + lookahead < self.source.len()
                            && count < 3
                            && self.source[self.pos + lookahead] >= '0'
                            && self.source[self.pos + lookahead] <= '7'
                        {
                            let next =
                                code * 8 + (self.source[self.pos + lookahead] as u32 - '0' as u32);
                            if next > 255 {
                                break;
                            }
                            code = next;
                            count += 1;
                            lookahead += 1;
                        }
                        for _ in 1..lookahead {
                            self.advance();
                        }
                        if let Some(ch) = char::from_u32(code) {
                            value.push(ch);
                        }
                    }
                }
                '8' | '9' => {
                    had_legacy_escape = true;
                    value.push(esc);
                }
                '\\' => value.push('\\'),
                '\'' => value.push('\''),
                '"' => value.push('"'),
                '`' => value.push('`'),
                'x' => {
                    if self.pos + 2 < self.source.len() {
                        let h1 = self.source[self.pos + 1];
                        let h2 = self.source[self.pos + 2];
                        if let (Some(a), Some(b)) = (h1.to_digit(16), h2.to_digit(16)) {
                            let code = (a << 4) | b;
                            if let Some(c) = char::from_u32(code) {
                                value.push(c);
                            }
                            self.advance();
                            self.advance();
                        } else {
                            self.fail("Invalid hexadecimal escape sequence");
                        }
                    } else {
                        self.fail("Invalid hexadecimal escape sequence");
                    }
                }
                'u' => {
                    if self.pos + 1 < self.source.len() && self.source[self.pos + 1] == '{' {
                        let mut code: u32 = 0;
                        let mut digits = 0;
                        let mut overflow = false;
                        let mut i = self.pos + 2;
                        while i < self.source.len() && self.source[i] != '}' {
                            if let Some(d) = self.source[i].to_digit(16) {
                                code = code.saturating_mul(16).saturating_add(d);
                                if code > 0x10FFFF {
                                    overflow = true;
                                }
                                digits += 1;
                                i += 1;
                            } else {
                                break;
                            }
                        }
                        if i < self.source.len() && self.source[i] == '}' && digits > 0 && !overflow {
                            if let Some(decoded) = char::from_u32(code) {
                                value.push(decoded);
                            }
                            let steps = i - self.pos;
                            for _ in 0..steps {
                                self.advance();
                            }
                        } else {
                            self.fail("Invalid Unicode escape sequence");
                        }
                    } else if self.pos + 4 < self.source.len() {
                        let h1 = self.source[self.pos + 1];
                        let h2 = self.source[self.pos + 2];
                        let h3 = self.source[self.pos + 3];
                        let h4 = self.source[self.pos + 4];
                        if let (Some(a), Some(b), Some(c), Some(d)) = (
                            h1.to_digit(16),
                            h2.to_digit(16),
                            h3.to_digit(16),
                            h4.to_digit(16),
                        ) {
                            let code = (a << 12) | (b << 8) | (c << 4) | d;
                            self.advance();
                            self.advance();
                            self.advance();
                            self.advance();
                            let mut combined_pair = false;
                            if (0xD800..=0xDBFF).contains(&code) && self.pos + 6 < self.source.len()
                            {
                                if self.source[self.pos + 1] == '\\'
                                    && self.source[self.pos + 2] == 'u'
                                {
                                    if let (Some(a2), Some(b2), Some(c2), Some(d2)) = (
                                        self.source[self.pos + 3].to_digit(16),
                                        self.source[self.pos + 4].to_digit(16),
                                        self.source[self.pos + 5].to_digit(16),
                                        self.source[self.pos + 6].to_digit(16),
                                    ) {
                                        let low = (a2 << 12) | (b2 << 8) | (c2 << 4) | d2;
                                        if (0xDC00..=0xDFFF).contains(&low) {
                                            let cp =
                                                0x10000 + ((code - 0xD800) << 10) + (low - 0xDC00);
                                            if let Some(ch) = char::from_u32(cp) {
                                                value.push(ch);
                                            }
                                            for _ in 0..6 {
                                                self.advance();
                                            }
                                            combined_pair = true;
                                        }
                                    }
                                }
                            }
                            if !combined_pair {
                                if let Some(decoded) = char::from_u32(code) {
                                    value.push(decoded);
                                }
                            }
                        } else {
                            self.fail("Invalid Unicode escape sequence");
                        }
                    } else {
                        self.fail("Invalid Unicode escape sequence");
                    }
                }
                _ => value.push(esc),
            }
            self.advance();
        }
        if !terminated && self.error.is_none() {
            self.fail("Unterminated string literal");
        }
        Token {
            token_type: TokenType::String,
            value,
            line: token_line,
            column: token_column,
            line_terminator_before: false,
            raw: String::new(),
            had_escape,
            had_legacy_escape,
            invalid_escape: false,
        }
    }

    fn read_hashbang(&mut self) -> Token {
        let start = self.pos;

        self.pos += 2;

        while self.pos < self.source.len() {
            let c = self.source[self.pos];
            if c == '\n' || c == '\r' {
                break;
            }
            self.pos += 1;
        }
        let value: String = self.source[start..self.pos].iter().collect();
        Token {
            token_type: TokenType::Hashbang,
            value,
            line: self.line,
            column: self.column,
            line_terminator_before: false,
            raw: String::new(),
            had_escape: false,
            had_legacy_escape: false,
            invalid_escape: false,
        }
    }

    fn read_identifier(&mut self) -> Token {
        let start = self.pos;
        let mut value = String::new();
        let mut had_escape = false;
        while self.pos < self.source.len() {
            let c = self.source[self.pos];
            if Self::is_identifier_part(c) {
                value.push(c);
                self.advance();
            } else if c == '\\' {
                if let Some(ch) = self.read_identifier_escape() {
                    value.push(ch);
                    had_escape = true;
                } else {
                    break;
                }
            } else {
                break;
            }
        }
        if value.is_empty() {
            value = self.source[start..self.pos].iter().collect();
        }
        let token_type = if !had_escape && Self::is_keyword(&value) {
            TokenType::Keyword
        } else {
            TokenType::Identifier
        };
        Token {
            token_type,
            value,
            line: self.line,
            column: self.column,
            line_terminator_before: false,
            raw: String::new(),
            had_escape,
            had_legacy_escape: false,
            invalid_escape: false,
        }
    }

    fn read_identifier_escape(&mut self) -> Option<char> {
        if self.pos >= self.source.len() || self.source[self.pos] != '\\' {
            return None;
        }
        self.advance();
        if self.pos >= self.source.len() || self.source[self.pos] != 'u' {
            return None;
        }
        self.advance();

        if self.pos < self.source.len() && self.source[self.pos] == '{' {
            self.advance();
            let mut hex = String::new();
            while self.pos < self.source.len() && self.source[self.pos] != '}' {
                let c = self.source[self.pos];
                if !c.is_ascii_hexdigit() {
                    return None;
                }
                hex.push(c);
                self.advance();
            }
            if self.pos >= self.source.len() || self.source[self.pos] != '}' {
                return None;
            }
            self.advance();
            let code = u32::from_str_radix(&hex, 16).ok()?;
            return char::from_u32(code);
        }

        if self.pos + 3 >= self.source.len() {
            return None;
        }
        let mut code: u32 = 0;
        for _ in 0..4 {
            let c = self.source[self.pos];
            let d = c.to_digit(16)?;
            code = (code << 4) | d;
            self.advance();
        }
        char::from_u32(code)
    }

    fn read_private_identifier(&mut self) -> Token {
        self.advance();
        let mut value = String::from("#");
        while self.pos < self.source.len() {
            let c = self.source[self.pos];
            if Self::is_identifier_part(c) {
                value.push(c);
                self.advance();
            } else if c == '\\' {
                if let Some(ch) = self.read_identifier_escape() {
                    value.push(ch);
                } else {
                    break;
                }
            } else {
                break;
            }
        }
        Token {
            token_type: TokenType::PrivateIdentifier,
            value,
            line: self.line,
            column: self.column,
            line_terminator_before: false,
            raw: String::new(),
            had_escape: false,
            had_legacy_escape: false,
            invalid_escape: false,
        }
    }

    fn read_comment_or_regex(&mut self) -> Token {
        self.advance();
        if self.pos < self.source.len() {
            if self.source[self.pos] == '/' {
                while self.pos < self.source.len()
                    && self.source[self.pos] != '\n'
                    && self.source[self.pos] != '\r'
                    && self.source[self.pos] != '\u{2028}'
                    && self.source[self.pos] != '\u{2029}'
                {
                    self.advance();
                }
                return self.next_token().unwrap_or(Token {
                    token_type: TokenType::Eof,
                    value: String::new(),
                    line: self.line,
                    column: self.column,
                    line_terminator_before: false,
                    raw: String::new(),
                    had_escape: false,
                    had_legacy_escape: false,
                    invalid_escape: false,
                });
            }

            if self.source[self.pos] == '*' {
                self.advance();
                while self.pos < self.source.len() {
                    if self.source[self.pos] == '*'
                        && self.pos + 1 < self.source.len()
                        && self.source[self.pos + 1] == '/'
                    {
                        self.advance();
                        self.advance();
                        break;
                    }
                    if matches!(self.source[self.pos], '\n' | '\r' | '\u{2028}' | '\u{2029}') {
                        self.pending_line_terminator = true;
                    }
                    self.advance();
                }
                return self.next_token().unwrap_or(Token {
                    token_type: TokenType::Eof,
                    value: String::new(),
                    line: self.line,
                    column: self.column,
                    line_terminator_before: false,
                    raw: String::new(),
                    had_escape: false,
                    had_legacy_escape: false,
                    invalid_escape: false,
                });
            }

            let next_char = self.source[self.pos];
            if self.last_token_kind == LastTokenKind::Dividend {
                if next_char == '=' {
                    self.advance();
                    self.last_token_kind = LastTokenKind::Dividend;
                    return Token {
                        token_type: TokenType::Punctuator,
                        value: "/=".to_string(),
                        line: self.line,
                        column: self.column,
                        line_terminator_before: false,
                        raw: String::new(),
                        had_escape: false,
                        had_legacy_escape: false,
                        invalid_escape: false,
                    };
                }

                self.last_token_kind = LastTokenKind::RegexPrefix;
                return Token {
                    token_type: TokenType::Punctuator,
                    value: "/".to_string(),
                    line: self.line,
                    column: self.column,
                    line_terminator_before: false,
                    raw: String::new(),
                    had_escape: false,
                    had_legacy_escape: false,
                    invalid_escape: false,
                };
            }

            let mut pattern = String::new();
            let mut flags = String::new();

            while self.pos < self.source.len() {
                let c = self.source[self.pos];
                if c == '/' {
                    self.advance();
                    break;
                } else if c == '\\' {
                    pattern.push(c);
                    self.advance();
                    if self.pos < self.source.len() {
                        pattern.push(self.source[self.pos]);
                        self.advance();
                    }
                } else if c == '[' {
                    pattern.push(c);
                    self.advance();
                    while self.pos < self.source.len() {
                        let cc = self.source[self.pos];
                        pattern.push(cc);
                        self.advance();
                        if cc == ']' {
                            break;
                        }
                    }
                } else if c == '\n' || c == '\r' {
                    break;
                } else {
                    pattern.push(c);
                    self.advance();
                }
            }

            while self.pos < self.source.len() {
                let c = self.source[self.pos];
                if c.is_ascii_alphabetic() {
                    flags.push(c);
                    self.advance();
                } else {
                    break;
                }
            }

            self.last_token_kind = LastTokenKind::Dividend;

            return Token {
                token_type: TokenType::Regex,
                value: format!("{}/{}", pattern, flags),
                line: self.line,
                column: self.column,
                line_terminator_before: false,
                raw: String::new(),
                had_escape: false,
                had_legacy_escape: false,
                invalid_escape: false,
            };
        }
        self.last_token_kind = LastTokenKind::RegexPrefix;
        Token {
            token_type: TokenType::Punctuator,
            value: "/".to_string(),
            line: self.line,
            column: self.column,
            line_terminator_before: false,
            raw: String::new(),
            had_escape: false,
            had_legacy_escape: false,
            invalid_escape: false,
        }
    }

    fn read_punctuator(&mut self) -> Token {
        let c = self.source[self.pos];
        self.advance();

        let value: String = if self.pos < self.source.len() {
            let next = self.source[self.pos];
            match c {
                '<' if next == '<' => {
                    self.advance();
                    if self.pos < self.source.len() && self.source[self.pos] == '=' {
                        self.advance();
                        "<<=".to_string()
                    } else {
                        "<<".to_string()
                    }
                }
                '<' if next == '=' => {
                    self.advance();
                    "<=".to_string()
                }
                '>' if next == '>' => {
                    self.advance();
                    if self.pos < self.source.len() && self.source[self.pos] == '>' {
                        self.advance();
                        if self.pos < self.source.len() && self.source[self.pos] == '=' {
                            self.advance();
                            ">>>=".to_string()
                        } else {
                            ">>>".to_string()
                        }
                    } else if self.pos < self.source.len() && self.source[self.pos] == '=' {
                        self.advance();
                        ">>=".to_string()
                    } else {
                        ">>".to_string()
                    }
                }
                '>' if next == '=' => {
                    self.advance();
                    ">=".to_string()
                }
                '=' if next == '>' => {
                    self.advance();
                    "=>".to_string()
                }
                '.' if next == '.' => {
                    self.advance();
                    if self.pos < self.source.len() && self.source[self.pos] == '.' {
                        self.advance();
                        "...".to_string()
                    } else {
                        "..".to_string()
                    }
                }
                '=' if next == '=' => {
                    self.advance();
                    if self.pos < self.source.len() && self.source[self.pos] == '=' {
                        self.advance();
                        "===".to_string()
                    } else {
                        "==".to_string()
                    }
                }
                '!' if next == '=' => {
                    self.advance();
                    if self.pos < self.source.len() && self.source[self.pos] == '=' {
                        self.advance();
                        "!==".to_string()
                    } else {
                        "!=".to_string()
                    }
                }
                '*' if next == '=' => {
                    self.advance();
                    "*=".to_string()
                }
                '/' if next == '=' => {
                    self.advance();
                    "/=".to_string()
                }
                '%' if next == '=' => {
                    self.advance();
                    "%=".to_string()
                }
                '+' if next == '=' => {
                    self.advance();
                    "+=".to_string()
                }
                '-' if next == '=' => {
                    self.advance();
                    "-=".to_string()
                }
                '&' if next == '=' => {
                    self.advance();
                    "&=".to_string()
                }
                '|' if next == '=' => {
                    self.advance();
                    "|=".to_string()
                }
                '^' if next == '=' => {
                    self.advance();
                    "^=".to_string()
                }
                '<' if next == '=' => {
                    self.advance();
                    "<=".to_string()
                }
                '>' if next == '=' => {
                    self.advance();
                    ">=".to_string()
                }
                '&' if next == '&' => {
                    self.advance();
                    if self.pos < self.source.len() && self.source[self.pos] == '=' {
                        self.advance();
                        "&&=".to_string()
                    } else {
                        "&&".to_string()
                    }
                }
                '|' if next == '|' => {
                    self.advance();
                    if self.pos < self.source.len() && self.source[self.pos] == '=' {
                        self.advance();
                        "||=".to_string()
                    } else {
                        "||".to_string()
                    }
                }
                '+' if next == '+' => {
                    self.advance();
                    "++".to_string()
                }
                '-' if next == '-' => {
                    self.advance();
                    "--".to_string()
                }
                '?' if next == '?' => {
                    self.advance();

                    if self.pos < self.source.len() && self.source[self.pos] == '=' {
                        self.advance();
                        "??=".to_string()
                    } else {
                        "??".to_string()
                    }
                }
                '?' if next == '.' => {
                    self.advance();
                    "?.".to_string()
                }
                '*' if next == '*' => {
                    self.advance();
                    if self.pos < self.source.len() && self.source[self.pos] == '=' {
                        self.advance();
                        "**=".to_string()
                    } else {
                        "**".to_string()
                    }
                }
                _ => c.to_string(),
            }
        } else {
            c.to_string()
        };

        Token {
            token_type: TokenType::Punctuator,
            value,
            line: self.line,
            column: self.column,
            line_terminator_before: false,
            raw: String::new(),
            had_escape: false,
            had_legacy_escape: false,
            invalid_escape: false,
        }
    }

    pub(crate) fn is_identifier_start(c: char) -> bool {
        if c == '$' || c == '_' {
            return true;
        }
        if c.is_ascii() {
            return c.is_ascii_alphabetic();
        }
        crate::builtins::unicode_data::XID_START.contains(c as u32)
            || matches!(c, '\u{2118}' | '\u{212E}' | '\u{309B}' | '\u{309C}')
    }

    pub(crate) fn is_identifier_part(c: char) -> bool {
        if c == '$' || c == '_' {
            return true;
        }
        if c.is_ascii() {
            return c.is_ascii_alphanumeric();
        }
        if c == '\u{200C}' || c == '\u{200D}' {
            return true;
        }
        Self::is_identifier_start(c)
            || crate::builtins::unicode_data::XID_CONTINUE.contains(c as u32)
    }

    fn is_keyword(s: &str) -> bool {
        matches!(
            s,
            "break"
                | "case"
                | "catch"
                | "class"
                | "const"
                | "continue"
                | "debugger"
                | "default"
                | "delete"
                | "do"
                | "else"
                | "export"
                | "extends"
                | "finally"
                | "for"
                | "function"
                | "if"
                | "import"
                | "in"
                | "instanceof"
                | "let"
                | "new"
                | "return"
                | "super"
                | "switch"
                | "this"
                | "throw"
                | "try"
                | "typeof"
                | "var"
                | "void"
                | "while"
                | "with"
                | "yield"
                | "async"
                | "await"
                | "static"
                | "get"
                | "set"
                | "true"
                | "false"
                | "null"
                | "from"
                | "as"
                | "of"
        )
    }

    pub fn read_template_chars(&mut self) -> Option<String> {
        let mut result = String::new();
        while self.pos < self.source.len() {
            let c = self.source[self.pos];

            if c == '$' && self.pos + 1 < self.source.len() && self.source[self.pos + 1] == '{' {
                break;
            }

            if c == '`' {
                break;
            }
            result.push(c);
            self.advance();
        }
        if result.is_empty() {
            None
        } else {
            Some(result)
        }
    }

    fn scan_template_segment(&mut self) -> (String, String, bool, bool) {
        let mut value = String::new();
        let mut raw = String::new();
        let mut invalid_escape = false;
        while self.pos < self.source.len() {
            let c = self.source[self.pos];

            if c == '$' && self.pos + 1 < self.source.len() && self.source[self.pos + 1] == '{' {
                self.advance();
                self.advance();
                return (value, raw, true, invalid_escape);
            }

            if c == '`' {
                self.advance();
                return (value, raw, false, invalid_escape);
            }

            if c != '\\' {
                if c == '\n' {
                    raw.push('\n');
                    value.push('\n');
                    self.pos += 1;
                    self.line += 1;
                    self.column = 1;
                    continue;
                }
                if c == '\r' {
                    raw.push('\n');
                    value.push('\n');
                    self.pos += 1;
                    self.line += 1;
                    self.column = 1;
                    if self.pos < self.source.len() && self.source[self.pos] == '\n' {
                        self.pos += 1;
                    }
                    continue;
                }
                raw.push(c);
                value.push(c);
                self.advance();
                continue;
            }

            raw.push('\\');
            self.advance();
            if self.pos >= self.source.len() {
                break;
            }
            let esc = self.source[self.pos];
            match esc {
                '\n' | '\u{2028}' | '\u{2029}' => {
                    raw.push(esc);
                }
                '\r' => {
                    raw.push('\n');
                    self.advance();
                    if self.pos < self.source.len() && self.source[self.pos] == '\n' {
                        self.advance();
                    }
                    continue;
                }
                'n' => {
                    raw.push('n');
                    value.push('\n');
                }
                'r' => {
                    raw.push('r');
                    value.push('\r');
                }
                't' => {
                    raw.push('t');
                    value.push('\t');
                }
                'b' => {
                    raw.push('b');
                    value.push('\x08');
                }
                'f' => {
                    raw.push('f');
                    value.push('\x0c');
                }
                'v' => {
                    raw.push('v');
                    value.push('\x0b');
                }
                '0' => {
                    raw.push('0');
                    if self.pos + 1 < self.source.len()
                        && self.source[self.pos + 1].is_ascii_digit()
                    {
                        invalid_escape = true;
                    } else {
                        value.push('\0');
                    }
                }
                '1'..='7' => {
                    invalid_escape = true;
                    raw.push(esc);
                    let mut count = 1;
                    while count < 3
                        && self.pos + 1 < self.source.len()
                        && matches!(self.source[self.pos + 1], '0'..='7')
                    {
                        self.advance();
                        raw.push(self.source[self.pos]);
                        count += 1;
                    }
                }
                '8' | '9' => {
                    invalid_escape = true;
                    raw.push(esc);
                }
                '\\' => {
                    raw.push('\\');
                    value.push('\\');
                }
                '\'' => {
                    raw.push('\'');
                    value.push('\'');
                }
                '"' => {
                    raw.push('"');
                    value.push('"');
                }
                '`' => {
                    raw.push('`');
                    value.push('`');
                }
                '$' => {
                    raw.push('$');
                    value.push('$');
                }
                'x' => {
                    raw.push('x');
                    self.advance();
                    if self.pos + 1 < self.source.len() {
                        let h1 = self.source[self.pos];
                        let h2 = self.source[self.pos + 1];
                        if let (Some(a), Some(b)) = (h1.to_digit(16), h2.to_digit(16)) {
                            raw.push(h1);
                            raw.push(h2);
                            let code = (a << 4) | b;
                            if let Some(ch) = char::from_u32(code) {
                                value.push(ch);
                            }
                            self.advance();
                            self.advance();
                            continue;
                        }
                    }
                    invalid_escape = true;
                    continue;
                }
                'u' => {
                    raw.push('u');
                    self.advance();
                    if self.pos < self.source.len() && self.source[self.pos] == '{' {
                        raw.push('{');
                        self.advance();
                        let mut code: u32 = 0;
                        let mut digits = 0;
                        let mut overflow = false;
                        while self.pos < self.source.len()
                            && self.source[self.pos] != '}'
                            && self.source[self.pos].is_ascii_hexdigit()
                        {
                            let d = self.source[self.pos].to_digit(16).unwrap();
                            code = code.saturating_mul(16).saturating_add(d);
                            if code > 0x10FFFF {
                                overflow = true;
                            }
                            digits += 1;
                            raw.push(self.source[self.pos]);
                            self.advance();
                        }
                        if self.pos < self.source.len()
                            && self.source[self.pos] == '}'
                            && digits > 0
                            && !overflow
                        {
                            raw.push('}');
                            self.advance();
                            if let Some(ch) = char::from_u32(code) {
                                value.push(ch);
                            }
                        } else {
                            invalid_escape = true;
                        }
                    } else {
                        let mut code: u32 = 0;
                        let mut digits = 0;
                        while digits < 4 && self.pos < self.source.len() {
                            let Some(d) = self.source[self.pos].to_digit(16) else {
                                break;
                            };
                            code = (code << 4) | d;
                            raw.push(self.source[self.pos]);
                            self.advance();
                            digits += 1;
                        }
                        if digits == 4 {
                            if let Some(ch) = char::from_u32(code) {
                                value.push(ch);
                            }
                        } else {
                            invalid_escape = true;
                        }
                    }
                    continue;
                }
                _ => {
                    raw.push(esc);
                    value.push(esc);
                }
            }
            self.advance();
        }

        (value, raw, false, invalid_escape)
    }

    pub fn scan_template_continuation(&mut self) -> Option<Token> {
        let (value, raw, terminated_by_interp, invalid_escape) =
            self.scan_template_segment();
        let token_type = if terminated_by_interp {
            TokenType::TemplateMiddle
        } else {
            TokenType::TemplateTail
        };
        self.last_token_kind = LastTokenKind::Dividend;
        Some(Token {
            token_type,
            value,
            raw,
            line: self.line,
            column: self.column,
            line_terminator_before: false,
            had_escape: false,
            had_legacy_escape: false,
            invalid_escape,
        })
    }

    pub fn source_from_pos(&self) -> String {
        self.source[self.pos..].iter().collect()
    }

    pub fn advance_char(&mut self) -> Option<char> {
        if self.pos < self.source.len() {
            let c = self.source[self.pos];
            self.advance();
            Some(c)
        } else {
            None
        }
    }

    pub fn at_str(&self, s: &str) -> bool {
        let chars: Vec<char> = s.chars().collect();
        if self.pos + chars.len() > self.source.len() {
            return false;
        }
        for (i, c) in chars.iter().enumerate() {
            if self.source[self.pos + i] != *c {
                return false;
            }
        }
        true
    }
}

fn is_radix_digit(radix: char, c: char) -> bool {
    match radix {
        'x' | 'X' => c.is_ascii_hexdigit(),
        'o' | 'O' => matches!(c, '0'..='7'),
        _ => matches!(c, '0' | '1'),
    }
}

fn valid_digit_run(s: &str) -> bool {
    !s.is_empty() && !s.starts_with('_') && !s.ends_with('_') && !s.contains("__")
}
