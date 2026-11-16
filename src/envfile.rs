// SPDX-FileCopyrightText: Copyright (c) 2026 Daniel Rahn
// SPDX-License-Identifier: Apache-2.0

//! Writing a systemd `EnvironmentFile=`.

use std::collections::BTreeMap;
use std::fmt::Write as _;

/// Why an environment cannot be written.
#[derive(Debug, Eq, PartialEq)]
pub enum EnvFileError {
    /// The name is not `[A-Za-z_][A-Za-z0-9_]*`.
    Name(String),
    /// The value holds a character a line of the file cannot carry.
    Value(String),
}

impl std::fmt::Display for EnvFileError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Name(name) => write!(formatter, "invalid environment variable name {name:?}"),
            Self::Value(name) => write!(
                formatter,
                "environment variable {name} holds a control character"
            ),
        }
    }
}

impl std::error::Error for EnvFileError {}

fn valid_name(name: &str) -> bool {
    let mut characters = name.chars();
    characters
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        && characters.all(|character| character.is_ascii_alphanumeric() || character == '_')
}

/// Renders `environment` as the contents of an `EnvironmentFile=`.
///
/// Each value is double-quoted; inside double quotes systemd reads a
/// backslash before `"`, `\`, `$` and `` ` `` as that character, so these
/// four are escaped and every other character stands for itself. A value
/// with a newline or another control character is refused: the file is read
/// by lines.
pub fn render(environment: &BTreeMap<String, String>) -> Result<String, EnvFileError> {
    let mut rendered = String::new();
    for (name, value) in environment {
        if !valid_name(name) {
            return Err(EnvFileError::Name(name.clone()));
        }
        if value.chars().any(char::is_control) {
            return Err(EnvFileError::Value(name.clone()));
        }
        let _ = write!(rendered, "{name}=\"");
        for character in value.chars() {
            if matches!(character, '"' | '\\' | '$' | '`') {
                rendered.push('\\');
            }
            rendered.push(character);
        }
        rendered.push_str("\"\n");
    }
    Ok(rendered)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn environment(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
        entries
            .iter()
            .map(|(name, value)| ((*name).to_string(), (*value).to_string()))
            .collect()
    }

    #[test]
    fn values_are_quoted_and_the_four_characters_escaped() {
        let rendered = render(&environment(&[
            ("PLAIN", "value with spaces # and = signs"),
            ("JSON", r#"{"a":"b\n","c":"$HOME `id`"}"#),
            ("EMPTY", ""),
        ]))
        .expect("render");
        assert_eq!(
            rendered,
            concat!(
                "EMPTY=\"\"\n",
                "JSON=\"{\\\"a\\\":\\\"b\\\\n\\\",\\\"c\\\":\\\"\\$HOME \\`id\\`\\\"}\"\n",
                "PLAIN=\"value with spaces # and = signs\"\n",
            )
        );
    }

    #[test]
    fn a_value_that_would_leave_its_line_is_refused() {
        assert_eq!(
            render(&environment(&[("A", "one\ntwo")])),
            Err(EnvFileError::Value("A".into()))
        );
        assert_eq!(
            render(&environment(&[("A", "bell\u{7}")])),
            Err(EnvFileError::Value("A".into()))
        );
    }

    #[test]
    fn a_name_that_is_not_a_variable_name_is_refused() {
        for name in ["", "1A", "A B", "A=B", "A\nB", "A-B"] {
            assert_eq!(
                render(&environment(&[(name, "v")])),
                Err(EnvFileError::Name(name.into())),
                "{name:?}"
            );
        }
    }
}
