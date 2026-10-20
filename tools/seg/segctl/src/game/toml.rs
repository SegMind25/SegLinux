//! The small part of TOML that ~/.config/seg/games.toml uses: `[[game]]`
//! tables with string, integer, boolean and string-array values, comments
//! and blank lines. Anything else is an error that names the line, so a typo
//! is not silently ignored.

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Str(String),
    Int(i64),
    Bool(bool),
    Array(Vec<Value>),
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Table {
    /// The `[[name]]` it belongs to; empty before the first header.
    pub name: String,
    pub line: usize,
    pub entries: Vec<(String, Value, usize)>,
}

impl Table {
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.entries
            .iter()
            .find(|(k, _, _)| k == key)
            .map(|(_, v, _)| v)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct ParseError {
    pub line: usize,
    pub message: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

fn err<T>(line: usize, message: impl Into<String>) -> Result<T, ParseError> {
    Err(ParseError {
        line,
        message: message.into(),
    })
}

/// Cut a `#` comment off a line, unless the `#` is inside a string.
fn strip_comment(line: &str) -> &str {
    let mut in_basic = false;
    let mut in_literal = false;
    let mut escaped = false;
    for (i, c) in line.char_indices() {
        match c {
            '\\' if in_basic && !escaped => {
                escaped = true;
                continue;
            }
            '"' if !in_literal && !escaped => in_basic = !in_basic,
            '\'' if !in_basic => in_literal = !in_literal,
            '#' if !in_basic && !in_literal => return line.get(..i).unwrap_or(line),
            _ => {}
        }
        escaped = false;
    }
    line
}

fn parse_string(text: &str, line: usize) -> Result<(String, &str), ParseError> {
    let mut chars = text.char_indices();
    let quote = match chars.next() {
        Some((_, q @ ('"' | '\''))) => q,
        _ => return err(line, "expected a string"),
    };
    let mut out = String::new();
    let mut escaped = false;
    for (i, c) in chars {
        if quote == '\'' {
            if c == '\'' {
                return Ok((out, text.get(i + 1..).unwrap_or("")));
            }
            out.push(c);
        } else if escaped {
            out.push(match c {
                'n' => '\n',
                't' => '\t',
                '"' => '"',
                '\\' => '\\',
                other => return err(line, format!("unknown escape \\{other}")),
            });
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == '"' {
            return Ok((out, text.get(i + 1..).unwrap_or("")));
        } else {
            out.push(c);
        }
    }
    err(line, "a string is not closed")
}

/// One value from the start of `text`, and what follows it.
fn parse_value(text: &str, line: usize) -> Result<(Value, &str), ParseError> {
    let text = text.trim_start();
    match text.chars().next() {
        Some('"' | '\'') => {
            let (s, rest) = parse_string(text, line)?;
            Ok((Value::Str(s), rest))
        }
        Some('[') => {
            let mut items = Vec::new();
            let mut rest = text.get(1..).unwrap_or("").trim_start();
            loop {
                if let Some(after) = rest.strip_prefix(']') {
                    return Ok((Value::Array(items), after));
                }
                let (item, after) = parse_value(rest, line)?;
                if matches!(item, Value::Array(_)) {
                    return err(line, "arrays inside arrays are not supported");
                }
                items.push(item);
                rest = after.trim_start();
                if let Some(after) = rest.strip_prefix(',') {
                    rest = after.trim_start();
                } else if !rest.starts_with(']') {
                    return err(line, "expected `,` or `]` in an array");
                }
            }
        }
        Some(_) => {
            let end = text
                .find(|c: char| c == ',' || c == ']' || c.is_whitespace())
                .unwrap_or(text.len());
            let (word, rest) = (text.get(..end).unwrap_or(""), text.get(end..).unwrap_or(""));
            let value = match word {
                "true" => Value::Bool(true),
                "false" => Value::Bool(false),
                _ => match word.replace('_', "").parse::<i64>() {
                    Ok(n) => Value::Int(n),
                    Err(_) => {
                        return err(
                            line,
                            format!("`{word}` is not a value (strings need quotes)"),
                        );
                    }
                },
            };
            Ok((value, rest))
        }
        None => err(line, "a value is missing"),
    }
}

fn valid_key(key: &str) -> bool {
    !key.is_empty()
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Parse a document into its tables, in order.
pub fn parse(text: &str) -> Result<Vec<Table>, ParseError> {
    let mut tables = vec![Table::default()];
    // A multi-line array being read: key, the text so far, its first line.
    let mut pending: Option<(String, String, usize)> = None;
    for (index, raw) in text.lines().enumerate() {
        let line_no = index + 1;
        let line = strip_comment(raw).trim();
        let (key, value_text, first_line) = match pending.take() {
            Some((key, mut so_far, first)) => {
                so_far.push(' ');
                so_far.push_str(line);
                (key, so_far, first)
            }
            None => {
                if line.is_empty() {
                    continue;
                }
                if let Some(name) = line.strip_prefix("[[") {
                    let Some(name) = name.strip_suffix("]]") else {
                        return err(line_no, "a table header must end with `]]`");
                    };
                    let name = name.trim();
                    if !valid_key(name) {
                        return err(line_no, format!("`{name}` is not a table name"));
                    }
                    tables.push(Table {
                        name: name.to_string(),
                        line: line_no,
                        entries: Vec::new(),
                    });
                    continue;
                }
                if line.starts_with('[') {
                    return err(line_no, "only [[table]] headers are supported");
                }
                let Some((key, value)) = line.split_once('=') else {
                    return err(line_no, "expected `key = value`");
                };
                let key = key.trim();
                if !valid_key(key) {
                    return err(line_no, format!("`{key}` is not a key"));
                }
                (key.to_string(), value.trim().to_string(), line_no)
            }
        };
        // An array whose `]` has not come yet continues on the next line.
        if value_text.starts_with('[') && !array_is_closed(&value_text) {
            pending = Some((key, value_text, first_line));
            continue;
        }
        let (value, rest) = parse_value(&value_text, first_line)?;
        if !rest.trim().is_empty() {
            return err(
                first_line,
                format!("unexpected `{}` after the value", rest.trim()),
            );
        }
        let Some(table) = tables.last_mut() else {
            return err(first_line, "no table");
        };
        if table.get(&key).is_some() {
            return err(first_line, format!("`{key}` is set twice"));
        }
        table.entries.push((key, value, first_line));
    }
    if let Some((_, _, first)) = pending {
        return err(first, "an array is not closed");
    }
    // The part before the first header must be empty.
    if let Some(first) = tables.first()
        && let Some((key, _, line)) = first.entries.first()
    {
        return err(*line, format!("`{key}` is outside a [[game]] table"));
    }
    tables.remove(0);
    Ok(tables)
}

/// Whether the `[` at the start of `text` has its closing `]` (brackets in
/// strings do not count).
fn array_is_closed(text: &str) -> bool {
    let mut depth = 0i32;
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for c in text.chars() {
        match quote {
            Some(q) => {
                if q == '"' && c == '\\' && !escaped {
                    escaped = true;
                    continue;
                }
                if c == q && !escaped {
                    quote = None;
                }
                escaped = false;
            }
            None => match c {
                '"' | '\'' => quote = Some(c),
                '[' => depth += 1,
                ']' => depth -= 1,
                _ => {}
            },
        }
    }
    depth <= 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &str) -> Value {
        Value::Str(v.into())
    }

    #[test]
    fn tables_and_values() {
        let doc = r#"
# my games
[[game]]
match = "eldenring.exe"   # the executable
governor = 'performance'
nice = -5
mangohud = true
gamescope = ["-W", "2560", "-H", "1440"]

[[game]]
match = "cs2"
pin = "0-7,16-23"
"#;
        let t = parse(doc).unwrap();
        assert_eq!(t.len(), 2);
        assert_eq!(t[0].name, "game");
        assert_eq!(t[0].line, 3);
        assert_eq!(t[0].get("match"), Some(&s("eldenring.exe")));
        assert_eq!(t[0].get("governor"), Some(&s("performance")));
        assert_eq!(t[0].get("nice"), Some(&Value::Int(-5)));
        assert_eq!(t[0].get("mangohud"), Some(&Value::Bool(true)));
        assert_eq!(
            t[0].get("gamescope"),
            Some(&Value::Array(vec![s("-W"), s("2560"), s("-H"), s("1440")]))
        );
        assert_eq!(t[1].get("pin"), Some(&s("0-7,16-23")));
        assert_eq!(t[1].get("governor"), None);
    }

    #[test]
    fn strings_keep_hashes_and_escapes() {
        let t = parse("[[game]]\nmatch = \"a#b\"  # comment\nname = \"say \\\"hi\\\"\\n\"\nlit = 'C:\\path # x'\n").unwrap();
        assert_eq!(t[0].get("match"), Some(&s("a#b")));
        assert_eq!(t[0].get("name"), Some(&s("say \"hi\"\n")));
        assert_eq!(t[0].get("lit"), Some(&s("C:\\path # x")));
    }

    #[test]
    fn arrays_may_span_lines_and_end_with_a_comma() {
        let doc = "[[game]]\nmatch = \"x\"\ngamescope = [\n  \"-W\", \"1920\",  # width\n  \"--adaptive-sync\",\n]\nempty = []\n";
        let t = parse(doc).unwrap();
        assert_eq!(
            t[0].get("gamescope"),
            Some(&Value::Array(vec![
                s("-W"),
                s("1920"),
                s("--adaptive-sync")
            ]))
        );
        assert_eq!(t[0].get("empty"), Some(&Value::Array(vec![])));
        assert_eq!(
            t[0].entries
                .iter()
                .find(|e| e.0 == "gamescope")
                .map(|e| e.2),
            Some(3)
        );
    }

    #[test]
    fn mistakes_name_their_line() {
        let cases = [
            ("[[game]]\nmatch = eldenring\n", 2, "strings need quotes"),
            ("[[game]]\nmatch\n", 2, "expected `key = value`"),
            ("[game]\n", 1, "only [[table]] headers"),
            ("[[game]\n", 1, "must end with `]]`"),
            ("[[game]]\nmatch = \"a\nx = 1\n", 2, "not closed"),
            ("[[game]]\na = 1\na = 2\n", 3, "set twice"),
            ("[[game]]\na = [1, 2\n", 2, "not closed"),
            ("[[game]]\na = [[1]]\n", 2, "inside arrays"),
            ("[[game]]\na = 1 2\n", 2, "unexpected"),
            ("a = 1\n", 1, "outside a [[game]] table"),
            ("[[game]]\n= 1\n", 2, "is not a key"),
            ("[[game]]\na = \"\\q\"\n", 2, "unknown escape"),
        ];
        for (doc, line, fragment) in cases {
            let e = parse(doc).unwrap_err();
            assert_eq!(e.line, line, "{doc:?} -> {e}");
            assert!(e.to_string().contains(fragment), "{doc:?} -> {e}");
        }
    }

    #[test]
    fn empty_documents_have_no_tables() {
        assert_eq!(parse("").unwrap(), vec![]);
        assert_eq!(parse("# only a comment\n\n").unwrap(), vec![]);
    }
}
