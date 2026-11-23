use logos::Logos;

/// Errors produced by the lexer callbacks.
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) enum LexerError {
    /// Character class mixes hex codes and plain characters. In a hex class,
    /// a literal `-` or `^` counts as a plain character.
    MixedClassItems,
    /// Invalid Unicode scalar value.
    InvalidCharCode(String),
    /// Character range has end < start (e.g. `[z-a]`).
    InvalidCharRange(String),
    /// Character class has no items (e.g. `[]` or `[^]`).
    EmptyCharClass(String),
    /// Unrecognized character (default for unmatched input).
    #[default]
    Unknown,
}

/// Parsed character class data (negation flag and items).
#[derive(Debug, Clone, PartialEq)]
pub struct CharClass {
    /// `true` if the class starts with `^`, so it matches any character *not* listed.
    pub negated: bool,
    /// The characters and ranges listed in the class, in order. Never empty.
    pub items: Vec<ClassItem>,
}

/// An item inside a character class.
#[derive(Debug, Clone, PartialEq)]
pub enum ClassItem {
    /// A single character.
    Char(char),
    /// A range of characters, e.g. `a-z`.
    Range {
        /// The first character in the range (inclusive).
        start: char,
        /// The last character in the range (inclusive). Never less than `start`.
        end: char,
    },
}

/// Convert a `#xN` hex code (including the `#x` prefix) to a `char`.
///
/// Fails with `InvalidCharCode` if the value overflows `u32`, is a surrogate
/// (`#xD800`–`#xDFFF`), or is above `#x10FFFF`.
fn parse_hex_char(slice: &str) -> Result<char, LexerError> {
    let hex_str = &slice[2..]; // skip "#x"
    u32::from_str_radix(hex_str, 16)
        .ok()
        .and_then(char::from_u32)
        .ok_or_else(|| LexerError::InvalidCharCode(slice.to_string()))
}

// ——— Inner lexer for the content inside `[ ... ]` ———

/// Tokens for the mini-language inside a character class.
#[derive(Debug, Clone, Copy, PartialEq, Logos)]
#[logos(error = LexerError)]
enum ClassToken {
    /// Negation caret `^` (only meaningful at the start).
    #[token("^")]
    Negate,

    /// A `#xN` hex code point, validated as a Unicode scalar value.
    #[regex(r"#x[0-9A-Fa-f]+", |lex| parse_hex_char(lex.slice()))]
    HexCode(char),

    /// A range dash `-`.
    #[token("-")]
    Dash,

    /// Any other single character (including spaces, letters, digits, `#`, `x`, etc.),
    /// excluding `^` and `-` which are handled by other rules.
    /// Note: `#` and `x` are included here as a fallback — when the input is a
    /// complete hex code like `#x20`, the `HexCode` rule matches first (longer match).
    #[regex(r"[^\-^]", |lex| lex.slice().chars().next().unwrap())]
    Char(char),
}

/// Tokens produced by the lexer for W3C EBNF input.
#[derive(Debug, Clone, PartialEq, Logos)]
#[logos(error = LexerError)]
pub(crate) enum Token {
    /// Whitespace — skipped by the parser.
    #[regex(r"[ \t\n\r\f]+", logos::skip)]
    Whitespace,

    /// Comments `/* ... */` — skipped by the parser.
    #[regex(r"/\*[^*]*\*+([^/*][^*]*\*+)*/", logos::skip)]
    Comment,

    /// A symbol name (e.g. `Name`, `foo`).
    #[regex(r"[a-zA-Z_][a-zA-Z0-9_]*", |lex| lex.slice().to_string())]
    Name(String),

    /// The rule definition operator `::=`.
    #[token("::=")]
    ColonColonEquals,

    /// Alternation operator `|`.
    #[token("|")]
    Pipe,

    /// Optional operator `?`.
    #[token("?")]
    Question,

    /// Zero-or-more operator `*`.
    #[token("*")]
    Star,

    /// One-or-more operator `+`.
    #[token("+")]
    Plus,

    /// Subtraction operator `-`.
    #[token("-")]
    Minus,

