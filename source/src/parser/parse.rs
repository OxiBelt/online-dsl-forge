use super::ast::{AstExpression, BinaryOp, ExprKind, UnaryOp};
use super::diagnostics::{Diagnostic, DiagnosticReport};
use super::lexer::{Token, TokenKind, tokenize};
use super::span::SourceSpan;

pub fn parse_expression(input: &str) -> Result<AstExpression, DiagnosticReport> {
  let tokens = tokenize(input).map_err(DiagnosticReport::new)?;
  Parser::new(tokens).parse()
}

struct Parser {
  tokens: Vec<Token>,
  position: usize,
}

impl Parser {
  fn new(tokens: Vec<Token>) -> Self {
    Self {
      tokens,
      position: 0,
    }
  }

  fn parse(mut self) -> Result<AstExpression, DiagnosticReport> {
    let expression = self.parse_or()?;
    if !matches!(self.peek().kind, TokenKind::Eof) {
      return Err(self.error_here("unexpected token after expression"));
    }
    Ok(expression)
  }

  fn parse_or(&mut self) -> Result<AstExpression, DiagnosticReport> {
    let mut expression = self.parse_and()?;
    while self
      .consume_kind(|kind| matches!(kind, TokenKind::OrOr))
      .is_some()
    {
      let right = self.parse_and()?;
      expression = binary(expression, BinaryOp::Or, right);
    }
    Ok(expression)
  }

  fn parse_and(&mut self) -> Result<AstExpression, DiagnosticReport> {
    let mut expression = self.parse_equality()?;
    while self
      .consume_kind(|kind| matches!(kind, TokenKind::AndAnd))
      .is_some()
    {
      let right = self.parse_equality()?;
      expression = binary(expression, BinaryOp::And, right);
    }
    Ok(expression)
  }

  fn parse_equality(&mut self) -> Result<AstExpression, DiagnosticReport> {
    let mut expression = self.parse_comparison()?;
    loop {
      let op = if self
        .consume_kind(|kind| matches!(kind, TokenKind::EqEq))
        .is_some()
      {
        Some(BinaryOp::Eq)
      } else if self
        .consume_kind(|kind| matches!(kind, TokenKind::Ne))
        .is_some()
      {
        Some(BinaryOp::Ne)
      } else {
        None
      };
      let Some(op) = op else {
        break;
      };
      let right = self.parse_comparison()?;
      expression = binary(expression, op, right);
    }
    Ok(expression)
  }

  fn parse_comparison(&mut self) -> Result<AstExpression, DiagnosticReport> {
    let mut expression = self.parse_additive()?;
    loop {
      let op = if self
        .consume_kind(|kind| matches!(kind, TokenKind::Lt))
        .is_some()
      {
        Some(BinaryOp::Lt)
      } else if self
        .consume_kind(|kind| matches!(kind, TokenKind::Le))
        .is_some()
      {
        Some(BinaryOp::Le)
      } else if self
        .consume_kind(|kind| matches!(kind, TokenKind::Gt))
        .is_some()
      {
        Some(BinaryOp::Gt)
      } else if self
        .consume_kind(|kind| matches!(kind, TokenKind::Ge))
        .is_some()
      {
        Some(BinaryOp::Ge)
      } else {
        None
      };
      let Some(op) = op else {
        break;
      };
      let right = self.parse_additive()?;
      expression = binary(expression, op, right);
    }
    Ok(expression)
  }

  fn parse_additive(&mut self) -> Result<AstExpression, DiagnosticReport> {
    let mut expression = self.parse_multiplicative()?;
    loop {
      let op = if self
        .consume_kind(|kind| matches!(kind, TokenKind::Plus))
        .is_some()
      {
        Some(BinaryOp::Add)
      } else if self
        .consume_kind(|kind| matches!(kind, TokenKind::Minus))
        .is_some()
      {
        Some(BinaryOp::Sub)
      } else {
        None
      };
      let Some(op) = op else {
        break;
      };
      let right = self.parse_multiplicative()?;
      expression = binary(expression, op, right);
    }
    Ok(expression)
  }

