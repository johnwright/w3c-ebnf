# w3c-ebnf

A Rust parser library for the **W3C variant of Extended Backus-Naur Form (EBNF)** as defined in the [XML 1.0 specification, Section 6 Notation](https://www.w3.org/TR/xml/#sec-notation).

This variant is used throughout W3C specifications to describe formal grammars. It differs from other EBNF/ABNF variants in its use of `|` for alternation, `?`/`*`/`+` for repetition, and `-` for subtraction.

## Grammar Rules

The W3C EBNF grammar itself, expressed in W3C EBNF:
```ebnf
grammar       ::= rule+
rule          ::= Symbol "::=" expression
expression    ::= subtraction ("|" subtraction)*
subtraction   ::= concatenation ("-" concatenation)?
concatenation ::= term+
term          ::= factor [*+?]?
factor        ::= String
                | CharCode
                | CharClass
                | Symbol
                | "(" expression ")"

Symbol            ::= SymbolStart SymbolChar*
SymbolStart       ::= [a-zA-Z_]
SymbolChar        ::= SymbolStart | [0-9]
String            ::= "'" (Char - "'")* "'"
                    | '"' (Char - '"')* '"'
CharCode          ::= '#' 'x' [0-9A-Fa-f]+
CharClass         ::= "[" "^"? ClassItems "]"
ClassItems        ::= CharCodeClassItem+
                    | CharClassItem+
CharCodeClassItem ::= CharCode ("-" CharCode)?
CharClassItem     ::= (Char - "]") ("-" (Char - "]"))?
Comment           ::= "/*" Char* "*/"
Char              ::= #x9 | #xA | #xD
                    | [#x20-#x7E]
                    | [#xA0-#xD7FF]
                    | [#xE000-#xFFFD]
                    | [#x10000-#x10FFFF]
```

### Operators and Their Precedence

*(Listed from lowest binding strength to highest)*

| Expression | Meaning |
|---|---|
| `A \| B` | Alternation — matches `A` or `B` (lowest precedence) |
| `A - B` | Subtraction — matches any string matching `A` but not `B` |
| `A B` | Concatenation — matches `A` followed by `B` |
| `A?` | Optional — matches `A` or nothing |
| `A*` | Zero or more occurrences of `A` |
| `A+` | One or more occurrences of `A` |
| `"string"` | A literal string matching the text inside double quotes |
| `'string'` | A literal string matching the text inside single quotes |
| `#xN` | The character whose code point is the hexadecimal integer `N` |
| `[a-zA-Z]` / `[#x61-#x7A]` | Any character in the indicated range(s) (inclusive) |
| `[abc]` / `[#x61#x62#x63]`| Any character among those enumerated |
| `[^a-z]` / `[^#x61-#x7A]`| Any character **outside** the indicated range |
| `[^abc]` / `[^#x61#x62#x63]` | Any character **not** among those enumerated |
| `/* ... */` | Comment (ignored at the lexer level) |

### Naming Conventions

- Symbols starting with an **upper-case** letter denote the start symbol of a regular language.
- Symbols starting with a **lower-case** letter denote other (non-terminal) symbols.

## Usage

```rust
use w3c_ebnf::parse;

let ebnf = r#"
    foo ::= "hello" | 'world'
    bar ::= foo+
"#;

let result = parse(ebnf);
// Returns a parsed representation of the grammar rules
```

## License

MIT
