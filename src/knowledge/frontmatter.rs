use crate::error::{AppError, ErrorCategory, Result};
use serde::{Serialize, de::DeserializeOwned};

pub fn parse<T: DeserializeOwned>(bytes: &[u8], kind: &str) -> Result<(T, String)> {
    let text = std::str::from_utf8(bytes).map_err(|_| invalid(kind, "document is not UTF-8"))?;
    if text.starts_with('\u{feff}') {
        return Err(invalid(kind, "a UTF-8 BOM is not allowed"));
    }
    let first_end = line_end(text, 0).ok_or_else(|| invalid(kind, "missing frontmatter"))?;
    if trim_line_end(&text[..first_end]) != "---" {
        return Err(invalid(
            kind,
            "frontmatter must begin with an exact --- line",
        ));
    }
    let yaml_start = first_end;
    let mut cursor = yaml_start;
    let body_start;
    loop {
        let end =
            line_end(text, cursor).ok_or_else(|| invalid(kind, "frontmatter is not closed"))?;
        if trim_line_end(&text[cursor..end]) == "---" {
            body_start = end;
            break;
        }
        cursor = end;
    }
    let yaml_end = cursor;
    let metadata = serde_yaml_ng::from_str(&text[yaml_start..yaml_end])
        .map_err(|error| invalid(kind, format!("invalid frontmatter: {error}")))?;
    Ok((metadata, text[body_start..].to_string()))
}

fn line_end(text: &str, start: usize) -> Option<usize> {
    if start >= text.len() {
        return None;
    }
    text[start..]
        .find('\n')
        .map(|offset| start + offset + 1)
        .or(Some(text.len()))
}

fn trim_line_end(line: &str) -> &str {
    line.strip_suffix("\r\n")
        .or_else(|| line.strip_suffix('\n'))
        .or_else(|| line.strip_suffix('\r'))
        .unwrap_or(line)
}

pub fn serialize<T: Serialize>(metadata: &T, body: &str) -> Result<Vec<u8>> {
    let yaml = serde_yaml_ng::to_string(metadata)
        .map_err(|error| AppError::input(format!("cannot serialize frontmatter: {error}")))?;
    let mut output = format!("---\n{}---\n", yaml.trim_start_matches("---\n"));
    output.push_str(body);
    Ok(output.into_bytes())
}

pub fn normalized_body(body: &str) -> String {
    format!("{}\n", body.trim_end_matches(['\r', '\n']))
}

fn invalid(kind: &str, message: impl Into<String>) -> AppError {
    AppError::new(
        format!("invalid_{kind}"),
        message,
        ErrorCategory::Validation,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Serialize, Deserialize, PartialEq)]
    #[serde(deny_unknown_fields)]
    struct Meta {
        id: String,
    }

    #[test]
    fn parses_lf_and_crlf_and_preserves_body() {
        for bytes in [
            b"---\nid: X\n---\nbody\n".as_slice(),
            b"---\r\nid: X\r\n---\r\nbody\r\n".as_slice(),
        ] {
            let (meta, body): (Meta, String) = parse(bytes, "resource").unwrap();
            assert_eq!(meta.id, "X");
            assert!(body.starts_with("body"));
        }
    }

    #[test]
    fn rejects_bom_unknown_and_missing_delimiter() {
        assert!(parse::<Meta>(b"\xef\xbb\xbf---\nid: X\n---\n", "resource").is_err());
        assert!(parse::<Meta>(b"id: X\n", "resource").is_err());
        assert!(parse::<Meta>(b"---\nid: X\nextra: true\n---\n", "resource").is_err());
    }
}
