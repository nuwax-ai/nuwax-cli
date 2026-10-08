//! Compose .env syntax. Values are resolved locally in source order; no process
//! environment is mutated, and interpolation results remain literal data.

use crate::container::interpolation::{MissingVariables, interpolate_env};
use anyhow::{Result, ensure};
use std::collections::HashMap;

pub(super) fn parse_env_values(
    source: &str,
    host_value: &impl Fn(&str) -> Option<String>,
) -> Result<HashMap<String, String>> {
    let source = source.strip_prefix('\u{feff}').unwrap_or(source);
    let mut values = HashMap::new();
    let mut position = 0;
    let mut assignment = 0;
    while position < source.len() {
        let end = line_end(source, position);
        let line = source[position..end].trim();
        if line.is_empty() || line.starts_with('#') {
            position = next_line(source, end);
            continue;
        }
        assignment += 1;
        let line = line
            .strip_prefix("export")
            .filter(|rest| rest.starts_with([' ', '\t']))
            .map(str::trim_start)
            .unwrap_or(line);
        let separator = line.find(['=', ':']);
        let key = line[..separator.unwrap_or(line.len())].trim();
        ensure!(
            !key.is_empty()
                && key.chars().all(|character| {
                    !character.is_whitespace() && !matches!(character, '#' | '\'' | '"' | '$')
                }),
            "Invalid Compose environment assignment {assignment}"
        );
        let Some(separator) = separator else {
            if let Some(value) = host_value(key) {
                values.insert(key.to_string(), value);
            }
            position = next_line(source, end);
            continue;
        };
        let rhs = line[separator + 1..].trim_start();
        // Derive the RHS byte position from this slice, including optional
        // export/leading whitespace. Quoted values may span physical lines.
        let rhs_position = rhs.as_ptr() as usize - source.as_ptr() as usize;
        let (value, literal, record_end) = parse_value(source, rhs_position)
            .map_err(|_| anyhow::anyhow!("Invalid Compose environment assignment {assignment}"))?;
        let value = if literal {
            value
        } else {
            interpolate_env(
                &value,
                &|name| host_value(name).or_else(|| values.get(name).cloned()),
                MissingVariables::Empty,
            )
            .map_err(|_| anyhow::anyhow!("Invalid Compose environment assignment {assignment}"))?
        };
        values.insert(key.to_string(), value);
        position = next_line(source, record_end);
    }
    Ok(values)
}

fn line_end(source: &str, start: usize) -> usize {
    source[start..]
        .find('\n')
        .map_or(source.len(), |offset| start + offset)
}

fn next_line(source: &str, end: usize) -> usize {
    if end < source.len() { end + 1 } else { end }
}

