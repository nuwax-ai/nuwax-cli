//! Compose scalar interpolation. Lookup results are literal data; only selected
//! default/alternate words are recursively expanded.

use anyhow::{Result, bail};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MissingVariables {
    Empty,
    Error,
}

pub(crate) fn interpolate_env(
    input: &str,
    lookup: &impl Fn(&str) -> Option<String>,
    missing: MissingVariables,
) -> Result<String> {
    interpolate_env_with_required(input, lookup, missing, &mut RequiredPolicy::Fail)
}

/// Collect only required references evaluated by Compose's interpolation rules.
/// Escaped dollars and unselected default/alternate words are never inspected.
pub(crate) fn missing_required_variables(
    input: &str,
    lookup: &impl Fn(&str) -> Option<String>,
) -> Result<Vec<String>> {
    let mut keys = Vec::new();
    interpolate_env_with_required(
        input,
        lookup,
        MissingVariables::Empty,
        &mut RequiredPolicy::Collect(&mut keys),
    )?;
    Ok(keys)
}

enum RequiredPolicy<'a> {
    Fail,
    Collect(&'a mut Vec<String>),
}

fn interpolate_env_with_required(
    input: &str,
    lookup: &impl Fn(&str) -> Option<String>,
    missing: MissingVariables,
    required: &mut RequiredPolicy<'_>,
) -> Result<String> {
    let mut output = String::with_capacity(input.len());
    let mut position = 0;
    while position < input.len() {
        let remainder = &input[position..];
        if remainder.starts_with("$$") {
            output.push('$');
            position += 2;
        } else if remainder.starts_with('$') {
            if let Some((reference, end)) = parse_reference(input, position)? {
                output.push_str(&resolve(reference, lookup, missing, required)?);
                position = end;
            } else {
                output.push('$');
                position += 1;
            }
        } else if let Some(character) = remainder.chars().next() {
            output.push(character);
            position += character.len_utf8();
        }
    }
    Ok(output)
}

pub(super) fn is_whole_reference(input: &str) -> bool {
    input.starts_with('$')
        && !input.starts_with("$$")
        && parse_reference(input, 0)
            .ok()
            .flatten()
            .is_some_and(|(_, end)| end == input.len())
}

struct Reference<'a> {
    name: &'a str,
    operation: Operation<'a>,
}

enum Operation<'a> {
    Value,
    Default { word: &'a str, empty: bool },
    Alternate { word: &'a str, empty: bool },
    Required { empty: bool },
}

fn name_start(character: u8) -> bool {
    character == b'_' || character.is_ascii_alphabetic()
}

fn name_continuation(character: u8) -> bool {
    character == b'_' || character.is_ascii_alphanumeric()
}

fn parse_reference(input: &str, start: usize) -> Result<Option<(Reference<'_>, usize)>> {
    let Some(&next) = input.as_bytes().get(start + 1) else {
        return Ok(None);
    };
    if next == b'{' {
        let mut depth = 1;
        let mut end = None;
        for (offset, character) in input[start + 2..].char_indices() {
            match character {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(start + 2 + offset);
                        break;
                    }
                }
                _ => {}
            }
        }
        // compose-go first looks for a balanced closing brace, then uses the
        // last closing brace matched by its default-word pattern. This permits
        // literal, unmatched '{' characters in defaults without changing how
        // balanced literal braces and nested variable words are consumed.
        let end = end.or_else(|| {
            input[start + 2..]
                .rfind('}')
                .map(|offset| start + 2 + offset)
        });
        let Some(end) = end else {
            bail!("Unterminated Compose variable expression");
        };
        let expression = &input[start + 2..end];
        if !expression
            .as_bytes()
            .first()
            .is_some_and(|first| name_start(*first))
        {
            bail!("Invalid Compose variable expression");
        }
        let name_end = expression
            .bytes()
            .take_while(|character| name_continuation(*character))
            .count();
        let (name, suffix) = expression.split_at(name_end);
        let operation = if suffix.is_empty() {
            Operation::Value
        } else if let Some(word) = suffix.strip_prefix(":-") {
            Operation::Default { word, empty: true }
        } else if let Some(word) = suffix.strip_prefix('-') {
            Operation::Default { word, empty: false }
        } else if let Some(word) = suffix.strip_prefix(":+") {
            Operation::Alternate { word, empty: true }
        } else if let Some(word) = suffix.strip_prefix('+') {
            Operation::Alternate { word, empty: false }
        } else if suffix.starts_with(":?") {
            // Required-message words can contain secrets. Never evaluate them
            // or include them in returned errors.
            Operation::Required { empty: true }
        } else if suffix.starts_with('?') {
            Operation::Required { empty: false }
        } else {
            bail!("Invalid Compose variable operator");
        };
        Ok(Some((Reference { name, operation }, end + 1)))
    } else if name_start(next) {
        let length = input[start + 1..]
            .bytes()
            .take_while(|character| name_continuation(*character))
            .count();
        let end = start + 1 + length;
        Ok(Some((
            Reference {
                name: &input[start + 1..end],
                operation: Operation::Value,
            },
            end,
        )))
    } else {
        // Native Compose retains standalone dollars and non-variable prefixes.
        Ok(None)
    }
}

