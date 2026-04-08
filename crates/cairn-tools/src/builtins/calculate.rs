//! calculate — safe arithmetic evaluator (no eval()).
//!
//! Recursive descent parser: + - * / % ** with parens and unary minus.
use super::{ToolError, ToolHandler, ToolResult, ToolTier};
use async_trait::async_trait;
use cairn_domain::ProjectKey;
use serde_json::Value;

// ── Tokenizer ─────────────────────────────────────────────────────────────────
#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(f64),
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    StarStar,
    LParen,
    RParen,
}

fn tokenize(s: &str) -> Result<Vec<Tok>, String> {
    let mut ts = Vec::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            ' ' | '\t' => {
                i += 1;
            }
            '0'..='9' | '.' => {
                let start = i;
                while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                    i += 1;
                }
                let n: f64 = s[start..i]
                    .parse()
                    .map_err(|e| format!("bad number: {e}"))?;
                ts.push(Tok::Num(n));
            }
            '+' => {
                ts.push(Tok::Plus);
                i += 1;
            }
            '-' => {
                ts.push(Tok::Minus);
                i += 1;
            }
            '*' => {
                if i + 1 < chars.len() && chars[i + 1] == '*' {
                    ts.push(Tok::StarStar);
                    i += 2;
                } else {
                    ts.push(Tok::Star);
                    i += 1;
                }
            }
            '/' => {
                ts.push(Tok::Slash);
                i += 1;
            }
            '%' => {
                ts.push(Tok::Percent);
                i += 1;
            }
            '(' => {
                ts.push(Tok::LParen);
                i += 1;
            }
            ')' => {
                ts.push(Tok::RParen);
                i += 1;
            }
            c => return Err(format!("unexpected character: '{c}'")),
        }
    }
    Ok(ts)
}

// ── Parser ────────────────────────────────────────────────────────────────────
struct Parser {
    toks: Vec<Tok>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }
    fn consume(&mut self) {
        self.pos += 1;
    }

    fn expr(&mut self) -> Result<f64, String> {
        let mut v = self.term()?;
        while matches!(self.peek(), Some(Tok::Plus) | Some(Tok::Minus)) {
            let op = self.peek().unwrap().clone();
            self.consume();
            let r = self.term()?;
            v = if op == Tok::Plus { v + r } else { v - r };
        }
        Ok(v)
    }
    fn term(&mut self) -> Result<f64, String> {
        let mut v = self.factor()?;
        while matches!(
            self.peek(),
            Some(Tok::Star) | Some(Tok::Slash) | Some(Tok::Percent)
        ) {
            let op = self.peek().unwrap().clone();
            self.consume();
            let r = self.factor()?;
            v = match op {
                Tok::Star => v * r,
                Tok::Slash => {
                    if r == 0.0 {
                        return Err("division by zero".into());
                    }
                    v / r
                }
                Tok::Percent => {
                    if r == 0.0 {
                        return Err("modulo by zero".into());
                    }
                    v % r
                }
                _ => unreachable!(),
            };
        }
        Ok(v)
    }
    fn factor(&mut self) -> Result<f64, String> {
        let base = self.unary()?;
        if matches!(self.peek(), Some(Tok::StarStar)) {
            self.consume();
            let exp = self.factor()?; // right-associative
            Ok(base.powf(exp))
        } else {
            Ok(base)
        }
    }
    fn unary(&mut self) -> Result<f64, String> {
        if matches!(self.peek(), Some(Tok::Minus)) {
            self.consume();
            Ok(-self.unary()?)
        } else {
            self.primary()
        }
    }
    fn primary(&mut self) -> Result<f64, String> {
        match self.peek().cloned() {
            Some(Tok::Num(n)) => {
                self.consume();
                Ok(n)
            }
            Some(Tok::LParen) => {
                self.consume();
                let v = self.expr()?;
                if self.peek() != Some(&Tok::RParen) {
                    return Err("expected ')'".into());
                }
                self.consume();
                Ok(v)
            }
            other => Err(format!("unexpected token: {other:?}")),
        }
    }
}

fn evaluate(expr: &str) -> Result<f64, String> {
    let toks = tokenize(expr.trim())?;
    let mut p = Parser { toks, pos: 0 };
    let v = p.expr()?;
    if p.pos < p.toks.len() {
        return Err(format!("unexpected token at position {}", p.pos));
    }
    Ok(v)
}

