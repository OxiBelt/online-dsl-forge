use super::ast::{AstExpression, BinaryOp, ExprKind, UnaryOp};
use super::diagnostics::{Diagnostic, DiagnosticReport};
use super::lexer::{Token, TokenKind, tokenize_with_limits};
use super::limits::ParseLimits;
use super::span::SourceSpan;
use super::validation::is_reserved_identifier;

const MAX_PARSE_RECURSION_DEPTH: usize = 256;
// `serde_json` rejects the 128th nested container by default. Keep every AST
// produced by the parser below that boundary so its public JSON form can be
// deserialized without caller-specific configuration.
const MAX_AST_SERIALIZED_DEPTH: usize = 127;
const PARSE_RECURSION_DEPTH_EXCEEDED: &str = "parse recursion depth limit exceeded";
const AST_DEPTH_EXCEEDED: &str = "AST depth limit exceeded";
const AST_NODE_LIMIT_EXCEEDED: &str = "AST node limit exceeded";
const COLLECTION_LIMIT_EXCEEDED: &str = "collection item limit exceeded";

pub fn parse_expression(input: &str) -> Result<AstExpression, DiagnosticReport> {
  parse_expression_with_limits(input, ParseLimits::default())
}

pub fn parse_expression_with_limits(
  input: &str,
  limits: ParseLimits,
) -> Result<AstExpression, DiagnosticReport> {
  let tokens = tokenize_with_limits(input, limits).map_err(DiagnosticReport::new)?;
  Parser::new(tokens, limits).parse().map_err(|report| {
    if report.diagnostics.len() > limits.max_diagnostics {
      let span = report
        .diagnostics
        .first()
        .map(|diagnostic| diagnostic.span)
        .unwrap_or_default();
      DiagnosticReport::single("diagnostic limit exceeded", span)
    } else {
      report
    }
  })
}

struct ParsedExpression {
  ast: AstExpression,
  serialized_depth: usize,
}

impl ParsedExpression {
  fn span(&self) -> SourceSpan {
    self.ast.span
  }
}

#[derive(Default)]
struct ParsedSequence {
  expressions: Vec<AstExpression>,
  max_serialized_depth: usize,
}

impl ParsedSequence {
  fn push(&mut self, expression: ParsedExpression) {
    self.max_serialized_depth = self.max_serialized_depth.max(expression.serialized_depth);
    self.expressions.push(expression.ast);
  }
}

struct Parser {
  tokens: Vec<Token>,
  position: usize,
  recursion_depth: usize,
  ast_nodes: usize,
  limits: ParseLimits,
}

impl Parser {
  fn new(tokens: Vec<Token>, limits: ParseLimits) -> Self {
    Self {
      tokens,
      position: 0,
      recursion_depth: 0,
      ast_nodes: 0,
      limits,
    }
  }

  fn parse(mut self) -> Result<AstExpression, DiagnosticReport> {
    let expression = self.parse_or()?;
    if !matches!(self.peek().kind, TokenKind::Eof) {
      return Err(self.error_here("unexpected token after expression"));
    }
    Ok(expression.ast)
  }

  fn parse_or(&mut self) -> Result<ParsedExpression, DiagnosticReport> {
    let mut expression = self.parse_and()?;
    while self
      .consume_kind(|kind| matches!(kind, TokenKind::OrOr))
      .is_some()
    {
      let right = self.parse_and()?;
      expression = self.binary(expression, BinaryOp::Or, right)?;
    }
    Ok(expression)
  }

  fn parse_and(&mut self) -> Result<ParsedExpression, DiagnosticReport> {
    let mut expression = self.parse_equality()?;
    while self
      .consume_kind(|kind| matches!(kind, TokenKind::AndAnd))
      .is_some()
    {
      let right = self.parse_equality()?;
      expression = self.binary(expression, BinaryOp::And, right)?;
    }
    Ok(expression)
  }

