//! Arithmetic typed in Start's search, as Plasma's search answers it
//! (KRunner's calculator): `12*7`, `2^10`, `(3+4.5)/2`, `15%`, `sqrt 2`.

/// The value of `text` when it is arithmetic, else None. Text without an
/// operator or a function (a plain number, an application's name) is not.
pub fn evaluate(text: &str) -> Option<f64> {
    let text = text.trim().trim_end_matches('=').trim();
    if !text.chars().any(|c| c.is_ascii_digit() || c == 'π') {
        return None;
    }
    let tokens = tokenize(text)?;
    // A leading minus alone makes a negative number, not a calculation.
    let body = match tokens.first() {
        Some(Token::Op('-')) => &tokens[1..],
        _ => &tokens[..],
    };
    if !body
        .iter()
        .any(|t| matches!(t, Token::Op(_) | Token::Open | Token::Name(_)))
    {
        return None;
    }
    let mut parser = Parser { tokens, at: 0 };
    let value = parser.sum()?;
    (parser.at == parser.tokens.len() && value.is_finite()).then_some(value)
}

/// The result as Start shows it: up to 12 significant digits, no
/// trailing zeros, no exponent for ordinary sizes.
pub fn format(value: f64) -> String {
    if value == 0.0 {
        return "0".into();
    }
    let magnitude = value.abs().log10().floor() as i32;
    if !(-6..15).contains(&magnitude) {
        return format!("{value:.6e}");
    }
    let decimals = (11 - magnitude).clamp(0, 12) as usize;
    let text = format!("{value:.decimals$}");
    let text = if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        text
    };
    if text == "-0" { "0".into() } else { text }
}

#[derive(Clone, Debug, PartialEq)]
enum Token {
    Number(f64),
    Op(char),
    Open,
    Close,
    Name(String),
}

fn tokenize(text: &str) -> Option<Vec<Token>> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            ' ' | '\t' => i += 1,
            '0'..='9' | '.' | ',' => {
                let start = i;
                while i < chars.len()
                    && (chars[i].is_ascii_digit() || chars[i] == '.' || chars[i] == ',')
                {
                    i += 1;
                }
                // A comma is a decimal separator (3,5) as in many languages.
                let number: String = chars[start..i]
                    .iter()
                    .map(|&c| if c == ',' { '.' } else { c })
                    .collect();
                if number.matches('.').count() > 1 {
                    return None;
                }
                tokens.push(Token::Number(number.parse().ok()?));
            }
            '+' | '-' | '*' | '/' | '^' | '%' => {
                tokens.push(Token::Op(c));
                i += 1;
            }
            '×' | 'x' if matches!(tokens.last(), Some(Token::Number(_) | Token::Close)) => {
                tokens.push(Token::Op('*'));
                i += 1;
            }
            '÷' => {
                tokens.push(Token::Op('/'));
                i += 1;
            }
            '(' => {
                tokens.push(Token::Open);
                i += 1;
            }
            ')' => {
                tokens.push(Token::Close);
                i += 1;
            }
            '√' => {
                tokens.push(Token::Name("sqrt".into()));
                i += 1;
            }
            'π' => {
                tokens.push(Token::Name("pi".into()));
                i += 1;
            }
            c if c.is_alphabetic() => {
                let start = i;
                while i < chars.len() && chars[i].is_alphabetic() {
                    i += 1;
                }
                let name: String = chars[start..i].iter().collect::<String>().to_lowercase();
                tokens.push(Token::Name(name));
            }
            _ => return None,
        }
    }
    Some(tokens)
}