  fn parse_multiplicative(&mut self) -> Result<AstExpression, DiagnosticReport> {
    let mut expression = self.parse_unary()?;
    loop {
      let op = if self
        .consume_kind(|kind| matches!(kind, TokenKind::Star))
        .is_some()
      {
        Some(BinaryOp::Mul)
      } else if self
        .consume_kind(|kind| matches!(kind, TokenKind::Slash))
        .is_some()
      {
        Some(BinaryOp::Div)
      } else if self
        .consume_kind(|kind| matches!(kind, TokenKind::Percent))
        .is_some()
      {
        Some(BinaryOp::Rem)
      } else {
        None
      };
      let Some(op) = op else {
        break;
      };
      let right = self.parse_unary()?;
      expression = binary(expression, op, right);
    }
    Ok(expression)
  }

  fn parse_unary(&mut self) -> Result<AstExpression, DiagnosticReport> {
    if let Some(token) = self.consume_kind(|kind| matches!(kind, TokenKind::Bang)) {
      let expr = self.parse_unary()?;
      let span = token.span.join(expr.span);
      return Ok(AstExpression::new(
        ExprKind::Unary {
          op: UnaryOp::Not,
          expr: Box::new(expr),
        },
        span,
      ));
    }

    if let Some(token) = self.consume_kind(|kind| matches!(kind, TokenKind::Minus)) {
      let expr = self.parse_unary()?;
      let span = token.span.join(expr.span);
      return Ok(AstExpression::new(
        ExprKind::Unary {
          op: UnaryOp::Neg,
          expr: Box::new(expr),
        },
        span,
      ));
    }

    self.parse_postfix()
  }

  fn parse_postfix(&mut self) -> Result<AstExpression, DiagnosticReport> {
    let mut expression = self.parse_primary()?;
    while self
      .consume_kind(|kind| matches!(kind, TokenKind::Dot))
      .is_some()
    {
      let name = self.expect_identifier()?;
      if self
        .consume_kind(|kind| matches!(kind, TokenKind::LParen))
        .is_some()
      {
        let (args, end_span) = self.parse_call_args()?;
        let span = expression.span.join(end_span);
        expression = AstExpression::new(
          ExprKind::MethodCall {
            receiver: Box::new(expression),
            name,
            args,
          },
          span,
        );
      } else {
        let span = expression.span.join(self.previous_span());
        expression = AstExpression::new(
          ExprKind::Member {
            receiver: Box::new(expression),
            name,
          },
          span,
        );
      }
    }
    Ok(expression)
  }

  fn parse_primary(&mut self) -> Result<AstExpression, DiagnosticReport> {
    let token = self.advance().clone();
    match token.kind {
      TokenKind::True => Ok(AstExpression::new(
        ExprKind::Bool { value: true },
        token.span,
      )),
      TokenKind::False => Ok(AstExpression::new(
        ExprKind::Bool { value: false },
        token.span,
      )),
      TokenKind::Null => Ok(AstExpression::new(ExprKind::Null, token.span)),
      TokenKind::Int(value) => Ok(AstExpression::new(ExprKind::Int { value }, token.span)),
      TokenKind::Float(value) => Ok(AstExpression::new(ExprKind::Float { value }, token.span)),
      TokenKind::String(value) => Ok(AstExpression::new(ExprKind::String { value }, token.span)),
      TokenKind::Identifier(name) => {
        validate_identifier(&name, token.span)?;
        if self
          .consume_kind(|kind| matches!(kind, TokenKind::LParen))
          .is_some()
        {
          let (args, end_span) = self.parse_call_args()?;
          Ok(AstExpression::new(
            ExprKind::FunctionCall { name, args },
            token.span.join(end_span),
          ))
        } else {
          Ok(AstExpression::new(
            ExprKind::Identifier { name },
            token.span,
          ))
        }
      }
      TokenKind::LParen => {
        let expression = self.parse_or()?;
        self.expect_kind("expected closing parenthesis", |kind| {
          matches!(kind, TokenKind::RParen)
        })?;
        Ok(expression)
      }
      TokenKind::LBracket => self.parse_array(token.span),
      _ => Err(DiagnosticReport::single("expected expression", token.span)),
    }
  }

  fn parse_array(&mut self, start_span: SourceSpan) -> Result<AstExpression, DiagnosticReport> {
    let mut items = Vec::new();
    if let Some(end) = self.consume_kind(|kind| matches!(kind, TokenKind::RBracket)) {
      return Ok(AstExpression::new(
        ExprKind::Array { items },
        start_span.join(end.span),
      ));
    }

    loop {
      items.push(self.parse_or()?);
      if let Some(end) = self.consume_kind(|kind| matches!(kind, TokenKind::RBracket)) {
        return Ok(AstExpression::new(
          ExprKind::Array { items },
          start_span.join(end.span),
        ));
      }
      self.expect_kind("expected comma in array literal", |kind| {
        matches!(kind, TokenKind::Comma)
      })?;
    }
  }