  fn parse_equality(&mut self) -> Result<ParsedExpression, DiagnosticReport> {
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
      expression = self.binary(expression, op, right)?;
    }
    Ok(expression)
  }

  fn parse_comparison(&mut self) -> Result<ParsedExpression, DiagnosticReport> {
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
      expression = self.binary(expression, op, right)?;
    }
    Ok(expression)
  }

  fn parse_additive(&mut self) -> Result<ParsedExpression, DiagnosticReport> {
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
      expression = self.binary(expression, op, right)?;
    }
    Ok(expression)
  }

  fn parse_multiplicative(&mut self) -> Result<ParsedExpression, DiagnosticReport> {
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
      expression = self.binary(expression, op, right)?;
    }
    Ok(expression)
  }

  fn parse_unary(&mut self) -> Result<ParsedExpression, DiagnosticReport> {
    if let Some(token) = self.consume_kind(|kind| matches!(kind, TokenKind::Bang)) {
      let expr = self.parse_nested(token.span, |parser| parser.parse_unary())?;
      let span = token.span.join(expr.span());
      let serialized_depth = self.add_depth(2, expr.serialized_depth, span)?;
      return self.make_expression(
        || ExprKind::Unary {
          op: UnaryOp::Not,
          expr: Box::new(expr.ast),
        },
        span,
        serialized_depth,
      );
    }

    if let Some(token) = self.consume_kind(|kind| matches!(kind, TokenKind::Minus)) {
      let expr = self.parse_nested(token.span, |parser| parser.parse_unary())?;
      let span = token.span.join(expr.span());
      let serialized_depth = self.add_depth(2, expr.serialized_depth, span)?;
      return self.make_expression(
        || ExprKind::Unary {
          op: UnaryOp::Neg,
          expr: Box::new(expr.ast),
        },
        span,
        serialized_depth,
      );
    }

    self.parse_postfix()
  }

  fn parse_postfix(&mut self) -> Result<ParsedExpression, DiagnosticReport> {
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
        let span = expression.span().join(end_span);
        let receiver_depth = self.add_depth(2, expression.serialized_depth, span)?;
        let args_depth = self.add_depth(3, args.max_serialized_depth, span)?;
        let serialized_depth = receiver_depth.max(args_depth);
        expression = self.make_expression(
          || ExprKind::MethodCall {
            receiver: Box::new(expression.ast),
            name,
            args: args.expressions,
          },
          span,
          serialized_depth,
        )?;
      } else {
        let span = expression.span().join(self.previous_span());
        let serialized_depth = self.add_depth(2, expression.serialized_depth, span)?;
        expression = self.make_expression(
          || ExprKind::Member {
            receiver: Box::new(expression.ast),
            name,
          },
          span,
          serialized_depth,
        )?;
      }
    }
    Ok(expression)
  }

  fn parse_primary(&mut self) -> Result<ParsedExpression, DiagnosticReport> {
    let token = self.advance().clone();
    match token.kind {
      TokenKind::True => self.make_leaf(|| ExprKind::Bool { value: true }, token.span),
      TokenKind::False => self.make_leaf(|| ExprKind::Bool { value: false }, token.span),
      TokenKind::Null => self.make_leaf(|| ExprKind::Null, token.span),
      TokenKind::Int(value) => self.make_leaf(|| ExprKind::Int { value }, token.span),
      TokenKind::Float(value) => self.make_leaf(|| ExprKind::Float { value }, token.span),
      TokenKind::String(value) => self.make_leaf(|| ExprKind::String { value }, token.span),
      TokenKind::Identifier(name) => {
        validate_identifier(&name, token.span)?;
        if self
          .consume_kind(|kind| matches!(kind, TokenKind::LParen))
          .is_some()
        {
          let (args, end_span) = self.parse_call_args()?;
          let serialized_depth = self.add_depth(3, args.max_serialized_depth, token.span)?;
          self.make_expression(
            || ExprKind::FunctionCall {
              name,
              args: args.expressions,
            },
            token.span.join(end_span),
            serialized_depth,
          )
        } else {
          self.make_leaf(|| ExprKind::Identifier { name }, token.span)
        }
      }
      TokenKind::LParen => {
        let expression = self.parse_nested(token.span, |parser| parser.parse_or())?;
        self.expect_kind("expected closing parenthesis", |kind| {
          matches!(kind, TokenKind::RParen)
        })?;
        Ok(expression)
      }
      TokenKind::LBracket => self.parse_array(token.span),
      _ => Err(DiagnosticReport::single("expected expression", token.span)),
    }
  }

  fn parse_array(&mut self, start_span: SourceSpan) -> Result<ParsedExpression, DiagnosticReport> {
    let mut items = ParsedSequence::default();
    if let Some(end) = self.consume_kind(|kind| matches!(kind, TokenKind::RBracket)) {
      return self.make_expression(
        || ExprKind::Array {
          items: items.expressions,
        },
        start_span.join(end.span),
        3,
      );
    }

    loop {
      self.check_collection_width(items.expressions.len(), self.peek().span)?;
      items.push(self.parse_nested(start_span, |parser| parser.parse_or())?);
      if let Some(end) = self.consume_kind(|kind| matches!(kind, TokenKind::RBracket)) {
        let serialized_depth = self.add_depth(3, items.max_serialized_depth, start_span)?;
        return self.make_expression(
          || ExprKind::Array {
            items: items.expressions,
          },
          start_span.join(end.span),
          serialized_depth,
        );
      }
      self.expect_kind("expected comma in array literal", |kind| {
        matches!(kind, TokenKind::Comma)
      })?;
    }
  }

  fn parse_call_args(&mut self) -> Result<(ParsedSequence, SourceSpan), DiagnosticReport> {
    let mut args = ParsedSequence::default();
    if let Some(end) = self.consume_kind(|kind| matches!(kind, TokenKind::RParen)) {
      return Ok((args, end.span));
    }

    loop {
      let span = self.peek().span;
      self.check_collection_width(args.expressions.len(), span)?;
      args.push(self.parse_nested(span, |parser| parser.parse_or())?);
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

  fn parse_nested<T>(
    &mut self,
    span: SourceSpan,
    parse: impl FnOnce(&mut Self) -> Result<T, DiagnosticReport>,
  ) -> Result<T, DiagnosticReport> {
    if self.recursion_depth >= MAX_PARSE_RECURSION_DEPTH {
      return Err(DiagnosticReport::single(
        PARSE_RECURSION_DEPTH_EXCEEDED,
        span,
      ));
    }
    self.recursion_depth += 1;
    let result = parse(self);
    self.recursion_depth -= 1;
    result
  }

  fn make_leaf(
    &mut self,
    kind: impl FnOnce() -> ExprKind,
    span: SourceSpan,
  ) -> Result<ParsedExpression, DiagnosticReport> {
    self.make_expression(kind, span, 2)
  }

  fn make_expression(
    &mut self,
    kind: impl FnOnce() -> ExprKind,
    span: SourceSpan,
    serialized_depth: usize,
  ) -> Result<ParsedExpression, DiagnosticReport> {
    if serialized_depth > MAX_AST_SERIALIZED_DEPTH {
      return Err(DiagnosticReport::single(AST_DEPTH_EXCEEDED, span));
    }
    let nodes = self
      .ast_nodes
      .checked_add(1)
      .ok_or_else(|| DiagnosticReport::single(AST_NODE_LIMIT_EXCEEDED, span))?;
    if nodes > self.limits.max_ast_nodes {
      return Err(DiagnosticReport::single(AST_NODE_LIMIT_EXCEEDED, span));
    }
    self.ast_nodes = nodes;
    Ok(ParsedExpression {
      ast: AstExpression::new(kind(), span),
      serialized_depth,
    })
  }

  fn add_depth(
    &self,
    outer: usize,
    inner: usize,
    span: SourceSpan,
  ) -> Result<usize, DiagnosticReport> {
    outer
      .checked_add(inner)
      .ok_or_else(|| DiagnosticReport::single(AST_DEPTH_EXCEEDED, span))
  }

  fn check_collection_width(
    &self,
    current_len: usize,
    span: SourceSpan,
  ) -> Result<(), DiagnosticReport> {
    if current_len >= self.limits.max_collection_items {
      Err(DiagnosticReport::single(COLLECTION_LIMIT_EXCEEDED, span))
    } else {
      Ok(())
    }
  }

  fn binary(
    &mut self,
    left: ParsedExpression,
    op: BinaryOp,
    right: ParsedExpression,
  ) -> Result<ParsedExpression, DiagnosticReport> {
    let span = left.span().join(right.span());
    let inner = left.serialized_depth.max(right.serialized_depth);
    let serialized_depth = self.add_depth(2, inner, span)?;
    self.make_expression(
      || ExprKind::Binary {
        left: Box::new(left.ast),
        op,
        right: Box::new(right.ast),
      },
      span,
      serialized_depth,
    )
  }
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