    /// Left parenthesis `(`.
    #[token("(")]
    LParen,

    /// Right parenthesis `)`.
    #[token(")")]
    RParen,

    /// A character class, e.g. `[a-zA-Z]` or `[^0-9]`.
    #[regex(r"\[[^\]]*\]", Token::parse_char_class)]
    CharClass(CharClass),

    /// A Unicode code point, e.g. `#x20`.
    #[regex(r"#x[0-9A-Fa-f]+", Token::parse_hex_code)]
    CharCode(char),

    /// A double-quoted string literal.
    #[regex(r#""[^"]*""#, Token::parse_string_literal)]
    DoubleQuoted(String),

    /// A single-quoted string literal.
    #[regex(r"'[^']*'", Token::parse_string_literal)]
    SingleQuoted(String),

    /// End of input.
    Eof,
}

impl Token {
    /// Parse a character class from the matched slice (e.g. `[a-zA-Z]`).
    ///
    /// Uses a nested Logos lexer (`ClassToken`) to tokenize the interior, then parses:
    ///
    /// ```text
    /// CharClass ::= "[" "^"? ClassItems "]"
    /// ClassItems ::= CharCodeClassItem+ | CharClassItem+
    /// ```
    ///
    /// The first item decides which kind of class it is: a hex code starts a
    /// hex-only class, and anything else starts a plain-character class.
    fn parse_char_class(lex: &logos::Lexer<'_, Self>) -> Result<CharClass, LexerError> {
        let class = lex.slice();
        // Strip the surrounding `[` and `]`
        let inner = &class[1..class.len() - 1];

        // Any token error (e.g. an invalid hex code) fails the whole class.
        let tokens: Vec<ClassToken> = ClassToken::lexer(inner).collect::<Result<_, _>>()?;

        let (negated, rest) = match tokens.split_first() {
            Some((ClassToken::Negate, rest)) => (true, rest),
            _ => (false, tokens.as_slice()),
        };

        let items = match rest.first() {
            None => return Err(LexerError::EmptyCharClass(class.to_string())),
            Some(ClassToken::HexCode(_)) => Self::parse_hex_class_items(rest, class)?,
            Some(_) => Self::parse_plain_class_items(rest, class)?,
        };

        Ok(CharClass { negated, items })
    }

    /// Parse the items of a hex-only class:
    ///
    /// ```text
    /// CharCodeClassItem ::= CharCode ("-" CharCode)?
    /// ```
    ///
    /// Any other token, including a literal `-` or `^`, is plain-character notation
    /// and makes the class mixed.
    fn parse_hex_class_items(
        mut tokens: &[ClassToken],
        class: &str,
    ) -> Result<Vec<ClassItem>, LexerError> {
        use ClassToken::{Dash, HexCode};

        let mut items = Vec::new();
        loop {
            tokens = match tokens {
                [] => break,
                [HexCode(start), Dash, HexCode(end), rest @ ..] => {
                    items.push(Self::char_range(*start, *end, class)?);
                    rest
                }
                [HexCode(c), rest @ ..] => {
                    items.push(ClassItem::Char(*c));
                    rest
                }
                _ => return Err(LexerError::MixedClassItems),
            };
        }
        Ok(items)
    }

    /// Parse the items of a plain-character class:
    ///
    /// ```text
    /// CharClassItem ::= (Char - "]") ("-" (Char - "]"))?
    /// ```
    ///
    /// A `^` that isn't at the start and a `-` that isn't a range separator are
    /// literal characters, so they can also be range endpoints (e.g. `[--z]`,
    /// `[!-^]`). Any hex code makes the class mixed.
    fn parse_plain_class_items(
        mut tokens: &[ClassToken],
        class: &str,
    ) -> Result<Vec<ClassItem>, LexerError> {
        use ClassToken::{Dash, HexCode};

        let mut items = Vec::new();
        loop {
            tokens = match tokens {
                [] => break,
                [HexCode(_), ..] => return Err(LexerError::MixedClassItems),
                [start, Dash, end, rest @ ..] if !matches!(end, HexCode(_)) => {
                    items.push(Self::char_range(
                        Self::plain_char(*start),
                        Self::plain_char(*end),
                        class,
                    )?);
                    rest
                }
                [token, rest @ ..] => {
                    items.push(ClassItem::Char(Self::plain_char(*token)));
                    rest
                }
            };
        }
        Ok(items)
    }

