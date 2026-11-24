use crate::Grammar;
use crate::ast::*;
use crate::lexer::{LexerError, Token, lexer};
use crate::result::{GrammarError, ParseError, ParseResult};

fn lexer_error_to_parse_error(e: LexerError) -> ParseError {
    match e {
        LexerError::MixedClassItems => {
            ParseError::new("Character class cannot mix hex codes and plain characters")
        }
        LexerError::InvalidCharCode(code) => {
            ParseError::new(format!("Invalid character code: {}", code))
        }
        LexerError::InvalidCharRange(range) => {
            ParseError::new(format!("Invalid character range: {}", range))
        }
        LexerError::EmptyCharClass(class) => {
            ParseError::new(format!("Empty character class: {}", class))
        }
        LexerError::InvalidChar(c) => ParseError::new(format!(
            "Invalid character U+{:04X} (use a hex code like #x{:X} instead)",
            c as u32, c as u32
        )),
        LexerError::Unknown =>ParseError::new("Unexpected character in input"),
    }
}

/// Parse W3C EBNF source text into a validated [`Grammar`].
///
/// The input is a sequence of rules of the form `name ::= expression`, with
/// optional whitespace and `/* ... */` comments between tokens.
///
/// After parsing, the rules are checked with the same validation as
/// [`Grammar::new`].
///
/// # Errors
///
/// Returns [`GrammarError::Parse`] if the input isn't valid W3C EBNF syntax,
/// or another [`GrammarError`] variant if the rules fail validation (for example,
/// an undefined symbol or a rule that can match the empty string).
///
/// # Example
///
/// ```
/// let grammar = w3c_ebnf::parse(r#"greeting ::= "hello" | "hi""#).unwrap();
/// assert_eq!(grammar.rules()[0].name, "greeting");
/// ```
pub fn parse(input: &str) -> Result<Grammar, GrammarError> {
    let mut parser = Parser::new(lexer(input));
    let mut rules = Vec::new();

    while !parser.at_end()? {
        let rule = parser.parse_rule()?;
        rules.push(rule);
    }

    Grammar::new(rules)
}

struct Parser<'a> {
    lex: logos::Lexer<'a, Token>,
    peek_cache: Option<Token>,
    peek_cache_2: Option<Token>,
}

impl<'a> Parser<'a> {
    fn new(lex: logos::Lexer<'a, Token>) -> Self {
        Parser {
            lex,
            peek_cache: None,
            peek_cache_2: None,
        }
    }

    /// Pull the next token from the lexer, converting any lexer error to a ParseError.
    fn next_token(&mut self) -> ParseResult<Token> {
        match self.lex.next() {
            Some(Ok(token)) => Ok(token),
            Some(Err(e)) => Err(lexer_error_to_parse_error(e)),
            None => Ok(Token::Eof),
        }
    }

    fn at_end(&mut self) -> ParseResult<bool> {
        // If we already have a cached peek, we're not at the end (unless it's EOF).
        if let Some(Token::Eof) = self.peek_cache {
            return Ok(true);
        }
        if self.peek_cache.is_some() {
            return Ok(false);
        }
        // Pull the next token to check.
        match self.next_token()? {
            Token::Eof => Ok(true),
            token => {
                self.peek_cache = Some(token);
                Ok(false)
            }
        }
    }

    fn peek(&mut self) -> ParseResult<Option<&Token>> {
        if self.peek_cache.is_none() {
            self.peek_cache = Some(self.next_token()?);
        }
        Ok(self.peek_cache.as_ref())
    }

    fn advance(&mut self) -> ParseResult<Token> {
        // If we have a first-level cache, promote the second-level cache first
        if let Some(token) = self.peek_cache.take() {
            self.peek_cache = self.peek_cache_2.take();
            Ok(token)
        } else if let Some(token) = self.peek_cache_2.take() {
            Ok(token)
        } else {
            self.next_token()
        }
    }

    fn expect(&mut self, expected: &Token) -> ParseResult<()> {
        let token = self.advance()?;
        if &token == expected {
            Ok(())
        } else {
            Err(ParseError::new(format!(
                "Expected {:?}, got {:?}",
                expected, token
            )))
        }
    }

    fn parse_rule(&mut self) -> ParseResult<Rule> {
        let name = match self.advance()? {
            Token::Name(n) => n,
            other => {
                return Err(ParseError::new(format!(
                    "Expected rule name, got {:?}",
                    other
                )));
            }
        };

        self.expect(&Token::ColonColonEquals)?;
        let expression = self.parse_expression()?;

        Ok(Rule { name, expression })
    }

    /// expression ::= subtraction ("|" subtraction)*
    fn parse_expression(&mut self) -> ParseResult<Expression> {
        let mut alts = vec![self.parse_subtraction()?];

        while self.peek()? == Some(&Token::Pipe) {
            self.advance()?;
            alts.push(self.parse_subtraction()?);
        }

        if alts.len() == 1 {
            Ok(alts.into_iter().next().unwrap())
        } else {
            Ok(Expression::Alt(alts))
        }
    }