fn parse_value(source: &str, start: usize) -> Result<(String, bool, usize)> {
    let mut characters = source[start..].char_indices().peekable();
    let quote = characters
        .peek()
        .map(|(_, character)| *character)
        .filter(|character| matches!(character, '\'' | '"'));
    let Some(quote) = quote else {
        let end = line_end(source, start);
        let raw = &source[start..end];
        let comment = raw.char_indices().find_map(|(index, character)| {
            (character == '#'
                && (index == 0
                    || raw[..index]
                        .chars()
                        .next_back()
                        .is_some_and(char::is_whitespace)))
            .then_some(index)
        });
        return Ok((
            raw[..comment.unwrap_or(raw.len())].trim_end().to_string(),
            false,
            end,
        ));
    };
    characters.next();
    let mut value = String::new();
    while let Some((offset, character)) = characters.next() {
        if character == quote {
            return Ok((value, quote == '\'', line_end(source, start + offset + 1)));
        }
        if character != '\\' {
            value.push(character);
            continue;
        }
        if quote == '\'' {
            if characters.peek().is_some_and(|(_, next)| *next == '\'') {
                characters.next();
                value.push('\'');
            } else {
                value.push('\\');
            }
            continue;
        }
        let Some((_, escaped)) = characters.next() else {
            anyhow::bail!("Unterminated Compose environment quote");
        };
        match escaped {
            'a' => value.push('\u{7}'),
            'b' => value.push('\u{8}'),
            'f' => value.push('\u{c}'),
            'n' => value.push('\n'),
            'r' => value.push('\r'),
            't' => value.push('\t'),
            'v' => value.push('\u{b}'),
            '\\' | '"' => value.push(escaped),
            // Protect a literal dollar from the single interpolation pass.
            '$' => value.push_str("$$"),
            other => {
                value.push('\\');
                value.push(other);
            }
        }
    }
    anyhow::bail!("Unterminated Compose environment quote")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compose_quotes_escapes_and_unquoted_paths() -> Result<()> {
        for (source, expected) in [
            ("VALUE='Let\\'s go'\n", "Let's go"),
            ("VALUE=Let's go\n", "Let's go"),
            ("VALUE=C:\\work\\dir\n", "C:\\work\\dir"),
            ("VALUE='C:\\work\\dir'\n", "C:\\work\\dir"),
            (
                "VALUE=\"one\\ttwo\\rthree\\nlast\"\n",
                "one\ttwo\rthree\nlast",
            ),
            ("VALUE=\"\\$TOKEN ${UNSET:-fallback}\"\n", "$TOKEN fallback"),
            (
                "VALUE='${UNSET:-literal} # : \\path'\n",
                "${UNSET:-literal} # : \\path",
            ),
            ("VALUE='first\r\nlast' # note\r\n", "first\r\nlast"),
            ("VALUE=space # comment\n", "space"),
            ("VALUE=space#literal\n", "space#literal"),
        ] {
            let values = parse_env_values(source, &|_| None)?;
            assert_eq!(values.get("VALUE").map(String::as_str), Some(expected));
        }
        Ok(())
    }

    #[test]
    fn compose_defaults_references_order_and_host_precedence() -> Result<()> {
        let values = parse_env_values(
            concat!(
                "\u{feff}BASE_VALUE=file\r\n",
                "VALUE=$BASE_VALUE\r\n",
                "DEFAULT=${UNSET:-${OTHER:-nested}}\r\n",
                "FORWARD=$LATER_VALUE\r\n",
                "LATER_VALUE=later\r\n",
                "DUPLICATE=first\r\nDUPLICATE=last\r\n",
                "EMPTY=\r\nUNSET_DEFAULT=${EMPTY-default}\r\n",
                "EMPTY_DEFAULT=${EMPTY:-default}\r\n"
            ),
            &|key| (key == "BASE_VALUE").then(|| "host".to_string()),
        )?;
        for (key, expected) in [
            ("VALUE", "host"),
            ("DEFAULT", "nested"),
            ("FORWARD", ""),
            ("DUPLICATE", "last"),
            ("UNSET_DEFAULT", ""),
            ("EMPTY_DEFAULT", "default"),
        ] {
            assert_eq!(values.get(key).map(String::as_str), Some(expected));
        }
        Ok(())
    }

    #[test]
    fn invalid_values_fail_without_disclosing_credentials() {
        for source in [
            "PASSWORD='synthetic-secret",
            "PASSWORD=${MISSING:?synthetic-secret}",
        ] {
            let error = parse_env_values(source, &|_| None).unwrap_err();
            assert!(!error.to_string().contains("synthetic-secret"));
        }
    }

    #[test]
    fn bare_assignments_keep_prior_values_and_use_available_host_values() -> Result<()> {
        let source = "VALUE=file\nVALUE\nUNSET\n";
        let values = parse_env_values(source, &|_| None)?;
        assert_eq!(values.get("VALUE").map(String::as_str), Some("file"));
        assert!(!values.contains_key("UNSET"));
        let values = parse_env_values(source, &|key| (key == "VALUE").then(|| "host".to_string()))?;
        assert_eq!(values.get("VALUE").map(String::as_str), Some("host"));
        Ok(())
    }
}
