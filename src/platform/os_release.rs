use std::collections::BTreeMap;

use anyhow::{bail, Context, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct OsRelease {
    fields: BTreeMap<String, String>,
}

impl OsRelease {
    pub(super) fn parse(input: &str) -> Result<Self> {
        let mut fields = BTreeMap::new();
        for (index, line) in input.lines().enumerate() {
            let line_number = index + 1;
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (key, value) = line
                .split_once('=')
                .with_context(|| format!("os-release line {line_number} is not KEY=VALUE"))?;
            validate_key(key)
                .with_context(|| format!("invalid os-release key on line {line_number}"))?;
            let value = parse_value(value)
                .with_context(|| format!("invalid os-release value on line {line_number}"))?;
            // os-release(5) specifies that readers use the later assignment.
            fields.insert(key.to_owned(), value);
        }
        Ok(Self { fields })
    }

    pub(super) fn get(&self, key: &str) -> Option<&str> {
        self.fields.get(key).map(String::as_str)
    }
}

fn validate_key(key: &str) -> Result<()> {
    let mut bytes = key.bytes();
    if !bytes.next().is_some_and(|byte| byte.is_ascii_uppercase())
        || !bytes.all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
    {
        bail!("keys must match [A-Z][A-Z0-9_]*");
    }
    Ok(())
}

fn parse_value(value: &str) -> Result<String> {
    if value.bytes().any(|byte| byte.is_ascii_control()) {
        bail!("control characters are not allowed");
    }
    match value.as_bytes().first().copied() {
        Some(b'\'') => parse_single_quoted(value),
        Some(b'"') => parse_double_quoted(value),
        _ => parse_unquoted(value),
    }
}

fn parse_single_quoted(value: &str) -> Result<String> {
    let body = value
        .strip_prefix('\'')
        .and_then(|value| value.strip_suffix('\''))
        .context("unmatched single quote")?;
    if body.contains('\'') {
        bail!("single-quoted values cannot contain a single quote");
    }
    Ok(body.to_owned())
}

fn parse_double_quoted(value: &str) -> Result<String> {
    let body = value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .context("unmatched double quote")?;
    let mut output = String::with_capacity(body.len());
    let mut chars = body.chars();
    while let Some(character) = chars.next() {
        match character {
            '\\' => {
                let escaped = chars.next().context("dangling escape")?;
                if matches!(escaped, '$' | '"' | '\\' | '`') {
                    output.push(escaped);
                } else {
                    // POSIX double quotes preserve a backslash before other characters.
                    output.push('\\');
                    output.push(escaped);
                }
            }
            '"' => bail!("unescaped double quote"),
            '$' | '`' => bail!("variable and command expansion are not supported"),
            _ => output.push(character),
        }
    }
    Ok(output)
}

fn parse_unquoted(value: &str) -> Result<String> {
    let mut output = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(character) = chars.next() {
        match character {
            '\\' => output.push(chars.next().context("dangling escape")?),
            '\'' | '"' => bail!("quoted and unquoted fragments cannot be concatenated"),
            '$' | '`' => bail!("variable and command expansion are not supported"),
            character
                if character.is_ascii_whitespace()
                    || matches!(
                        character,
                        '|' | '&'
                            | ';'
                            | '('
                            | ')'
                            | '<'
                            | '>'
                            | '*'
                            | '?'
                            | '['
                            | ']'
                            | '{'
                            | '}'
                            | '~'
                            | '#'
                    ) =>
            {
                bail!("shell-special characters must be quoted or escaped")
            }
            _ => output.push(character),
        }
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_supported_shell_quoting_and_escapes() {
        let release = OsRelease::parse(
            "ID=ubuntu\nSINGLE='literal $HOME'\nDOUBLE=\"a \\\"quote\\\" \\\\ \\$ \\`\"\nUNQUOTED=a\\ b\\;c\n",
        )
        .unwrap();
        assert_eq!(release.get("ID"), Some("ubuntu"));
        assert_eq!(release.get("SINGLE"), Some("literal $HOME"));
        assert_eq!(release.get("DOUBLE"), Some("a \"quote\" \\ $ `"));
        assert_eq!(release.get("UNQUOTED"), Some("a b;c"));

        let release = OsRelease::parse("VALUE=\"keep\\\\q\"\n").unwrap();
        assert_eq!(release.get("VALUE"), Some(r"keep\q"));
    }

    #[test]
    fn later_duplicate_assignment_wins() {
        let release = OsRelease::parse("ID=debian\nID=ubuntu\n").unwrap();
        assert_eq!(release.get("ID"), Some("ubuntu"));
    }

    #[test]
    fn rejects_malformed_or_executable_shell_syntax() {
        for input in [
            "lower=value\n",
            "ID\n",
            "ID='unterminated\n",
            "ID=\"unterminated\n",
            "ID=value\\\n",
            "ID=\"$HOME\"\n",
            "ID=foo'bar'\n",
            "ID=foo bar\n",
            "ID=foo&bar\n",
        ] {
            assert!(OsRelease::parse(input).is_err(), "accepted {input:?}");
        }
    }
}
