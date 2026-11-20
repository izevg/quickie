//! Tokenizer and recursive-descent parser for `when` clauses.

use std::iter::Peekable;
use std::str::CharIndices;

use super::{ContextKey, ContextValue, MAX_DEPTH, Node, WhenError};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Token<'a> {
    Ident(&'a str),
    Text(&'a str),
    Not,
    And,
    Or,
    Equals,
    NotEquals,
    Open,
    Close,
}

/// Parses a whole clause.
pub(super) fn parse(text: &str) -> Result<Node, WhenError> {
    let mut parser = Parser {
        tokens: tokenize(text)?.into_iter().peekable(),
        end: text.len(),
        depth: 0,
    };
    let node = parser.disjunction()?;
    match parser.next() {
        Some((offset, _)) => Err(WhenError::UnexpectedToken { offset }),
        None => Ok(node),
    }
}

fn tokenize(text: &str) -> Result<Vec<(usize, Token<'_>)>, WhenError> {
    let mut tokens = Vec::new();
    let mut chars = text.char_indices().peekable();
    while let Some((offset, c)) = chars.next() {
        let token = match c {
            c if c.is_whitespace() => continue,
            '(' => Token::Open,
            ')' => Token::Close,
            '!' if chars.next_if(|&(_, next)| next == '=').is_some() => Token::NotEquals,
            '!' => Token::Not,
            '=' => doubled(&mut chars, offset, '=', Token::Equals)?,
            '&' => doubled(&mut chars, offset, '&', Token::And)?,
            '|' => doubled(&mut chars, offset, '|', Token::Or)?,
            '\'' | '"' => Token::Text(string(text, &mut chars, offset, c)?),
            c if c.is_ascii_alphabetic() || c == '_' => {
                while chars
                    .next_if(|&(_, next)| is_ident_continue(next))
                    .is_some()
                {}
                let end = chars.peek().map_or(text.len(), |&(end, _)| end);
                Token::Ident(&text[offset..end])
            }
            found => return Err(WhenError::UnexpectedCharacter { offset, found }),
        };
        tokens.push((offset, token));
    }
    Ok(tokens)
}

/// A two-character operator such as `&&`: the second character must repeat the first.
fn doubled<'a>(
    chars: &mut Peekable<CharIndices<'_>>,
    offset: usize,
    c: char,
    token: Token<'a>,
) -> Result<Token<'a>, WhenError> {
    match chars.next_if(|&(_, next)| next == c) {
        Some(_) => Ok(token),
        None => Err(WhenError::UnexpectedCharacter { offset, found: c }),
    }
}

/// The text between the quote at `offset` and the next matching quote.
fn string<'a>(
    text: &'a str,
    chars: &mut Peekable<CharIndices<'_>>,
    offset: usize,
    quote: char,
) -> Result<&'a str, WhenError> {
    for (end, c) in chars.by_ref() {
        if c == quote {
            // Both quotes are one byte, so these are character boundaries.
            return Ok(&text[offset + 1..end]);
        }
    }
    Err(WhenError::UnterminatedString { offset })
}

fn is_ident_continue(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '.'
}

/// Consumes tokens from an iterator, so every step moves forward and no loop can spin in place.
struct Parser<'a> {
    tokens: Peekable<std::vec::IntoIter<(usize, Token<'a>)>>,
    end: usize,
    depth: usize,
}

impl<'a> Parser<'a> {
    fn next(&mut self) -> Option<(usize, Token<'a>)> {
        self.tokens.next()
    }

    /// Consumes the next token if it is `token`.
    fn eat(&mut self, token: Token<'_>) -> Option<(usize, Token<'a>)> {
        self.tokens.next_if(|&(_, next)| next == token)
    }

    /// `a || b || c`
    fn disjunction(&mut self) -> Result<Node, WhenError> {
        let first = self.conjunction()?;
        let mut operands = Vec::new();
        while self.eat(Token::Or).is_some() {
            operands.push(self.conjunction()?);
        }
        Ok(join(first, operands, Node::Or))
    }

    /// `a && b && c`
    fn conjunction(&mut self) -> Result<Node, WhenError> {
        let first = self.negation()?;
        let mut operands = Vec::new();
        while self.eat(Token::And).is_some() {
            operands.push(self.negation()?);
        }
        Ok(join(first, operands, Node::And))
    }

    /// `!!a`, counted in a loop so a long run of `!` cannot exhaust the stack.
    fn negation(&mut self) -> Result<Node, WhenError> {
        let mut nots = 0;
        while self.eat(Token::Not).is_some() {
            nots += 1;
        }
        let mut node = self.primary()?;
        for _ in 0..nots {
            node = Node::Not(Box::new(node));
        }
        Ok(node)
    }

    /// `(…)`, `key == 'v'`, `key != 'v'`, `key`, `true`, `false`
    fn primary(&mut self) -> Result<Node, WhenError> {
        match self.next() {
            Some((offset, Token::Open)) => {
                if self.depth == MAX_DEPTH {
                    return Err(WhenError::TooDeep { offset });
                }
                self.depth += 1;
                let inner = self.disjunction()?;
                self.depth -= 1;
                self.expect_close()?;
                Ok(inner)
            }
            Some((_, Token::Ident(key))) => self.comparison(key),
            Some((offset, _)) => Err(WhenError::UnexpectedToken { offset }),
            None => Err(WhenError::UnexpectedEnd { offset: self.end }),
        }
    }

    fn comparison(&mut self, name: &str) -> Result<Node, WhenError> {
        if self.eat(Token::Equals).is_some() {
            let key = ContextKey::of(name);
            let value = ContextValue::of(&key, self.expect_text()?);
            return Ok(Node::Equals { key, value });
        }
        if self.eat(Token::NotEquals).is_some() {
            let key = ContextKey::of(name);
            let value = ContextValue::of(&key, self.expect_text()?);
            return Ok(Node::NotEquals { key, value });
        }
        Ok(match name {
            "true" => Node::Literal(true),
            "false" => Node::Literal(false),
            _ => Node::Flag(ContextKey::of(name)),
        })
    }

    fn expect_text(&mut self) -> Result<String, WhenError> {
        match self.next() {
            Some((_, Token::Text(value))) => Ok(value.to_owned()),
            Some((offset, _)) => Err(WhenError::UnexpectedToken { offset }),
            None => Err(WhenError::UnexpectedEnd { offset: self.end }),
        }
    }

    fn expect_close(&mut self) -> Result<(), WhenError> {
        match self.next() {
            Some((_, Token::Close)) => Ok(()),
            Some((offset, _)) => Err(WhenError::UnexpectedToken { offset }),
            None => Err(WhenError::UnexpectedEnd { offset: self.end }),
        }
    }
}

/// One operand stands alone; more make a list under `make`.
fn join(first: Node, mut rest: Vec<Node>, make: fn(Vec<Node>) -> Node) -> Node {
    if rest.is_empty() {
        first
    } else {
        rest.insert(0, first);
        make(rest)
    }
}