  fn parse_call_args(&mut self) -> Result<(Vec<AstExpression>, SourceSpan), DiagnosticReport> {
    let mut args = Vec::new();
    if let Some(end) = self.consume_kind(|kind| matches!(kind, TokenKind::RParen)) {
      return Ok((args, end.span));
    }

    loop {
      args.push(self.parse_or()?);
      if let Some(end) = self.consume_kind(|kind| matches!(kind, TokenKind::RParen)) {
        return Ok((args, end.span));
      }
      self.expect_kind("expected comma in argument list", |kind| {
        matches!(kind, TokenKind::Comma)
      })?;
    }
  }

  fn expect_identifier(&mut self) -> Result<String, DiagnosticReport> {
    let token = self.advance().clone();
    match token.kind {
      TokenKind::Identifier(name) => {
        validate_identifier(&name, token.span)?;
        Ok(name)
      }
      _ => Err(DiagnosticReport::single("expected identifier", token.span)),
    }
  }

  fn expect_kind(
    &mut self,
    message: &'static str,
    predicate: impl FnOnce(&TokenKind) -> bool,
  ) -> Result<Token, DiagnosticReport> {
    let token = self.advance().clone();
    if predicate(&token.kind) {
      Ok(token)
    } else {
      Err(DiagnosticReport::single(message, token.span))
    }
  }

  fn consume_kind(&mut self, predicate: impl FnOnce(&TokenKind) -> bool) -> Option<Token> {
    if predicate(&self.peek().kind) {
      let token = self.peek().clone();
      self.position += 1;
      Some(token)
    } else {
      None
    }
  }

  fn advance(&mut self) -> &Token {
    let index = self.position.min(self.tokens.len().saturating_sub(1));
    if !matches!(self.tokens[index].kind, TokenKind::Eof) {
      self.position += 1;
    }
    &self.tokens[index]
  }

  fn peek(&self) -> &Token {
    self.tokens.get(self.position).unwrap_or_else(|| {
      self
        .tokens
        .last()
        .expect("parser requires lexer to append an EOF token")
    })
  }

  fn previous_span(&self) -> SourceSpan {
    self
      .tokens
      .get(self.position.saturating_sub(1))
      .map(|token| token.span)
      .unwrap_or_default()
  }

  fn error_here(&self, message: &'static str) -> DiagnosticReport {
    DiagnosticReport::single(message, self.peek().span)
  }
}

fn binary(left: AstExpression, op: BinaryOp, right: AstExpression) -> AstExpression {
  let span = left.span.join(right.span);
  AstExpression::new(
    ExprKind::Binary {
      left: Box::new(left),
      op,
      right: Box::new(right),
    },
    span,
  )
}

fn validate_identifier(identifier: &str, span: SourceSpan) -> Result<(), DiagnosticReport> {
  if is_reserved_identifier(identifier) {
    Err(DiagnosticReport::new(vec![Diagnostic::new(
      format!("reserved identifier {identifier}"),
      span,
    )]))
  } else {
    Ok(())
  }
}

fn is_reserved_identifier(identifier: &str) -> bool {
  matches!(
    identifier,
    "if"
      | "else"
      | "for"
      | "while"
      | "do"
      | "switch"
      | "let"
      | "const"
      | "function"
      | "import"
      | "export"
      | "new"
      | "try"
      | "catch"
      | "throw"
      | "await"
      | "return"
      | "true"
      | "false"
      | "null"
  )
}

#[cfg(test)]
mod tests {
  use crate::format_expression;

  use super::parse_expression;

  #[test]
  fn parses_precedence() {
    let ast = parse_expression("1 + 2 * 3 == 7 || false").expect("expression should parse");
    assert_eq!(format_expression(&ast), "1 + 2 * 3 == 7 || false");
  }

  #[test]
  fn parses_calls_members_and_arrays() {
    let ast = parse_expression("user.name.starts_with('pi') && len([1, 2]) == 2")
      .expect("expression should parse");
    assert_eq!(
      format_expression(&ast),
      "user.name.starts_with(\"pi\") && len([1, 2]) == 2"
    );
  }
}
