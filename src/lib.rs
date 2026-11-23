//! A parser for the W3C variant of Extended Backus-Naur Form (EBNF)
//! as defined in the [XML 1.0 specification, Section 6 Notation](https://www.w3.org/TR/xml/#sec-notation).
//!
//! This crate parses EBNF grammar definitions into an AST representation.
//!
//! # Example
//!
//! ```
//! use w3c_ebnf::parse;
//!
//! let ebnf = r#"
//!     foo ::= "hello" | 'world'
//!     bar ::= foo+
//! "#;
//!
//! let grammar = parse(ebnf).expect("failed to parse grammar");
//! assert_eq!(grammar.rules().len(), 2);
//! ```

#![warn(missing_docs)]

use std::collections::HashSet;

pub use ast::*;
pub use lexer::{CharClass, ClassItem};
pub use parser::parse;
pub use result::{GrammarError, ParseError};

mod lexer;
mod parser;
mod result;

/// Represents a grammar consisting of W3C EBNF production rules.
///
/// A `Grammar` can only be created through [`Grammar::new`] or [`parse`],
/// so its rules have always passed validation.
#[derive(Debug, Clone, PartialEq)]
pub struct Grammar {
    rules: Vec<Rule>,
}

impl Grammar {

    /// Create a new `Grammar` from production rules.
    ///
    /// Validates that:
    /// * There is at least one rule
    /// * All rule names are unique
    /// * All symbol references point to defined rules
    /// * Lexer rules do not reference parser rules
    /// * The subtraction operator is used only in lexer rules
    /// * No rules can derive the empty string (nullable rules)
    /// * No rules are unproductive (infinite recursion without terminals)
    pub fn new(rules: Vec<Rule>) -> Result<Self, GrammarError> {
        if rules.is_empty() {
            return Err(GrammarError::EmptyGrammar);
        }
        let defined = Self::check_duplicates(&rules)?;
        Self::check_undefined_symbols(&rules, &defined)?;
        Self::check_no_parser_refs_in_lexer_rules(&rules)?;
        Self::check_no_subtraction_in_parser_rules(&rules)?;
        Self::check_nullable_rules(&rules)?;
        Self::check_unproductive_rules(&rules)?;
        Ok(Self { rules })
    }

    /// The grammar's production rules, in the order they were defined.
    pub fn rules(&self) -> &[Rule] {
        &self.rules
    }

    fn check_duplicates(rules: &[Rule]) -> Result<HashSet<String>, GrammarError> {
        let mut defined = HashSet::new();
        for rule in rules {
            if !defined.insert(rule.name.clone()) {
                return Err(GrammarError::DuplicateRule(rule.name.clone()));
            }
        }
        Ok(defined)
    }

    fn check_undefined_symbols(
        rules: &[Rule],
        defined: &HashSet<String>,
    ) -> Result<(), GrammarError> {
        for rule in rules {
            for symbol in rule.expression.collect_symbols() {
                if !defined.contains(symbol.as_str()) {
                    return Err(GrammarError::UndefinedSymbol(symbol));
                }
            }
        }
        Ok(())
    }

    fn check_no_parser_refs_in_lexer_rules(rules: &[Rule]) -> Result<(), GrammarError> {
        for rule in rules {
            if !rule.is_lexer_rule() {
                continue;
            }
            for symbol in rule.expression.collect_symbols() {
                if !Rule::is_lexer_name(&symbol) {
                    return Err(GrammarError::InvalidLexerReference {
                        lexer_rule: rule.name.clone(),
                        parser_rule: symbol,
                    });
                }
            }
        }
        Ok(())
    }

    fn check_no_subtraction_in_parser_rules(rules: &[Rule]) -> Result<(), GrammarError> {
        for rule in rules {
            if rule.is_lexer_rule() {
                continue;
            }
            if rule.expression.contains_subtraction() {
                return Err(GrammarError::SubtractionInParserRule {
                    rule: rule.name.clone(),
                });
            }
        }
        Ok(())
    }

    fn check_nullable_rules(rules: &[Rule]) -> Result<(), GrammarError> {
        let nullable = Self::fixed_point(rules, |expr, known| expr.is_nullable(known));
        for rule in rules {
            if nullable.contains(rule.name.as_str()) {
                return Err(GrammarError::NullableRule {
                    rule: rule.name.clone(),
                });
            }
        }
        Ok(())
    }