struct Parser {
    tokens: Vec<Token>,
    at: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.at)
    }

    fn take_op(&mut self, ops: &[char]) -> Option<char> {
        match self.peek() {
            Some(Token::Op(op)) if ops.contains(op) => {
                let op = *op;
                self.at += 1;
                Some(op)
            }
            _ => None,
        }
    }

    /// sum: product (('+' | '-') product)*
    fn sum(&mut self) -> Option<f64> {
        let mut value = self.product()?;
        while let Some(op) = self.take_op(&['+', '-']) {
            let rhs = self.product()?;
            value = if op == '+' { value + rhs } else { value - rhs };
        }
        Some(value)
    }

    /// product: unary (('*' | '/') unary | implicit multiplication)*
    fn product(&mut self) -> Option<f64> {
        let mut value = self.unary()?;
        loop {
            if let Some(op) = self.take_op(&['*', '/']) {
                let rhs = self.unary()?;
                value = if op == '*' { value * rhs } else { value / rhs };
            } else if matches!(self.peek(), Some(Token::Open | Token::Name(_))) {
                // 2(3+4), 2pi, 3 sqrt 4
                value *= self.unary()?;
            } else {
                return Some(value);
            }
        }
    }

    /// unary: '-' unary | '+' unary | power
    fn unary(&mut self) -> Option<f64> {
        match self.take_op(&['-', '+']) {
            Some('-') => Some(-self.unary()?),
            Some(_) => self.unary(),
            None => self.power(),
        }
    }

    /// power: postfix ('^' unary)?  (right-associative)
    fn power(&mut self) -> Option<f64> {
        let base = self.postfix()?;
        if self.take_op(&['^']).is_some() {
            return Some(base.powf(self.unary()?));
        }
        Some(base)
    }

    /// postfix: atom '%'*
    fn postfix(&mut self) -> Option<f64> {
        let mut value = self.atom()?;
        while self.take_op(&['%']).is_some() {
            value /= 100.0;
        }
        Some(value)
    }

    fn atom(&mut self) -> Option<f64> {
        let token = self.peek()?.clone();
        self.at += 1;
        match token {
            Token::Number(n) => Some(n),
            Token::Open => {
                let value = self.sum()?;
                if self.peek() == Some(&Token::Close) {
                    self.at += 1;
                }
                // An unclosed parenthesis is closed at the end, as typed
                // so far.
                Some(value)
            }
            Token::Name(name) => match name.as_str() {
                "pi" => Some(std::f64::consts::PI),
                "e" => Some(std::f64::consts::E),
                function => {
                    let argument = self.power()?;
                    Some(match function {
                        "sqrt" => argument.sqrt(),
                        "sin" => argument.sin(),
                        "cos" => argument.cos(),
                        "tan" => argument.tan(),
                        "ln" => argument.ln(),
                        "log" => argument.log10(),
                        "abs" => argument.abs(),
                        _ => return None,
                    })
                }
            },
            Token::Op(_) | Token::Close => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn calc(text: &str) -> Option<String> {
        evaluate(text).map(format)
    }

    #[test]
    fn arithmetic() {
        assert_eq!(calc("12*7").as_deref(), Some("84"));
        assert_eq!(calc("2+3*4").as_deref(), Some("14"));
        assert_eq!(calc("(2+3)*4").as_deref(), Some("20"));
        assert_eq!(calc("2^10").as_deref(), Some("1024"));
        assert_eq!(calc("2^3^2").as_deref(), Some("512"), "right-associative");
        assert_eq!(calc("-2^2").as_deref(), Some("-4"));
        assert_eq!(calc("10/4").as_deref(), Some("2.5"));
        assert_eq!(calc("3,5*2").as_deref(), Some("7"), "comma decimals");
        assert_eq!(calc("1/3").as_deref(), Some("0.333333333333"));
        assert_eq!(calc("15%").as_deref(), Some("0.15"));
        assert_eq!(calc("200*15%").as_deref(), Some("30"));
        assert_eq!(calc("6 × 7").as_deref(), Some("42"));
        assert_eq!(calc("6x7").as_deref(), Some("42"));
        assert_eq!(calc("9 ÷ 3").as_deref(), Some("3"));
        assert_eq!(calc("sqrt 16").as_deref(), Some("4"));
        assert_eq!(calc("√2").as_deref(), Some("1.41421356237"));
        assert_eq!(calc("2pi").as_deref(), Some("6.28318530718"));
        assert_eq!(calc("2(3+4)").as_deref(), Some("14"));
        assert_eq!(calc("(1+2").as_deref(), Some("3"), "still typing");
        assert_eq!(calc("0.1+0.2").as_deref(), Some("0.3"));
        assert_eq!(calc("1e3"), None, "e is not a unit");
        assert_eq!(calc("10^20").as_deref(), Some("1.000000e20"));
        assert_eq!(calc("12*7=").as_deref(), Some("84"));
    }

    #[test]
    fn not_arithmetic() {
        for text in [
            "", "firefox", "42", "-5", "steam", "7zip", "1/0", "2*", "*2", "1..2+1", "0/0",
        ] {
            assert_eq!(calc(text), None, "{text:?}");
        }
    }
}