    /// subtraction ::= concatenation ("-" concatenation)?
    fn parse_subtraction(&mut self) -> ParseResult<Expression> {
        let left = self.parse_concatenation()?;

        if self.peek()? == Some(&Token::Minus) {
            self.advance()?;
            let right = self.parse_concatenation()?;
            Ok(Expression::Sub {
                left: Box::new(left),
                right: Box::new(right),
            })
        } else {
            Ok(left)
        }
    }

    /// concatenation ::= term+
    fn parse_concatenation(&mut self) -> ParseResult<Expression> {
        let mut items = vec![self.parse_term()?];

        while !self.is_concat_boundary()? {
            items.push(self.parse_term()?);
        }

        if items.len() == 1 {
            Ok(items.into_iter().next().unwrap())
        } else {
            Ok(Expression::Concat(items))
        }
    }

    /// Check whether the current position is a boundary where concatenation should stop.
    fn is_concat_boundary(&mut self) -> ParseResult<bool> {
        match self.peek()? {
            Some(Token::Pipe)
            | Some(Token::RParen)
            | Some(Token::Star)
            | Some(Token::Plus)
            | Some(Token::Question)
            | Some(Token::Minus)
            | Some(Token::ColonColonEquals)
            | Some(Token::Eof) => Ok(true),
            // If the next token is a Name followed by ::= it's the start of a new rule
            Some(Token::Name(_)) => {
                // Two-token lookahead: check if the token after Name is ::=
                // peek_cache holds the Name; we need to look at the next token
                if self.peek_cache_2.is_none() {
                    self.peek_cache_2 = Some(self.next_token()?);
                }
                Ok(matches!(
                    self.peek_cache_2.as_ref(),
                    Some(&Token::ColonColonEquals)
                ))
            }
            _ => Ok(false),
        }
    }

    /// term ::= factor [*+?]?
    fn parse_term(&mut self) -> ParseResult<Expression> {
        let inner = self.parse_factor()?;
        match self.peek()? {
            Some(Token::Question) => {
                self.advance()?;
                Ok(Expression::Opt(Box::new(inner)))
            }
            Some(Token::Star) => {
                self.advance()?;
                Ok(Expression::Star(Box::new(inner)))
            }
            Some(Token::Plus) => {
                self.advance()?;
                Ok(Expression::Plus(Box::new(inner)))
            }
            _ => Ok(inner),
        }
    }