    fn check_unproductive_rules(rules: &[Rule]) -> Result<(), GrammarError> {
        let productive = Self::fixed_point(rules, |expr, known| expr.is_productive(known));
        for rule in rules {
            if !productive.contains(rule.name.as_str()) {
                return Err(GrammarError::UnproductiveRule {
                    rule: rule.name.clone(),
                });
            }
        }
        Ok(())
    }


    /// Iteratively compute a property over grammar rules until a fixed point is reached.
    ///
    /// `predicate` determines whether a rule belongs in the known set.
    fn fixed_point(
        rules: &[Rule],
        predicate: fn(&Expression, &HashSet<String>) -> bool,
    ) -> HashSet<String> {
        let mut known = HashSet::new();
        loop {
            let mut changed = false;
            for rule in rules {
                if known.contains(rule.name.as_str()) {
                    continue;
                }
                if predicate(&rule.expression, &known) {
                    known.insert(rule.name.clone());
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        known
    }
}

/// AST nodes representing parsed EBNF expressions.
mod ast {
    use std::collections::HashSet;

    use crate::lexer::CharClass;

    /// A single grammar production rule.
    #[derive(Debug, Clone, PartialEq)]
    pub struct Rule {
        /// The name of the symbol being defined.
        pub name: String,
        /// The right-hand side expression.
        pub expression: Expression,
    }

    impl Rule {
        /// Returns `true` if the rule name starts with an upper-case letter,
        /// indicating a lexer rule.
        pub fn is_lexer_rule(&self) -> bool {
            Self::is_lexer_name(&self.name)
        }


        /// Returns `true` if a rule name starts with an upper-case letter,
        /// indicating a lexer rule.
        pub fn is_lexer_name(name: &str) -> bool {
            name.chars().next().map(|c| c.is_ascii_uppercase()).unwrap_or(false)
        }
    }

    /// An EBNF expression.
    #[derive(Debug, Clone, PartialEq)]
    pub enum Expression {
        /// A reference to another symbol (e.g. `Name`, `foo`).
        Symbol(String),

        /// A literal string in double or single quotes.
        StringLiteral(String),

        /// A character class, e.g. `[a-zA-Z]` or `[^0-9]`.
        CharClass(CharClass),

        /// A Unicode code point, e.g. `#x20`.
        CharCode(char),

        /// Alternation: `A | B | C`
        Alt(Vec<Expression>),

        /// Concatenation: `A B C` (implicit, via adjacency)
        Concat(Vec<Expression>),

        /// Subtraction: `A - B`
        Sub {
            /// The expression to match (`A`).
            left: Box<Expression>,
            /// The expression whose matches are excluded (`B`).
            right: Box<Expression>,
        },

        /// Optional: `A?`
        Opt(Box<Expression>),

        /// Kleene star (zero or more): `A*`
        Star(Box<Expression>),

        /// Kleene plus (one or more): `A+`
        Plus(Box<Expression>),
    }

    impl Expression {
        /// Collects all symbols from this expression into a new `Vec<String>`.
        pub fn collect_symbols(&self) -> Vec<String> {
            let mut symbols = Vec::new();
            self.collect_symbols_into(&mut symbols);
            symbols
        }

        fn collect_symbols_into(&self, out: &mut Vec<String>) {
            match self {
                Expression::Symbol(name) => out.push(name.clone()),
                Expression::Alt(items) | Expression::Concat(items) => {
                    for item in items {
                        item.collect_symbols_into(out);
                    }
                }
                Expression::Sub { left, right } => {
                    left.collect_symbols_into(out);
                    right.collect_symbols_into(out);
                }
                Expression::Opt(inner) | Expression::Star(inner) | Expression::Plus(inner) => {
                    inner.collect_symbols_into(out);
                }
                Expression::StringLiteral(_)
                | Expression::CharClass(_)
                | Expression::CharCode(_) => {}
            }
        }

        /// Returns `true` if the expression contains a subtraction (`-`) operator.
        pub fn contains_subtraction(&self) -> bool {
            match self {
                Expression::Sub { .. } => true,
                Expression::Alt(items) | Expression::Concat(items) => {
                    items.iter().any(|item| item.contains_subtraction())
                }
                Expression::Opt(inner) | Expression::Star(inner) | Expression::Plus(inner) => {
                    inner.contains_subtraction()
                }
                Expression::Symbol(_)
                | Expression::StringLiteral(_)
                | Expression::CharClass(_)
                | Expression::CharCode(_) => false,
            }
        }

        pub(crate) fn is_nullable(&self, nullable_rules: &HashSet<String>) -> bool {
            match self {
                Expression::Symbol(name) => nullable_rules.contains(name.as_str()),
                Expression::StringLiteral(s) => s.is_empty(),
                Expression::CharClass(_) | Expression::CharCode(_) => false,
                Expression::Opt(_) | Expression::Star(_) => true,
                Expression::Plus(inner) => inner.is_nullable(nullable_rules),
                Expression::Alt(items) => items.iter().any(|item| item.is_nullable(nullable_rules)),
                Expression::Concat(items) => items.iter().all(|item| item.is_nullable(nullable_rules)),
                Expression::Sub { left, right } => {
                    // Rules referenced from `right` are treated as non-nullable (empty set) rather than
                    // looked up in `nullable_rules`, which grows during the fixed-point loop and would
                    // make the result depend on rule order. If such a rule is nullable, it fails this
                    // check itself, so the result is exact for any grammar that passes validation.
                    left.is_nullable(nullable_rules) && !right.is_nullable(&HashSet::new())
                }
            }
        }

        pub(crate) fn is_productive(&self, productive_rules: &HashSet<String>) -> bool {
            match self {
                Expression::Symbol(name) => productive_rules.contains(name.as_str()),
                Expression::StringLiteral(_) | Expression::CharClass(_) | Expression::CharCode(_) => {
                    true
                }
                Expression::Opt(_) | Expression::Star(_) => true,
                Expression::Plus(inner) => inner.is_productive(productive_rules),
                Expression::Alt(items) => {
                    items.iter().any(|item| item.is_productive(productive_rules))
                }
                Expression::Concat(items) => {
                    items.iter().all(|item| item.is_productive(productive_rules))
                }
                Expression::Sub { left, right: _ } => left.is_productive(productive_rules),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{GrammarError, parse};

    mod duplicates {
        use super::*;

        #[test]
        fn duplicate_rule_name() {
            let result = parse(
                r#"foo ::= "a"
                foo ::= "b"
            "#,
            );
            match result {
                Err(GrammarError::DuplicateRule(name)) => {
                    assert_eq!(name, "foo")
                }
                other => panic!("Expected duplicate rule error, got {:?}", other),
            }
        }
    }

    mod undefined_symbols {
        use super::*;

        #[test]
        fn undefined_symbol() {
            let result = parse(r#"foo ::= Bar"#);
            match result {
                Err(GrammarError::UndefinedSymbol(name)) => {
                    assert_eq!(name, "Bar")
                }
                other => panic!("Expected undefined symbol error, got {:?}", other),
            }
        }

        #[test]
        fn undefined_symbol_in_alternation() {
            let result = parse(r#"foo ::= "a" | Undefined"#);
            match result {
                Err(GrammarError::UndefinedSymbol(name)) => {
                    assert_eq!(name, "Undefined")
                }
                other => panic!("Expected undefined symbol error, got {:?}", other),
            }
        }

        #[test]
        fn undefined_symbol_in_concatenation() {
            let result = parse(r#"foo ::= "a" Missing "b""#);
            match result {
                Err(GrammarError::UndefinedSymbol(name)) => {
                    assert_eq!(name, "Missing")
                }
                other => panic!("Expected undefined symbol error, got {:?}", other),
            }
        }

        #[test]
        fn undefined_symbol_in_repetition() {
            let result = parse(r#"foo ::= Gone+"#);
            match result {
                Err(GrammarError::UndefinedSymbol(name)) => {
                    assert_eq!(name, "Gone")
                }
                other => panic!("Expected undefined symbol error, got {:?}", other),
            }
        }

        #[test]
        fn defined_symbol_is_ok() {
            let result = parse(
                r#"foo ::= Bar
                Bar ::= "a"
            "#,
            );
            assert!(result.is_ok(), "Expected Ok, got {:?}", result.err());
        }
    }

    mod lexer_parser_refs {
        use super::*;

        #[test]
        fn lexer_rule_references_parser_rule() {
            let result = parse(
                r#"Foo ::= bar
                bar ::= "a"
            "#,
            );
            match result {
                Err(GrammarError::InvalidLexerReference {
                    lexer_rule,
                    parser_rule,
                }) => {
                    assert_eq!(lexer_rule, "Foo");
                    assert_eq!(parser_rule, "bar");
                }
                other => panic!("Expected InvalidLexerReference error, got {:?}", other),
            }
        }

        #[test]
        fn parser_rule_references_lexer_rule() {
            let result = parse(
                r#"foo ::= Bar
                Bar ::= "a"
            "#,
            );
            assert!(result.is_ok(), "Expected Ok, got {:?}", result.err());
        }

        #[test]
        fn lexer_rule_references_only_lexer_rules() {
            let result = parse(
                r#"Foo ::= Bar Baz
                Bar ::= "a"
                Baz ::= "b"
            "#,
            );
            assert!(result.is_ok(), "Expected Ok, got {:?}", result.err());
        }
    }

    mod subtraction {
        use super::*;

        #[test]
        fn subtraction_in_parser_rule() {
            let result = parse(r#"foo ::= [a-z] - [aeiou]"#);
            match result {
                Err(GrammarError::SubtractionInParserRule { rule }) => {
                    assert_eq!(rule, "foo");
                }
                other => panic!("Expected SubtractionInParserRule error, got {:?}", other),
            }
        }

        #[test]
        fn subtraction_in_lexer_rule() {
            let result = parse(r#"Foo ::= [a-z] - [aeiou]"#);
            assert!(result.is_ok(), "Expected Ok, got {:?}", result.err());
        }

        #[test]
        fn subtraction_nested_in_parser_rule() {
            let result = parse(r#"foo ::= ([a-z] - [aeiou])+"#);
            match result {
                Err(GrammarError::SubtractionInParserRule { rule }) => {
                    assert_eq!(rule, "foo");
                }
                other => panic!("Expected SubtractionInParserRule error, got {:?}", other),
            }
        }

        #[test]
        fn subtraction_in_alternation_of_parser_rule() {
            let result = parse(r#"foo ::= "a" | ([b-z] - [m-z])"#);
            match result {
                Err(GrammarError::SubtractionInParserRule { rule }) => {
                    assert_eq!(rule, "foo");
                }
                other => panic!("Expected SubtractionInParserRule error, got {:?}", other),
            }
        }
    }

    mod nullable_rules {
        use super::*;

        #[test]
        fn nullable_rule_optional() {
            let result = parse(
                r#"Foo ::= Bar?
                Bar ::= "a"?
            "#,
            );
            match result {
                Err(GrammarError::NullableRule { rule }) => {
                    assert_eq!(rule, "Foo");
                }
                other => panic!("Expected NullableRule error, got {:?}", other),
            }
        }

        #[test]
        fn nullable_rule_star() {
            let result = parse(r#"Foo ::= "a"*"#);
            match result {
                Err(GrammarError::NullableRule { rule }) => {
                    assert_eq!(rule, "Foo");
                }
                other => panic!("Expected NullableRule error, got {:?}", other),
            }
        }

        #[test]
        fn nullable_rule_empty_string() {
            let result = parse(r#"Foo ::= """#);
            match result {
                Err(GrammarError::NullableRule { rule }) => {
                    assert_eq!(rule, "Foo");
                }
                other => panic!("Expected NullableRule error, got {:?}", other),
            }
        }

        #[test]
        fn nullable_rule_concatenation() {
            let result = parse(
                r#"Foo ::= Bar Baz
                Bar ::= "a"?
                Baz ::= "b"?
            "#,
            );
            match result {
                Err(GrammarError::NullableRule { rule }) => {
                    assert_eq!(rule, "Foo");
                }
                other => panic!("Expected NullableRule error, got {:?}", other),
            }
        }

        #[test]
        fn non_nullable_rule_plus() {
            let result = parse(r#"Foo ::= "a"+"#);
            assert!(result.is_ok(), "Expected Ok, got {:?}", result.err());
        }

        #[test]
        fn non_nullable_rule_literal() {
            let result = parse(r#"Foo ::= "a""#);
            assert!(result.is_ok(), "Expected Ok, got {:?}", result.err());
        }

        #[test]
        fn non_nullable_subtraction_removes_empty_string() {
            // [a-z]* matches "", but "" is subtracted away
            let result = parse(r#"Foo ::= [a-z]* - """#);
            assert!(result.is_ok(), "Expected Ok, got {:?}", result.err());
        }

        #[test]
        fn non_nullable_subtraction_of_optional() {
            // Both sides match "", so the difference doesn't
            let result = parse(r#"Foo ::= [a-z]* - "x"?"#);
            assert!(result.is_ok(), "Expected Ok, got {:?}", result.err());
        }

        #[test]
        fn nullable_subtraction() {
            // [a-z]* matches "" and [0-9] doesn't, so the difference does
            let result = parse(r#"Foo ::= [a-z]* - [0-9]"#);
            match result {
                Err(GrammarError::NullableRule { rule }) => {
                    assert_eq!(rule, "Foo");
                }
                other => panic!("Expected NullableRule error, got {:?}", other),
            }
        }
    }

    mod empty_grammar {
        use super::*;
        use crate::Grammar;

        #[test]
        fn empty_input() {
            assert_eq!(parse("").err(), Some(GrammarError::EmptyGrammar));
        }

        #[test]
        fn only_whitespace_and_comments() {
            assert_eq!(
                parse("  /* no rules here */\n").err(),
                Some(GrammarError::EmptyGrammar)
            );
        }

        #[test]
        fn grammar_new_with_no_rules() {
            assert_eq!(Grammar::new(vec![]).err(), Some(GrammarError::EmptyGrammar));
        }
    }

    mod unproductive_rules {
        use super::*;

        #[test]
        fn unproductive_rule_self_recursion() {
            let result = parse(r#"foo ::= foo"#);
            match result {
                Err(GrammarError::UnproductiveRule { rule }) => {
                    assert_eq!(rule, "foo");
                }
                other => panic!("Expected UnproductiveRule error, got {:?}", other),
            }
        }

        #[test]
        fn unproductive_rule_mutual_recursion() {
            let result = parse(
                r#"foo ::= bar
                bar ::= foo
            "#,
            );
            match result {
                Err(GrammarError::UnproductiveRule { rule }) => {
                    assert_eq!(rule, "foo");
                }
                other => panic!("Expected UnproductiveRule error, got {:?}", other),
            }
        }

        #[test]
        fn productive_rule_with_base_case() {
            let result = parse(r#"foo ::= "a" | foo "b""#);
            assert!(result.is_ok(), "Expected Ok, got {:?}", result.err());
        }

        #[test]
        fn productive_rule_simple() {
            let result = parse(r#"foo ::= "hello""#);
            assert!(result.is_ok(), "Expected Ok, got {:?}", result.err());
        }

        #[test]
        fn unproductive_rule_concatenation_with_recursion() {
            // Every expansion of `foo` contains another `foo`, so it never terminates,
            // even though the concatenation also contains a terminal.
            let result = parse(r#"foo ::= "a" foo"#);
            match result {
                Err(GrammarError::UnproductiveRule { rule }) => {
                    assert_eq!(rule, "foo");
                }
                other => panic!("Expected UnproductiveRule error, got {:?}", other),
            }
        }

        #[test]
        fn productive_rule_concatenation() {
            let result = parse(
                r#"foo ::= "a" bar
                bar ::= "b"
            "#,
            );
            assert!(result.is_ok(), "Expected Ok, got {:?}", result.err());
        }

        #[test]
        fn unproductive_rule_plus_with_recursion() {
            // `foo+` needs at least one `foo`, which is itself unproductive.
            let result = parse(r#"foo ::= foo+"#);
            match result {
                Err(GrammarError::UnproductiveRule { rule }) => {
                    assert_eq!(rule, "foo");
                }
                other => panic!("Expected UnproductiveRule error, got {:?}", other),
            }
        }

        #[test]
        fn unproductive_rule_subtraction_with_recursion() {
            // `A - B` only produces strings that `A` produces, so a productive
            // right-hand side doesn't make the rule productive.
            let result = parse(r#"Foo ::= Foo - "a""#);
            match result {
                Err(GrammarError::UnproductiveRule { rule }) => {
                    assert_eq!(rule, "Foo");
                }
                other => panic!("Expected UnproductiveRule error, got {:?}", other),
            }
        }

        #[test]
        fn productive_rule_subtraction() {
            let result = parse(
                r#"Foo ::= Bar - "x"
                Bar ::= [a-z]
            "#,
            );
            assert!(result.is_ok(), "Expected Ok, got {:?}", result.err());
        }
    }

    mod readme_grammar {
        use super::*;

        #[test]
        fn readme_meta_grammar_parses() {
            let readme = std::fs::read_to_string("README.md")
                .expect("README.md should exist");
            let grammar_text = extract_grammar_from_readme(&readme);
            let grammar = parse(&grammar_text);
            assert!(grammar.is_ok(), "README grammar should parse and validate");
        }

        fn extract_grammar_from_readme(readme: &str) -> String {
            use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};

            let parser = Parser::new_ext(readme, Options::all());
            let mut in_grammar_block = false;
            let mut grammar = String::new();

            for event in parser {
                match event {
                    Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(lang))) => {
                        if lang.as_ref() == "ebnf" {
                            in_grammar_block = true;
                        }
                    }
                    Event::End(TagEnd::CodeBlock) => {
                        in_grammar_block = false;
                    }
                    Event::Text(text) if in_grammar_block => {
                        grammar.push_str(&text);
                    }
                    _ => {}
                }
            }

            grammar
        }
    }
}
