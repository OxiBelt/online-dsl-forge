use super::diagnostics::Diagnostic;
use super::limits::ParseLimits;
use super::span::SourceSpan;

const SOURCE_LIMIT_EXCEEDED: &str = "source byte limit exceeded";
const SCALAR_LIMIT_EXCEEDED: &str = "decoded scalar byte limit exceeded";
const TOKEN_LIMIT_EXCEEDED: &str = "token limit exceeded";
const DIAGNOSTIC_LIMIT_EXCEEDED: &str = "diagnostic limit exceeded";

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
  tokenize_with_limits(input, ParseLimits::default())
}

pub fn tokenize_with_limits(
  input: &str,
  limits: ParseLimits,
) -> Result<Vec<Token>, Vec<Diagnostic>> {
  if input.len() > limits.max_source_bytes {
    return Err(vec![Diagnostic::new(
      SOURCE_LIMIT_EXCEEDED,
      SourceSpan::new(0, input.len()),
    )]);
  }
  let mut lexer = Lexer {
    input,
    position: 0,
    decoded_scalar_bytes: 0,
    stopped: false,
    limits,
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
  decoded_scalar_bytes: usize,
  stopped: bool,
  limits: ParseLimits,
  diagnostics: Vec<Diagnostic>,
  tokens: Vec<Token>,
}

impl Lexer<'_> {
  fn run(&mut self) {
    while !self.stopped
      && let Some(ch) = self.peek_char()
    {
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
    if !self.stopped {
      self.push_token(TokenKind::Eof, self.position, self.position);
    }
  }

  fn lex_string(&mut self, quote: char) {
    let start = self.position;
    if !self.has_token_capacity(SourceSpan::new(start, start + quote.len_utf8())) {
      return;
    }
    self.advance_char();
    let mut value = String::new();

    while let Some(ch) = self.peek_char() {
      if ch == quote {
        self.advance_char();
        self.push_token(TokenKind::String(value), start, self.position);
        return;
      }

      if ch == '\\' {
        self.advance_char();
        let Some(escaped) = self.peek_char() else {
          break;
        };
        self.advance_char();
        match escaped {
          '\\' => self.push_scalar(&mut value, '\\', start),
          '"' => self.push_scalar(&mut value, '"', start),
          '\'' => self.push_scalar(&mut value, '\'', start),
          'n' => self.push_scalar(&mut value, '\n', start),
          'r' => self.push_scalar(&mut value, '\r', start),
          't' => self.push_scalar(&mut value, '\t', start),
          other => self.push_scalar(&mut value, other, start),
        }
      } else {
        self.push_scalar(&mut value, ch, start);
        self.advance_char();
      }
      if self.stopped {
        return;
      }
    }

    self.push_diagnostic(Diagnostic::new(
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
        Ok(value) if value.is_finite() => {
          self.push_token(TokenKind::Float(value), start, self.position)
        }
        Ok(_) | Err(_) => self.push_diagnostic(Diagnostic::new(
          "invalid float literal",
          SourceSpan::new(start, self.position),
        )),
      }
    } else {
      match raw.parse::<i64>() {
        Ok(value) => self.push_token(TokenKind::Int(value), start, self.position),
        Err(_) => self.push_diagnostic(Diagnostic::new(
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
    if !self.charge_scalar_bytes(raw.len(), SourceSpan::new(start, self.position)) {
      return;
    }
    if !self.has_token_capacity(SourceSpan::new(start, self.position)) {
      return;
    }
    let kind = match raw {
      "true" => TokenKind::True,
      "false" => TokenKind::False,
      "null" => TokenKind::Null,
      _ => TokenKind::Identifier(raw.to_string()),
    };
    self.push_token(kind, start, self.position);
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
    self.push_token(kind, start, self.position);
  }

  fn push_two(&mut self, kind: TokenKind) {
    let start = self.position;
    self.advance_char();
    self.advance_char();
    self.push_token(kind, start, self.position);
  }

  fn invalid_char(&mut self, ch: char) {
    let start = self.position;
    self.advance_char();
    self.push_diagnostic(Diagnostic::new(
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

  fn push_token(&mut self, kind: TokenKind, start: usize, end: usize) {
    if !self.has_token_capacity(SourceSpan::new(start, end)) {
      return;
    }
    self.tokens.push(Token::new(kind, start, end));
  }

  fn has_token_capacity(&mut self, span: SourceSpan) -> bool {
    if self.tokens.len() >= self.limits.max_tokens {
      self.fail_limit(TOKEN_LIMIT_EXCEEDED, span);
      false
    } else {
      true
    }
  }

  fn push_diagnostic(&mut self, diagnostic: Diagnostic) {
    if self.diagnostics.len() >= self.limits.max_diagnostics {
      self.fail_limit(DIAGNOSTIC_LIMIT_EXCEEDED, diagnostic.span);
      return;
    }
    self.diagnostics.push(diagnostic);
  }

  fn push_scalar(&mut self, value: &mut String, ch: char, start: usize) {
    let span = SourceSpan::new(start, self.position);
    if self.charge_scalar_bytes(ch.len_utf8(), span) {
      value.push(ch);
    }
  }

  fn charge_scalar_bytes(&mut self, bytes: usize, span: SourceSpan) -> bool {
    let Some(total) = self.decoded_scalar_bytes.checked_add(bytes) else {
      self.fail_limit(SCALAR_LIMIT_EXCEEDED, span);
      return false;
    };
    if total > self.limits.max_decoded_scalar_bytes {
      self.fail_limit(SCALAR_LIMIT_EXCEEDED, span);
      return false;
    }
    self.decoded_scalar_bytes = total;
    true
  }

  fn fail_limit(&mut self, message: &'static str, span: SourceSpan) {
    self.diagnostics.clear();
    self.diagnostics.push(Diagnostic::new(message, span));
    self.stopped = true;
  }
}
