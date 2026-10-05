//! Restricted static Gradle declarations. Never evaluates a Gradle/Groovy/Kotlin script.
use serde_json::{json, Value};
use std::collections::BTreeMap;

#[derive(Debug, PartialEq, Eq)]
enum Token {
    Word(String),
    Literal(String),
    Mark(char),
}

fn tokens(text: &str) -> Result<Vec<Token>, &'static str> {
    let mut input = text.chars().peekable();
    let mut out = Vec::new();
    while let Some(character) = input.next() {
        if character == '\n' {
            out.push(Token::Mark('\n'));
            continue;
        }
        if character.is_whitespace() {
            continue;
        }
        if character == '/' && input.peek() == Some(&'/') {
            input.next();
            for c in input.by_ref() {
                if c == '\n' {
                    out.push(Token::Mark('\n'));
                    break;
                }
            }
            continue;
        }
        if character == '/' && input.peek() == Some(&'*') {
            input.next();
            let mut ended = false;
            while let Some(c) = input.next() {
                if c == '*' && input.peek() == Some(&'/') {
                    input.next();
                    ended = true;
                    break;
                }
            }
            if !ended {
                return Err("gradle_comment_incomplete");
            }
            continue;
        }
        if matches!(character, '\'' | '"') {
            let mut value = String::new();
            let mut ended = false;
            while let Some(c) = input.next() {
                if c == character {
                    ended = true;
                    break;
                }
                if c == '\\' {
                    let escaped = input.next().ok_or("gradle_literal_incomplete")?;
                    if escaped != character && escaped != '\\' {
                        return Err("gradle_literal_escape_unresolved");
                    }
                    value.push(escaped);
                } else {
                    value.push(c);
                }
            }
            if !ended {
                return Err("gradle_literal_incomplete");
            }
            out.push(Token::Literal(value));
        } else if character.is_ascii_alphabetic() || character == '_' {
            let mut word = character.to_string();
            while input
                .peek()
                .is_some_and(|c| c.is_ascii_alphanumeric() || *c == '_')
            {
                word.push(input.next().expect("peeked character"));
            }
            out.push(Token::Word(word));
        } else {
            out.push(Token::Mark(character));
        }
    }
    Ok(out)
}

fn is_mark(input: &[Token], at: usize, mark: char) -> bool {
    input.get(at) == Some(&Token::Mark(mark))
}

fn skip_newlines(input: &[Token], mut at: usize) -> usize {
    while is_mark(input, at, '\n') {
        at += 1;
    }
    at
}

fn coordinate(
    input: &[Token],
    start: usize,
    nesting: usize,
) -> Result<(String, usize), &'static str> {
    if nesting > 3 {
        return Err("gradle_declaration_nesting_unresolved");
    }
    let start = if nesting > 0 {
        skip_newlines(input, start)
    } else {
        start
    };
    match input.get(start) {
        Some(Token::Literal(value)) => Ok((value.clone(), start + 1)),
        Some(Token::Mark('(')) => {
            let (value, end) = coordinate(input, start + 1, nesting + 1)?;
            let end = skip_newlines(input, end);
            if !is_mark(input, end, ')') {
                return Err("gradle_declaration_unresolved");
            }
            Ok((value, end + 1))
        }
        Some(Token::Word(wrapper))
            if matches!(wrapper.as_str(), "platform" | "enforcedPlatform") =>
        {
            if !is_mark(input, start + 1, '(') {
                return Err("gradle_declaration_unresolved");
            }
            coordinate(input, start + 1, nesting + 1)
        }
        Some(Token::Word(_)) => {
            let mut fields = BTreeMap::new();
            let mut at = start;
            loop {
                let Some(Token::Word(key)) = input.get(at) else {
                    return Err("gradle_declaration_unresolved");
                };
                if !["group", "name", "version"].contains(&key.as_str()) {
                    return Err("gradle_declaration_unresolved");
                }
                if !is_mark(input, at + 1, ':') && !is_mark(input, at + 1, '=') {
                    return Err("gradle_declaration_unresolved");
                }
                let Some(Token::Literal(value)) = input.get(at + 2) else {
                    return Err("gradle_declaration_unresolved");
                };
                if fields.insert(key.as_str(), value.as_str()).is_some() {
                    return Err("gradle_declaration_duplicate_field");
                }
                at += 3;
                if !is_mark(input, at, ',') {
                    break;
                }
                at = skip_newlines(input, at + 1);
                if nesting > 0 && is_mark(input, at, ')') {
                    break;
                }
            }
            if fields.len() != 3 {
                return Err("gradle_declaration_unresolved");
            }
            Ok((
                format!(
                    "{}:{}:{}",
                    fields["group"], fields["name"], fields["version"]
                ),
                at,
            ))
        }
        _ => Err("gradle_declaration_unresolved"),
    }
}

pub(super) fn parse(text: &str) -> Result<Vec<Value>, &'static str> {
    let input = tokens(text)?;
    let mut depth: usize = 0;
    let mut block = None;
    let mut rows = Vec::new();
    let mut at = 0;
    const CONFIGURATIONS: &[&str] = &[
        "api",
        "implementation",
        "compileOnly",
        "runtimeOnly",
        "testImplementation",
        "testCompileOnly",
        "testRuntimeOnly",
        "annotationProcessor",
        "testAnnotationProcessor",
        "kapt",
        "ksp",
        "compile",
        "runtime",
        "testCompile",
        "testRuntime",
    ];
    while at < input.len() {
        match &input[at] {
            Token::Mark('{') => depth += 1,
            Token::Mark('}') => {
                if block == Some(depth) {
                    block = None;
                }
                depth = depth.checked_sub(1).ok_or("gradle_block_invalid")?;
            }
            Token::Word(name)
                if name == "dependencies" && block.is_none() && is_mark(&input, at + 1, '{') =>
            {
                block = Some(depth + 1);
            }
            Token::Word(name) if block == Some(depth) => {
                if !CONFIGURATIONS.contains(&name.as_str()) {
                    return Err("gradle_configuration_unresolved");
                }
                let (value, end) = coordinate(&input, at + 1, 0)?;
                // A concatenation, method call, or extra argument is not a literal declaration.
                if !matches!(
                    input.get(end),
                    None | Some(Token::Mark('\n' | ';' | '}' | '{'))
                ) {
                    return Err("gradle_declaration_unresolved");
                }
                if value.contains('$') {
                    return Err("gradle_version_unresolved");
                }
                let parts = value.split(':').collect::<Vec<_>>();
                if !(3..=4).contains(&parts.len())
                    || parts.iter().any(|part| part.trim().is_empty())
                {
                    return Err("gradle_coordinate_invalid");
                }
                rows.push(json!({"name":format!("{}:{}",parts[0],parts[1]),"requirement":parts[2],"scope":"java"}));
                at = end;
                continue;
            }
            Token::Mark(mark) if block == Some(depth) && !matches!(mark, '\n' | ';') => {
                return Err("gradle_declaration_unresolved");
            }
            Token::Literal(_) if block == Some(depth) => {
                return Err("gradle_declaration_unresolved")
            }
            _ => {}
        }
        at += 1;
    }
    if depth != 0 {
        return Err("gradle_block_invalid");
    }
    Ok(rows)
}