fn resolve(
    reference: Reference<'_>,
    lookup: &impl Fn(&str) -> Option<String>,
    missing: MissingVariables,
    required: &mut RequiredPolicy<'_>,
) -> Result<String> {
    let value = lookup(reference.name);
    match reference.operation {
        Operation::Value => match value {
            Some(value) => Ok(value),
            None if missing == MissingVariables::Empty => Ok(String::new()),
            None => bail!("Missing Compose environment variable: {}", reference.name),
        },
        Operation::Default { word, empty } => {
            if value.as_ref().is_none_or(|value| empty && value.is_empty()) {
                interpolate_env_with_required(word, lookup, missing, required)
            } else {
                Ok(value.unwrap_or_default())
            }
        }
        Operation::Alternate { word, empty } => {
            if value
                .as_ref()
                .is_some_and(|value| !empty || !value.is_empty())
            {
                interpolate_env_with_required(word, lookup, missing, required)
            } else {
                Ok(String::new())
            }
        }
        Operation::Required { empty } => {
            if value.as_ref().is_none_or(|value| empty && value.is_empty()) {
                match required {
                    RequiredPolicy::Fail => bail!(
                        "Required Compose environment variable is missing or empty: {}",
                        reference.name
                    ),
                    RequiredPolicy::Collect(keys) => keys.push(reference.name.to_string()),
                }
            }
            Ok(value.unwrap_or_default())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lookup(name: &str) -> Option<String> {
        match name {
            "SET" => Some("value".to_string()),
            "EMPTY" => Some(String::new()),
            "LITERAL" => Some(r#"$word ${word} $$ # literal: \"quote\""#.to_string()),
            _ => None,
        }
    }

    #[test]
    fn defaults_distinguish_unset_and_empty_values() -> Result<()> {
        for (expression, expected) in [
            ("${UNSET-default}", "default"),
            ("${EMPTY-default}", ""),
            ("${SET-default}", "value"),
            ("${UNSET:-default}", "default"),
            ("${EMPTY:-default}", "default"),
            ("${SET:-default}", "value"),
        ] {
            assert_eq!(
                interpolate_env(expression, &lookup, MissingVariables::Error)?,
                expected
            );
        }
        Ok(())
    }

    #[test]
    fn alternates_distinguish_unset_and_empty_values() -> Result<()> {
        for (expression, expected) in [
            ("${UNSET+alternate}", ""),
            ("${EMPTY+alternate}", "alternate"),
            ("${SET+alternate}", "alternate"),
            ("${UNSET:+alternate}", ""),
            ("${EMPTY:+alternate}", ""),
            ("${SET:+alternate}", "alternate"),
        ] {
            assert_eq!(
                interpolate_env(expression, &lookup, MissingVariables::Error)?,
                expected
            );
        }
        Ok(())
    }

    #[test]
    fn required_values_do_not_disclose_custom_messages() -> Result<()> {
        assert_eq!(
            interpolate_env("${SET?secret}", &lookup, MissingVariables::Error)?,
            "value"
        );
        assert_eq!(
            interpolate_env("${EMPTY?secret}", &lookup, MissingVariables::Error)?,
            ""
        );
        for expression in [
            "${UNSET?synthetic-private-text}",
            "${UNSET:?synthetic-private-text}",
            "${EMPTY:?synthetic-private-text}",
        ] {
            let error = interpolate_env(expression, &lookup, MissingVariables::Empty).unwrap_err();
            assert!(!error.to_string().contains("synthetic-private-text"));
        }
        Ok(())
    }

    #[test]
    fn nested_words_are_expanded_only_when_selected() -> Result<()> {
        for (expression, expected) in [
            ("${UNSET:-${INNER:-fallback}}", "fallback"),
            ("${SET:+${UNSET:-nested}}", "nested"),
            ("${EMPTY+${UNSET:-present}}", "present"),
            ("${EMPTY:+${UNSET:?secret}}", ""),
            ("${SET:-${UNSET:?secret}}", "value"),
            ("${UNSET:-{literal}}", "{literal}"),
            ("${UNSET:-$${literal}}", "${literal}"),
        ] {
            assert_eq!(
                interpolate_env(expression, &lookup, MissingVariables::Error)?,
                expected
            );
        }
        Ok(())
    }

    #[test]
    fn escaped_dollars_and_resolved_literals_are_never_reexpanded() -> Result<()> {
        assert_eq!(
            interpolate_env("$$SET $${SET} $$$$", &lookup, MissingVariables::Error)?,
            "$SET ${SET} $$"
        );
        assert_eq!(
            interpolate_env("$LITERAL", &lookup, MissingVariables::Error)?,
            lookup("LITERAL").unwrap()
        );
        assert_eq!(
            interpolate_env("${LITERAL}", &lookup, MissingVariables::Error)?,
            lookup("LITERAL").unwrap()
        );
        assert_eq!(
            interpolate_env("$5 a$ b $字 $", &lookup, MissingVariables::Empty)?,
            "$5 a$ b $字 $"
        );
        Ok(())
    }

    #[test]
    fn strict_missing_and_malformed_expressions_fail_safely() -> Result<()> {
        assert_eq!(
            interpolate_env("$UNSET/${UNSET}", &lookup, MissingVariables::Empty)?,
            "/"
        );
        assert!(interpolate_env("$UNSET", &lookup, MissingVariables::Error).is_err());
        for expression in ["${", "${}", "${1}", "${SET/bad}"] {
            assert!(interpolate_env(expression, &lookup, MissingVariables::Empty).is_err());
        }
        Ok(())
    }

    #[test]
    fn whole_reference_detection_handles_nested_words() {
        assert!(is_whole_reference("$SET"));
        assert!(is_whole_reference("${UNSET:-${INNER:-2}}"));
        assert!(is_whole_reference("${SET:+${UNSET:-true}}"));
        assert!(!is_whole_reference("${SET}-suffix"));
        assert!(!is_whole_reference("$${SET}"));
        assert!(!is_whole_reference("plain text"));
    }

    #[test]
    fn literal_open_braces_in_default_words_follow_native_compose() -> Result<()> {
        for (expression, expected) in [
            ("${UNSET:-{word}", "{word"),
            ("${UNSET:-{word}}", "{word}"),
            ("${SET:-{word}}", "value"),
            ("${UNSET:-{${OTHER:-nested}}}", "{nested}"),
            ("${SET:-{${OTHER:-nested}}}", "value"),
            ("${UNSET:-$${word}}", "${word}"),
            ("${SET:-$${word}}", "value"),
            ("${SET:?{message}", "value"),
        ] {
            assert_eq!(
                interpolate_env(expression, &lookup, MissingVariables::Error)?,
                expected
            );
        }
        assert!(interpolate_env("${UNSET:-${OTHER}", &lookup, MissingVariables::Error).is_err());
        Ok(())
    }
}