// ── Tool ──────────────────────────────────────────────────────────────────────
pub struct CalculateTool;
impl Default for CalculateTool {
    fn default() -> Self {
        Self
    }
}

#[async_trait]
impl ToolHandler for CalculateTool {
    fn name(&self) -> &str {
        "calculate"
    }
    fn tier(&self) -> ToolTier {
        ToolTier::Registered
    }
    fn description(&self) -> &str {
        "Evaluate a mathematical expression safely. \
         Supports +, -, *, /, %, ** (power), parentheses, and unary minus."
    }
    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type":"object","required":["expression"],
            "properties":{
                "expression":{"type":"string","description":"Math expression e.g. '2 ** 10 + (3 * 4)'"},
                "precision":{"type":"integer","default":6,"minimum":0,"maximum":15,
                             "description":"Decimal places for the formatted result"}
            }
        })
    }
    async fn execute(&self, _: &ProjectKey, args: Value) -> Result<ToolResult, ToolError> {
        let expr = args["expression"]
            .as_str()
            .ok_or_else(|| ToolError::InvalidArgs {
                field: "expression".into(),
                message: "required string".into(),
            })?
            .trim();
        if expr.is_empty() {
            return Err(ToolError::InvalidArgs {
                field: "expression".into(),
                message: "must not be empty".into(),
            });
        }
        let precision = args["precision"].as_u64().unwrap_or(6).min(15) as usize;
        let result = evaluate(expr).map_err(|e| {
            if e.contains("division") || e.contains("modulo") {
                ToolError::Permanent(e)
            } else {
                ToolError::InvalidArgs {
                    field: "expression".into(),
                    message: e,
                }
            }
        })?;
        let formatted = format!("{:.prec$}", result, prec = precision);
        Ok(ToolResult::ok(serde_json::json!({
            "result": result, "expression": expr, "formatted": formatted
        })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn p() -> ProjectKey {
        ProjectKey::new("t", "w", "p")
    }
    fn calc() -> CalculateTool {
        CalculateTool
    }

    #[tokio::test]
    async fn addition() {
        let r = calc()
            .execute(&p(), serde_json::json!({"expression":"2+3"}))
            .await
            .unwrap();
        assert_eq!(r.output["result"], 5.0);
    }
    #[tokio::test]
    async fn precedence_mult_before_add() {
        let r = calc()
            .execute(&p(), serde_json::json!({"expression":"2+3*4"}))
            .await
            .unwrap();
        assert_eq!(r.output["result"], 14.0);
    }
    #[tokio::test]
    async fn parens_override_precedence() {
        let r = calc()
            .execute(&p(), serde_json::json!({"expression":"(2+3)*4"}))
            .await
            .unwrap();
        assert_eq!(r.output["result"], 20.0);
    }
    #[tokio::test]
    async fn power_right_associative() {
        let r = calc()
            .execute(&p(), serde_json::json!({"expression":"2**10"}))
            .await
            .unwrap();
        assert_eq!(r.output["result"], 1024.0);
    }
    #[tokio::test]
    async fn unary_minus() {
        let r = calc()
            .execute(&p(), serde_json::json!({"expression":"-5+3"}))
            .await
            .unwrap();
        assert_eq!(r.output["result"], -2.0);
    }
    #[tokio::test]
    async fn division_by_zero_err() {
        let err = calc()
            .execute(&p(), serde_json::json!({"expression":"1/0"}))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::Permanent(_)));
    }
    #[tokio::test]
    async fn modulo() {
        let r = calc()
            .execute(&p(), serde_json::json!({"expression":"10%3"}))
            .await
            .unwrap();
        assert_eq!(r.output["result"], 1.0);
    }
    #[tokio::test]
    async fn precision_applied() {
        let r = calc()
            .execute(&p(), serde_json::json!({"expression":"1/3","precision":2}))
            .await
            .unwrap();
        assert_eq!(r.output["formatted"], "0.33");
    }
    #[tokio::test]
    async fn empty_expression_err() {
        let err = calc()
            .execute(&p(), serde_json::json!({"expression":""}))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs { .. }));
    }
    #[tokio::test]
    async fn invalid_expression_err() {
        let err = calc()
            .execute(&p(), serde_json::json!({"expression":"2++3"}))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs { .. }));
    }
}
