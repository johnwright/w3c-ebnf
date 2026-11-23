use std::fmt;

/// A parse error with a human-readable message.
#[derive(Debug, Clone, PartialEq)]
pub struct ParseError {
    message: String,
}

impl ParseError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        ParseError {
            message: message.into(),
        }
    }

    /// A human-readable description of the error, without the "Parse error: "
    /// prefix that `Display` adds.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Parse error: {}", self.message)
    }
}

impl std::error::Error for ParseError {}

/// Result type for parsing operations.
pub(crate) type ParseResult<T> = Result<T, ParseError>;

/// Errors produced when validating a `Grammar`.
#[derive(Debug, Clone, PartialEq)]
pub enum GrammarError {
    /// A parse error occurred during tokenization or parsing.
    Parse(ParseError),

    /// The grammar contains no rules.
    EmptyGrammar,

    /// A rule name appears more than once.
    DuplicateRule(String),

    /// A symbol is referenced but never defined as a rule.
    UndefinedSymbol(String),

    /// A lexer rule references a parser rule.
    InvalidLexerReference {
        /// The name of the lexer rule containing the reference.
        lexer_rule: String,
        /// The name of the parser rule it references.
        parser_rule: String,
    },

    /// A parser rule uses the subtraction operator, which is only allowed in lexer rules.
    SubtractionInParserRule {
        /// The name of the parser rule.
        rule: String,
    },

    /// A rule can derive the empty string (epsilon).
    NullableRule {
        /// The name of the nullable rule.
        rule: String,
    },

    /// A rule can never derive a string of terminals (infinite recursion without progress).
    UnproductiveRule {
        /// The name of the unproductive rule.
        rule: String,
    },
}

impl From<ParseError> for GrammarError {
    fn from(e: ParseError) -> Self {
        GrammarError::Parse(e)
    }
}

impl fmt::Display for GrammarError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GrammarError::Parse(e) => write!(f, "{}", e),
            GrammarError::EmptyGrammar => write!(f, "Grammar contains no rules"),
            GrammarError::DuplicateRule(name) => {
                write!(f, "Duplicate rule definition: {}", name)
            }
            GrammarError::UndefinedSymbol(name) => {
                write!(f, "Undefined symbol: {}", name)
            }
            GrammarError::InvalidLexerReference { lexer_rule, parser_rule } => {
                write!(
                    f,
                    "Lexer rule '{}' references parser rule '{}'",
                    lexer_rule, parser_rule
                )
            }
            GrammarError::SubtractionInParserRule { rule } => {
                write!(
                    f,
                    "Parser rule '{}' uses subtraction, which is only allowed in lexer rules",
                    rule
                )
            }
            GrammarError::NullableRule { rule } => {
                write!(f, "Rule '{}' can derive the empty string", rule)
            }
            GrammarError::UnproductiveRule { rule } => {
                write!(f, "Rule '{}' can never derive a string of terminals", rule)
            }
        }
    }
}

impl std::error::Error for GrammarError {}