    /// The character a token stands for in a plain-character class.
    /// Callers have already rejected hex codes.
    fn plain_char(token: ClassToken) -> char {
        match token {
            ClassToken::HexCode(c) | ClassToken::Char(c) => c,
            ClassToken::Dash => '-',
            ClassToken::Negate => '^',
        }
    }

    /// Build a range item, rejecting ranges whose end comes before their start.
    fn char_range(start: char, end: char, class: &str) -> Result<ClassItem, LexerError> {
        if start > end {
            return Err(LexerError::InvalidCharRange(class.to_string()));
        }
        Ok(ClassItem::Range { start, end })
    }

    fn parse_hex_code(lex: &logos::Lexer<'_, Self>) -> Result<char, LexerError> {
        parse_hex_char(lex.slice())
    }

    fn parse_string_literal(lex: &logos::Lexer<'_, Self>) -> String {
        let s = lex.slice();
        s[1..s.len() - 1].to_string()
    }
}

/// Create a new lexer from an input string.
pub(crate) fn lexer(input: &str) -> logos::Lexer<'_, Token> {
    Token::lexer(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_name() {
        let mut lex = lexer("Name");
        assert_eq!(lex.next(), Some(Ok(Token::Name("Name".to_string()))));
    }

    #[test]
    fn colon_equals() {
        let mut lex = lexer("::=");
        assert_eq!(lex.next(), Some(Ok(Token::ColonColonEquals)));
    }

    #[test]
    fn double_quoted() {
        let mut lex = lexer("\"hello world\"");
        assert_eq!(
            lex.next(),
            Some(Ok(Token::DoubleQuoted("hello world".to_string())))
        );
    }

    #[test]
    fn single_quoted() {
        let mut lex = lexer("'hello world'");
        assert_eq!(
            lex.next(),
            Some(Ok(Token::SingleQuoted("hello world".to_string())))
        );
    }

    #[test]
    fn char_code() {
        let mut lex = lexer("#x20");
        assert_eq!(lex.next(), Some(Ok(Token::CharCode(' '))));
    }

    #[test]
    fn char_code_overflow() {
        // #x1FFFFFFFF exceeds u32::MAX (0xFFFFFFFF), so u32::from_str_radix fails
        let input = "#x1FFFFFFFF";
        let mut lex = lexer(input);
        match lex.next() {
            Some(Err(LexerError::InvalidCharCode(code))) => {
                assert_eq!(code, input);
            }
            other => panic!("Expected InvalidCharCode error, got {:?}", other),
        }
    }

    #[test]
    fn char_code_surrogate() {
        // #xD800 is a valid u32 but a Unicode surrogate, not a valid scalar value
        let input = "#xD800";
        let mut lex = lexer(input);
        match lex.next() {
            Some(Err(LexerError::InvalidCharCode(code))) => {
                assert_eq!(code, input);
            }
            other => panic!("Expected InvalidCharCode error, got {:?}", other),
        }
    }

    #[test]
    fn char_code_above_max() {
        // #x110000 exceeds the maximum Unicode code point U+10FFFF
        let input = "#x110000";
        let mut lex = lexer(input);
        match lex.next() {
            Some(Err(LexerError::InvalidCharCode(code))) => {
                assert_eq!(code, input);
            }
            other => panic!("Expected InvalidCharCode error, got {:?}", other),
        }
    }

    #[test]
    fn comment_skipped() {
        let mut lex = lexer("/* this is a comment */ Name");
        assert_eq!(lex.next(), Some(Ok(Token::Name("Name".to_string()))));
    }

    #[test]
    fn full_production() {
        let mut lex = lexer("foo ::= \"bar\" | 'baz'");
        assert_eq!(lex.next(), Some(Ok(Token::Name("foo".to_string()))));
        assert_eq!(lex.next(), Some(Ok(Token::ColonColonEquals)));
        assert_eq!(lex.next(), Some(Ok(Token::DoubleQuoted("bar".to_string()))));
        assert_eq!(lex.next(), Some(Ok(Token::Pipe)));
        assert_eq!(lex.next(), Some(Ok(Token::SingleQuoted("baz".to_string()))));
    }

    #[test]
    fn char_class_token() {
        let mut lex = lexer("[a-zA-Z]");
        match lex.next() {
            Some(Ok(Token::CharClass(info))) => {
                assert!(!info.negated);
                assert_eq!(info.items.len(), 2);
            }
            other => panic!("Expected CharClass, got {:?}", other),
        }
    }

    #[test]
    fn char_class_with_space() {
        // Spaces inside character classes should be treated as literal chars, not skipped
        let mut lex = lexer("[a-z ]");
        match lex.next() {
            Some(Ok(Token::CharClass(info))) => {
                assert!(!info.negated);
                assert_eq!(
                    info.items,
                    vec![
                        ClassItem::Range {
                            start: 'a',
                            end: 'z'
                        },
                        ClassItem::Char(' '),
                    ]
                );
            }
            other => panic!("Expected CharClass, got {:?}", other),
        }
    }

    #[test]
    fn char_class_with_hex() {
        let mut lex = lexer("[#x20-#x7E]");
        match lex.next() {
            Some(Ok(Token::CharClass(info))) => {
                assert!(!info.negated);
                assert_eq!(
                    info.items,
                    vec![ClassItem::Range {
                        start: ' ',
                        end: '~'
                    }]
                );
            }
            other => panic!("Expected CharClass, got {:?}", other),
        }
    }

    #[test]
    fn char_class_mixed_rejected() {
        // Mixed hex codes and plain characters should be rejected
        let mut lex = lexer("[a-z #x20]");
        match lex.next() {
            Some(Err(_)) => { /* expected: mixed class is an error */ }
            other => panic!("Expected error for mixed class, got {:?}", other),
        }
    }

    /// Lex `input` and assert it fails with `MixedClassItems`.
    fn assert_mixed_class(input: &str) {
        let mut lex = lexer(input);
        match lex.next() {
            Some(Err(LexerError::MixedClassItems)) => {}
            other => panic!("Expected MixedClassItems error for {input}, got {:?}", other),
        }
    }

    #[test]
    fn hex_class_literal_dash_at_end() {
        assert_mixed_class("[#x41-]");
    }

    #[test]
    fn hex_class_literal_dash_at_start() {
        // Would otherwise be a range from '-' to #x41
        assert_mixed_class("[--#x41]");
    }

    #[test]
    fn hex_class_range_to_literal_caret() {
        // Would otherwise be a range from #x41 to '^'
        assert_mixed_class("[#x41-^]");
    }

    #[test]
    fn hex_class_literal_caret() {
        assert_mixed_class("[#x41^]");
    }

    #[test]
    fn hex_class_chained_ranges() {
        // `#x41-#x42` is a range, leaving a literal `-` before `#x43`
        assert_mixed_class("[#x41-#x42-#x43]");
    }

    #[test]
    fn negated_hex_class_with_range_and_code() {
        let mut lex = lexer("[^#x41-#x5A#x61]");
        match lex.next() {
            Some(Ok(Token::CharClass(info))) => {
                assert!(info.negated);
                assert_eq!(
                    info.items,
                    vec![
                        ClassItem::Range { start: 'A', end: 'Z' },
                        ClassItem::Char('a'),
                    ]
                );
            }
            other => panic!("Expected CharClass, got {:?}", other),
        }
    }

    #[test]
    fn negated_char_class() {
        let mut lex = lexer("[^a-z]");
        match lex.next() {
            Some(Ok(Token::CharClass(info))) => {
                assert!(info.negated);
                assert_eq!(
                    info.items,
                    vec![ClassItem::Range {
                        start: 'a',
                        end: 'z'
                    }]
                );
            }
            other => panic!("Expected CharClass, got {:?}", other),
        }
    }

    #[test]
    fn multiple_comments() {
        // Multiple comments should each be skipped independently
        let input = r#"/* comment 1 */ foo /* comment 2 */ bar"#;
        let tokens: Vec<_> = lexer(input).collect();
        let names: Vec<_> = tokens
            .into_iter()
            .filter_map(|r| {
                if let Ok(Token::Name(n)) = r {
                    Some(n)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(names, vec!["foo".to_string(), "bar".to_string()]);
    }

    #[test]
    fn char_class_hash_literal() {
        // [#] — hash as a plain character (not part of a hex code)
        let mut lex = lexer("[#]");
        match lex.next() {
            Some(Ok(Token::CharClass(info))) => {
                assert!(!info.negated);
                assert_eq!(info.items, vec![ClassItem::Char('#')]);
            }
            other => panic!("Expected CharClass, got {:?}", other),
        }
    }

    #[test]
    fn char_class_dash_literal() {
        // [-] — dash as a plain character
        let mut lex = lexer("[-]");
        match lex.next() {
            Some(Ok(Token::CharClass(info))) => {
                assert!(!info.negated);
                assert_eq!(info.items, vec![ClassItem::Char('-')]);
            }
            other => panic!("Expected CharClass, got {:?}", other),
        }
    }

    #[test]
    fn char_class_incomplete_hex() {
        // [#x] — incomplete hex code; both # and x treated as plain chars
        let mut lex = lexer("[#x]");
        match lex.next() {
            Some(Ok(Token::CharClass(info))) => {
                assert!(!info.negated);
                assert_eq!(info.items, vec![ClassItem::Char('#'), ClassItem::Char('x')]);
            }
            other => panic!("Expected CharClass, got {:?}", other),
        }
    }

    #[test]
    fn char_class_invalid_hex() {
        // [#xG] — invalid hex code; #, x and G treated as plain chars
        let mut lex = lexer("[#xG]");
        match lex.next() {
            Some(Ok(Token::CharClass(info))) => {
                assert!(!info.negated);
                assert_eq!(
                    info.items,
                    vec![
                        ClassItem::Char('#'),
                        ClassItem::Char('x'),
                        ClassItem::Char('G')
                    ]
                );
            }
            other => panic!("Expected CharClass, got {:?}", other),
        }
    }

    fn assert_invalid_char_code(input: &str, expected_code: &str) {
        let mut lex = lexer(input);
        match lex.next() {
            Some(Err(LexerError::InvalidCharCode(code))) => {
                assert_eq!(code, expected_code);
            }
            other => panic!("Expected InvalidCharCode error, got {:?}", other),
        }
    }

    #[test]
    fn char_class_hex_overflow() {
        // Exceeds u32::MAX; must not silently become #x0
        assert_invalid_char_code("[#x1FFFFFFFF]", "#x1FFFFFFFF");
    }

    #[test]
    fn char_class_hex_surrogate() {
        assert_invalid_char_code("[#xD800]", "#xD800");
    }

    #[test]
    fn char_class_hex_above_max() {
        assert_invalid_char_code("[#x110000]", "#x110000");
    }

    #[test]
    fn char_class_hex_range_invalid_start() {
        assert_invalid_char_code("[#x110000-#x10FFFF]", "#x110000");
    }

    #[test]
    fn char_class_hex_range_invalid_end() {
        assert_invalid_char_code("[#x41-#xD800]", "#xD800");
    }

    #[test]
    fn char_class_x_literal() {
        // [x] — the letter x as a plain character
        let mut lex = lexer("[x]");
        match lex.next() {
            Some(Ok(Token::CharClass(info))) => {
                assert!(!info.negated);
                assert_eq!(info.items, vec![ClassItem::Char('x')]);
            }
            other => panic!("Expected CharClass, got {:?}", other),
        }
    }

    #[test]
    fn char_class_dash_at_end() {
        // [a-] — dash at end treated as literal
        let mut lex = lexer("[a-]");
        match lex.next() {
            Some(Ok(Token::CharClass(info))) => {
                assert!(!info.negated);
                assert_eq!(info.items, vec![ClassItem::Char('a'), ClassItem::Char('-')]);
            }
            other => panic!("Expected CharClass, got {:?}", other),
        }
    }

    #[test]
    fn char_class_dash_at_start() {
        // [-a] — dash at start treated as literal
        let mut lex = lexer("[-a]");
        match lex.next() {
            Some(Ok(Token::CharClass(info))) => {
                assert!(!info.negated);
                assert_eq!(info.items, vec![ClassItem::Char('-'), ClassItem::Char('a')]);
            }
            other => panic!("Expected CharClass, got {:?}", other),
        }
    }

    /// Lex `input` as a single character class and return its items.
    fn class_items(input: &str) -> Vec<ClassItem> {
        let mut lex = lexer(input);
        match lex.next() {
            Some(Ok(Token::CharClass(info))) => info.items,
            other => panic!("Expected CharClass, got {:?}", other),
        }
    }

    #[test]
    fn char_class_range_starting_with_dash() {
        // [--z] — range from '-' to 'z'
        assert_eq!(
            class_items("[--z]"),
            vec![ClassItem::Range { start: '-', end: 'z' }]
        );
    }

    #[test]
    fn char_class_range_ending_with_dash() {
        // [!--] — range from '!' to '-'
        assert_eq!(
            class_items("[!--]"),
            vec![ClassItem::Range { start: '!', end: '-' }]
        );
    }

    #[test]
    fn char_class_range_ending_with_caret() {
        // [!-^] — a non-leading '^' is a literal, so it can end a range
        assert_eq!(
            class_items("[!-^]"),
            vec![ClassItem::Range { start: '!', end: '^' }]
        );
    }

    #[test]
    fn char_class_dash_after_range() {
        // [a-z-0] — dash after a complete range is a literal
        assert_eq!(
            class_items("[a-z-0]"),
            vec![
                ClassItem::Range { start: 'a', end: 'z' },
                ClassItem::Char('-'),
                ClassItem::Char('0'),
            ]
        );
    }

    #[test]
    fn char_class_reversed_range_ending_with_dash() {
        // [a--] — range from 'a' to '-', which is reversed ('a' > '-')
        let mut lex = lexer("[a--]");
        match lex.next() {
            Some(Err(LexerError::InvalidCharRange(s))) => assert_eq!(s, "[a--]"),
            other => panic!("Expected InvalidCharRange error, got {:?}", other),
        }
    }

    #[test]
    fn char_class_empty() {
        let mut lex = lexer("[]");
        match lex.next() {
            Some(Err(LexerError::EmptyCharClass(s))) => assert_eq!(s, "[]"),
            other => panic!("Expected EmptyCharClass error, got {:?}", other),
        }
    }

    #[test]
    fn char_class_empty_negated() {
        let mut lex = lexer("[^]");
        match lex.next() {
            Some(Err(LexerError::EmptyCharClass(s))) => assert_eq!(s, "[^]"),
            other => panic!("Expected EmptyCharClass error, got {:?}", other),
        }
    }

    #[test]
    fn char_class_caret_at_end() {
        // [x^] — caret at end is a literal, should match x or ^
        let mut lex = lexer("[x^]");
        match lex.next() {
            Some(Ok(Token::CharClass(info))) => {
                assert!(!info.negated);
                assert_eq!(info.items, vec![ClassItem::Char('x'), ClassItem::Char('^')]);
            }
            other => panic!("Expected CharClass, got {:?}", other),
        }
    }


    #[test]
    fn char_class_reversed_plain_range() {
        // [z-a] — plain range where end < start
        let input = "[z-a]";
        let mut lex = lexer(input);
        match lex.next() {
            Some(Err(LexerError::InvalidCharRange(s))) => {
                assert_eq!(s, input);
            }
            other => panic!("Expected InvalidRange error, got {:?}", other),
        }
    }

    #[test]
    fn char_class_reversed_hex_range() {
        // [#x7A-#x61] — hex range where end < start ('z'-'a')
        let input = "[#x7A-#x61]";
        let mut lex = lexer(input);
        match lex.next() {
            Some(Err(LexerError::InvalidCharRange(s))) => {
                assert_eq!(s, input);
            }
            other => panic!("Expected InvalidRange error, got {:?}", other),
        }
    }
}