    /// factor ::= String | CharCode | CharClass | Symbol | "(" expression ")"
    fn parse_factor(&mut self) -> ParseResult<Expression> {
        let token = self.advance()?;
        match token {
            Token::Name(name) => Ok(Expression::Symbol(name)),
            Token::DoubleQuoted(s) => Ok(Expression::StringLiteral(s)),
            Token::SingleQuoted(s) => Ok(Expression::StringLiteral(s)),
            Token::CharCode(code) => Ok(Expression::CharCode(code)),
            Token::LParen => {
                let expr = self.parse_expression()?;
                self.expect(&Token::RParen)?;
                Ok(expr)
            }
            Token::CharClass(char_class) => Ok(Expression::CharClass(char_class)),
            other => Err(ParseError::new(format!("Expected factor, got {:?}", other))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::CharClass;

    #[test]
    fn simple_rule() {
        let grammar = parse(r#"Foo ::= "bar""#).unwrap();
        assert_eq!(grammar.rules().len(), 1);
        assert_eq!(grammar.rules()[0].name, "Foo");
        assert_eq!(
            grammar.rules()[0].expression,
            Expression::StringLiteral("bar".to_string())
        );
    }

    #[test]
    fn alternation() {
        let grammar = parse(r#"Foo ::= "a" | "b""#).unwrap();
        assert_eq!(grammar.rules()[0].name, "Foo");
        match &grammar.rules()[0].expression {
            Expression::Alt(alts) => assert_eq!(alts.len(), 2),
            other => panic!("Expected alternation, got {:?}", other),
        }
    }

    #[test]
    fn repetition() {
        let grammar = parse(r#"Foo ::= "a"+ "#).unwrap();
        assert_eq!(grammar.rules()[0].name, "Foo");
        match &grammar.rules()[0].expression {
            Expression::Plus(inner) => {
                assert_eq!(*inner.clone(), Expression::StringLiteral("a".to_string()));
            }
            other => panic!("Expected repetition, got {:?}", other),
        }
    }

    #[test]
    fn char_class() {
        let grammar = parse(r#"Foo ::= [a-zA-Z]"#).unwrap();
        assert_eq!(grammar.rules()[0].name, "Foo");
        match &grammar.rules()[0].expression {
            Expression::CharClass(CharClass { negated, items }) => {
                assert!(!negated);
                assert_eq!(items.len(), 2);
            }
            other => panic!("Expected CharClass, got {:?}", other),
        }
    }

    #[test]
    fn grouping() {
        let grammar = parse(r#"Foo ::= ("a" | "b")+"#).unwrap();
        assert_eq!(grammar.rules()[0].name, "Foo");
        match &grammar.rules()[0].expression {
            Expression::Plus(inner) => match inner.as_ref() {
                Expression::Alt(alts) => assert_eq!(alts.len(), 2),
                other => panic!("Expected alternation inside group, got {:?}", other),
            },
            other => panic!("Expected repetition, got {:?}", other),
        }
    }

    #[test]
    fn name_reference() {
        let grammar = parse(
            r#"foo ::= Bar Baz
            Bar ::= "a"
            Baz ::= "b"
        "#,
        )
        .unwrap();
        assert_eq!(grammar.rules()[0].name, "foo");
        match &grammar.rules()[0].expression {
            Expression::Concat(items) => {
                assert_eq!(items.len(), 2);
                assert_eq!(items[0], Expression::Symbol("Bar".to_string()));
                assert_eq!(items[1], Expression::Symbol("Baz".to_string()));
            }
            other => panic!("Expected concat, got {:?}", other),
        }
    }

    #[test]
    fn char_code() {
        let grammar = parse(r#"Foo ::= #x20"#).unwrap();
        assert_eq!(grammar.rules()[0].name, "Foo");
        assert_eq!(grammar.rules()[0].expression, Expression::CharCode(' '));
    }

    #[test]
    fn multiple_rules() {
        let grammar = parse(
            r#"
            Foo ::= "hello" | 'world'
            bar ::= Foo+
        "#,
        )
        .unwrap();
        assert_eq!(grammar.rules().len(), 2);
        assert_eq!(grammar.rules()[0].name, "Foo");
        assert_eq!(grammar.rules()[1].name, "bar");
    }

    #[test]
    fn comment_in_input() {
        let grammar = parse(r#"/* comment */ foo ::= "bar""#).unwrap();
        assert_eq!(grammar.rules().len(), 1);
        assert_eq!(grammar.rules()[0].name, "foo");
    }

    #[test]
    fn subtraction() {
        // Subtraction in a lexer rule (upper-case name) is allowed
        let grammar = parse(r#"Foo ::= [a-z] - [aeiou]"#).unwrap();
        assert_eq!(grammar.rules()[0].name, "Foo");
        match &grammar.rules()[0].expression {
            Expression::Sub { left, right } => {
                match left.as_ref() {
                    Expression::CharClass(CharClass { negated, .. }) => assert!(!*negated),
                    other => panic!("Expected CharClass on left, got {:?}", other),
                }
                match right.as_ref() {
                    Expression::CharClass(CharClass { negated, .. }) => assert!(!*negated),
                    other => panic!("Expected CharClass on right, got {:?}", other),
                }
            }
            other => panic!("Expected subtraction, got {:?}", other),
        }
    }

    #[test]
    fn invalid_char_code_surrogate() {
        // #xD800 is a Unicode surrogate, not a valid scalar value
        let result = parse(r#"Foo ::= #xD800"#);
        match result {
            Err(GrammarError::Parse(e)) => {
                assert_eq!(e.message(), "Invalid character code: #xD800")
            }
            other => panic!("Expected parse error, got {:?}", other),
        }
    }

    #[test]
    fn invalid_char_code_overflow() {
        // #x1FFFFFFFF exceeds u32::MAX
        let result = parse(r#"Foo ::= #x1FFFFFFFF"#);
        match result {
            Err(GrammarError::Parse(e)) => {
                assert_eq!(e.message(), "Invalid character code: #x1FFFFFFFF")
            }
            other => panic!("Expected parse error, got {:?}", other),
        }
    }

    #[test]
    fn invalid_char_code_above_max() {
        // #x110000 exceeds the maximum Unicode code point U+10FFFF
        let result = parse(r#"Foo ::= #x110000"#);
        match result {
            Err(GrammarError::Parse(e)) => {
                assert_eq!(e.message(), "Invalid character code: #x110000")
            }
            other => panic!("Expected parse error, got {:?}", other),
        }
    }

    #[test]
    fn invalid_char_range_plain() {
        // [z-a] — plain range where end < start
        let result = parse(r#"Foo ::= [z-a]"#);
        match result {
            Err(GrammarError::Parse(e)) => {
                assert_eq!(e.message(), "Invalid character range: [z-a]")
            }
            other => panic!("Expected parse error, got {:?}", other),
        }
    }

    #[test]
    fn invalid_char_range_hex() {
        // [#x7A-#x61] — hex range where end < start ('z'-'a')
        let result = parse(r#"Foo ::= [#x7A-#x61]"#);
        match result {
            Err(GrammarError::Parse(e)) => {
                assert_eq!(e.message(), "Invalid character range: [#x7A-#x61]")
            }
            other => panic!("Expected parse error, got {:?}", other),
        }
    }

    #[test]
    fn invalid_char_message() {
        // A form feed inside a string literal
        let result = parse("Foo ::= \"a\x0Cb\"");
        match result {
            Err(GrammarError::Parse(e)) => assert_eq!(
                e.message(),
                "Invalid character U+000C (use a hex code like #xC instead)"
            ),
            other => panic!("Expected parse error, got {:?}", other),
        }
    }
}
