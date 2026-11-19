//! `when` clauses: the condition under which a binding applies, a subset of VS Code's.

mod parse;
#[cfg(test)]
mod tests;

use std::collections::{HashMap, HashSet};
use std::fmt;

/// How deep parentheses may nest; a guard against a `keybindings.json` that would exhaust the
/// stack.
const MAX_DEPTH: usize = 32;

/// A parsed `when` clause such as `pageFocused && !inputFocused`.
///
/// Grammar, loosest first: `||`, then `&&`, then `!`, then `key == 'value'` / `key != 'value'`,
/// identifiers, `true`, `false`, and parentheses. Two clauses are equal when they parse to the
/// same tree, so `a&&b` equals `a && b`; [`fmt::Display`] prints that tree canonically.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct WhenExpr(Node);

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Node {
    Literal(bool),
    Flag(String),
    Equals { key: String, value: String },
    NotEquals { key: String, value: String },
    Not(Box<Node>),
    And(Vec<Node>),
    Or(Vec<Node>),
}

impl WhenExpr {
    /// Whether the clause holds in `context`.
    #[must_use]
    pub fn evaluate(&self, context: &WhenContext) -> bool {
        self.0.evaluate(context)
    }
}

impl TryFrom<&str> for WhenExpr {
    type Error = WhenError;

    fn try_from(text: &str) -> Result<Self, Self::Error> {
        parse::parse(text).map(Self)
    }
}

impl fmt::Display for WhenExpr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl Node {
    fn evaluate(&self, context: &WhenContext) -> bool {
        match self {
            Self::Literal(value) => *value,
            Self::Flag(key) => context.is_truthy(key),
            Self::Equals { key, value } => context.value(key) == Some(value),
            Self::NotEquals { key, value } => context.value(key) != Some(value),
            Self::Not(inner) => !inner.evaluate(context),
            Self::And(operands) => operands.iter().all(|operand| operand.evaluate(context)),
            Self::Or(operands) => operands.iter().any(|operand| operand.evaluate(context)),
        }
    }

    /// Writes `self`, in parentheses when `parenthesize` says it would otherwise bind wrongly.
    fn fmt_operand(&self, f: &mut fmt::Formatter<'_>, parenthesize: bool) -> fmt::Result {
        if parenthesize {
            write!(f, "({self})")
        } else {
            write!(f, "{self}")
        }
    }

    fn fmt_joined(
        f: &mut fmt::Formatter<'_>,
        operands: &[Self],
        operator: &str,
        parenthesize: impl Fn(&Self) -> bool,
    ) -> fmt::Result {
        for (index, operand) in operands.iter().enumerate() {
            if index > 0 {
                f.write_str(operator)?;
            }
            operand.fmt_operand(f, parenthesize(operand))?;
        }
        Ok(())
    }
}

impl fmt::Display for Node {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Literal(value) => write!(f, "{value}"),
            Self::Flag(key) => f.write_str(key),
            Self::Equals { key, value } => write!(f, "{key} == {}", Quoted(value)),
            Self::NotEquals { key, value } => write!(f, "{key} != {}", Quoted(value)),
            Self::Not(inner) => {
                f.write_str("!")?;
                inner.fmt_operand(f, matches!(**inner, Self::And(_) | Self::Or(_)))
            }
            Self::And(operands) => Self::fmt_joined(f, operands, " && ", |operand| {
                matches!(operand, Self::And(_) | Self::Or(_))
            }),
            Self::Or(operands) => Self::fmt_joined(f, operands, " || ", |operand| {
                matches!(operand, Self::Or(_))
            }),
        }
    }
}

/// A string value in the quotes that can hold it: single quotes unless it has one itself.
struct Quoted<'a>(&'a str);

impl fmt::Display for Quoted<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let quote = if self.0.contains('\'') { '"' } else { '\'' };
        write!(f, "{quote}{}{quote}", self.0)
    }
}

/// What is true right now: the flags that are set and the string values of context keys, such as
/// `inputFocused` and `focusedView == 'palette'`. The shell fills it on each key event.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WhenContext {
    flags: HashSet<String>,
    values: HashMap<String, String>,
}

impl WhenContext {
    /// A context where nothing is set.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The same context with the flag `key` set.
    #[must_use]
    pub fn with_flag(mut self, key: impl Into<String>) -> Self {
        self.flags.insert(key.into());
        self
    }

    /// The same context with `key` holding `value`.
    #[must_use]
    pub fn with_value(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.values.insert(key.into(), value.into());
        self
    }

    /// A key is true when it is a set flag or holds a non-empty value, as in VS Code.
    fn is_truthy(&self, key: &str) -> bool {
        self.flags.contains(key) || self.value(key).is_some_and(|value| !value.is_empty())
    }

    fn value(&self, key: &str) -> Option<&String> {
        self.values.get(key)
    }
}

/// A `when` clause that does not parse. Each variant carries the byte offset where it went wrong,
/// so the keymap editor can point at it.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum WhenError {
    /// A character that starts no token, or a lone `&`, `|`, or `=`.
    #[error("unexpected `{found}` at offset {offset}")]
    UnexpectedCharacter {
        /// Byte offset of the character.
        offset: usize,
        /// The character.
        found: char,
    },
    /// A quote that is never closed.
    #[error("unterminated string starting at offset {offset}")]
    UnterminatedString {
        /// Byte offset of the opening quote.
        offset: usize,
    },
    /// A token where it cannot stand, such as `&&` at the start or a second identifier in a row.
    #[error("unexpected token at offset {offset}")]
    UnexpectedToken {
        /// Byte offset of the token.
        offset: usize,
    },
    /// The clause ends where an operand, a string, or `)` is still due.
    #[error("unexpected end of the clause at offset {offset}")]
    UnexpectedEnd {
        /// Byte offset of the end, the clause's length.
        offset: usize,
    },
    /// Parentheses nest deeper than Quickie allows.
    #[error("parentheses nest deeper than {MAX_DEPTH} at offset {offset}")]
    TooDeep {
        /// Byte offset of the parenthesis that goes too deep.
        offset: usize,
    },
}

impl WhenError {
    /// The byte offset in the clause where parsing failed.
    #[must_use]
    pub const fn offset(&self) -> usize {
        match *self {
            Self::UnexpectedCharacter { offset, .. }
            | Self::UnterminatedString { offset }
            | Self::UnexpectedToken { offset }
            | Self::UnexpectedEnd { offset }
            | Self::TooDeep { offset } => offset,
        }
    }
}
