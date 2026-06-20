use super::diagnostics::Diagnostic;
use super::span::SourceSpan;

#[derive(Debug, Clone, PartialEq)]
pub struct Token {
  pub kind: TokenKind,
  pub span: SourceSpan,
}

impl Token {
  fn new(kind: TokenKind, start: usize, end: usize) -> Self {
    Self {
      kind,
      span: SourceSpan::new(start, end),
    }
  }
}

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
  Identifier(String),
  String(String),
  Int(i64),
  Float(f64),
  True,
  False,
  Null,
  Dot,
  Comma,
  LParen,
  RParen,
  LBracket,
  RBracket,
  Bang,
  Minus,
  Plus,
  Star,
  Slash,
  Percent,
  EqEq,
  Ne,
  Lt,
  Le,
  Gt,
  Ge,
  AndAnd,
  OrOr,
  Eof,
}

pub fn tokenize(input: &str) -> Result<Vec<Token>, Vec<Diagnostic>> {
  let mut lexer = Lexer {
    input,
    position: 0,
    diagnostics: Vec::new(),
    tokens: Vec::new(),
  };
  lexer.run();
  if lexer.diagnostics.is_empty() {
    Ok(lexer.tokens)
  } else {
    Err(lexer.diagnostics)
  }
}

struct Lexer<'a> {
  input: &'a str,
  position: usize,
  diagnostics: Vec<Diagnostic>,
  tokens: Vec<Token>,
}

impl Lexer<'_> {
  fn run(&mut self) {
    while let Some(ch) = self.peek_char() {
      match ch {
        ch if ch.is_whitespace() => {
          self.advance_char();
        }
        '/' if self.peek_next_char() == Some('/') => self.skip_line_comment(),
        '"' | '\'' => self.lex_string(ch),
        '0'..='9' => self.lex_number(),
        'A'..='Z' | 'a'..='z' | '_' => self.lex_identifier(),
        '.' => self.push_simple(TokenKind::Dot),
        ',' => self.push_simple(TokenKind::Comma),
        '(' => self.push_simple(TokenKind::LParen),
        ')' => self.push_simple(TokenKind::RParen),
        '[' => self.push_simple(TokenKind::LBracket),
        ']' => self.push_simple(TokenKind::RBracket),
        '-' => self.push_simple(TokenKind::Minus),
        '+' => self.push_simple(TokenKind::Plus),
        '*' => self.push_simple(TokenKind::Star),
        '%' => self.push_simple(TokenKind::Percent),
        '/' => self.push_simple(TokenKind::Slash),
        '!' if self.peek_next_char() == Some('=') => self.push_two(TokenKind::Ne),
        '!' => self.push_simple(TokenKind::Bang),
        '=' if self.peek_next_char() == Some('=') => self.push_two(TokenKind::EqEq),
        '=' => self.invalid_char(ch),
        '<' if self.peek_next_char() == Some('=') => self.push_two(TokenKind::Le),
        '<' => self.push_simple(TokenKind::Lt),
        '>' if self.peek_next_char() == Some('=') => self.push_two(TokenKind::Ge),
        '>' => self.push_simple(TokenKind::Gt),
        '&' if self.peek_next_char() == Some('&') => self.push_two(TokenKind::AndAnd),
        '&' => self.invalid_char(ch),
        '|' if self.peek_next_char() == Some('|') => self.push_two(TokenKind::OrOr),
        '|' => self.invalid_char(ch),
        other => self.invalid_char(other),
      }
    }
    self
      .tokens
      .push(Token::new(TokenKind::Eof, self.position, self.position));
  }

  fn lex_string(&mut self, quote: char) {
    let start = self.position;
    self.advance_char();
    let mut value = String::new();

    while let Some(ch) = self.peek_char() {
      if ch == quote {
        self.advance_char();
        self
          .tokens
          .push(Token::new(TokenKind::String(value), start, self.position));
        return;
      }

      if ch == '\\' {
        self.advance_char();
        let Some(escaped) = self.peek_char() else {
          break;
        };
        self.advance_char();
        match escaped {
          '\\' => value.push('\\'),
          '"' => value.push('"'),
          '\'' => value.push('\''),
          'n' => value.push('\n'),
          'r' => value.push('\r'),
          't' => value.push('\t'),
          other => value.push(other),
        }
      } else {
        value.push(ch);
        self.advance_char();
      }
    }

    self.diagnostics.push(Diagnostic::new(
      "unterminated string literal",
      SourceSpan::new(start, self.position),
    ));
  }

  fn lex_number(&mut self) {
    let start = self.position;
    while matches!(self.peek_char(), Some('0'..='9')) {
      self.advance_char();
    }

    let is_float =
      self.peek_char() == Some('.') && matches!(self.peek_next_char(), Some('0'..='9'));
    if is_float {
      self.advance_char();
      while matches!(self.peek_char(), Some('0'..='9')) {
        self.advance_char();
      }
    }

    let raw = &self.input[start..self.position];
    if is_float {
      match raw.parse::<f64>() {
        Ok(value) => self
          .tokens
          .push(Token::new(TokenKind::Float(value), start, self.position)),
        Err(_) => self.diagnostics.push(Diagnostic::new(
          "invalid float literal",
          SourceSpan::new(start, self.position),
        )),
      }
    } else {
      match raw.parse::<i64>() {
        Ok(value) => self
          .tokens
          .push(Token::new(TokenKind::Int(value), start, self.position)),
        Err(_) => self.diagnostics.push(Diagnostic::new(
          "invalid integer literal",
          SourceSpan::new(start, self.position),
        )),
      }
    }
  }

  fn lex_identifier(&mut self) {
    let start = self.position;
    self.advance_char();
    while let Some(ch) = self.peek_char() {
      if ch.is_ascii_alphanumeric() || ch == '_' {
        self.advance_char();
      } else {
        break;
      }
    }

    let raw = &self.input[start..self.position];
    let kind = match raw {
      "true" => TokenKind::True,
      "false" => TokenKind::False,
      "null" => TokenKind::Null,
      _ => TokenKind::Identifier(raw.to_string()),
    };
    self.tokens.push(Token::new(kind, start, self.position));
  }

  fn skip_line_comment(&mut self) {
    while let Some(ch) = self.peek_char() {
      self.advance_char();
      if ch == '\n' {
        break;
      }
    }
  }

  fn push_simple(&mut self, kind: TokenKind) {
    let start = self.position;
    self.advance_char();
    self.tokens.push(Token::new(kind, start, self.position));
  }

  fn push_two(&mut self, kind: TokenKind) {
    let start = self.position;
    self.advance_char();
    self.advance_char();
    self.tokens.push(Token::new(kind, start, self.position));
  }

  fn invalid_char(&mut self, ch: char) {
    let start = self.position;
    self.advance_char();
    self.diagnostics.push(Diagnostic::new(
      format!("invalid character {ch:?}"),
      SourceSpan::new(start, self.position),
    ));
  }

  fn peek_char(&self) -> Option<char> {
    self.input[self.position..].chars().next()
  }

  fn peek_next_char(&self) -> Option<char> {
    let mut chars = self.input[self.position..].chars();
    chars.next()?;
    chars.next()
  }

  fn advance_char(&mut self) -> Option<char> {
    let ch = self.peek_char()?;
    self.position += ch.len_utf8();
    Some(ch)
  }
}
