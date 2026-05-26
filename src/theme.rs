//! Theme file parser.
//!
//! Extracts `--plume-*: value;` declarations from a CSS file. The Sprint 2
//! resolve pipeline can layer these over built-in defaults (between built-ins
//! and the `defaults {}` block). Sprint 6 wires this to the `--theme` CLI flag.
//!
//! Intentionally a line scanner, not a CSS parser — we only care about a flat
//! set of custom-property declarations. Anything else in the file is ignored.

/// Extract `(name_without_plume_prefix, raw_value_string)` pairs from CSS-like
/// text. Names without the `--plume-` prefix are skipped — those are not
/// Plume's to own.
pub fn extract_plume_vars(src: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let cleaned = strip_block_comments(src);
    // Split on `;` to walk declarations one at a time (works whether they sit
    // on separate lines or share a line).
    for decl in cleaned.split(';') {
        let decl = decl.trim();
        if decl.is_empty() {
            continue;
        }
        let Some(start) = decl.find("--plume-") else {
            continue;
        };
        let rest = &decl[start + "--plume-".len()..];
        let Some(colon) = rest.find(':') else {
            continue;
        };
        let name = rest[..colon].trim();
        let value = rest[colon + 1..].trim();
        // Trim any trailing `}` that landed in this segment (e.g.,
        // `gap: 10; }` after the split).
        let value = value.trim_end_matches('}').trim();
        if name.is_empty() || value.is_empty() {
            continue;
        }
        out.push((name.to_string(), value.to_string()));
    }
    out
}

/// Remove `/* … */` block comments. Themes are simple flat files; we don't
/// support nested comments.
fn strip_block_comments(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut in_comment = false;
    let bytes = src.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if !in_comment && i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            in_comment = true;
            i += 2;
            continue;
        }
        if in_comment && i + 1 < bytes.len() && bytes[i] == b'*' && bytes[i + 1] == b'/' {
            in_comment = false;
            i += 2;
            continue;
        }
        if !in_comment {
            out.push(bytes[i] as char);
        }
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_simple_var() {
        let css = ".plume { --plume-gap: 30; }";
        let vars = extract_plume_vars(css);
        assert_eq!(vars, vec![("gap".into(), "30".into())]);
    }

    #[test]
    fn extracts_multiple_lines() {
        let css = "\
            :root, .plume {\n\
              --plume-gap: 30;\n\
              --plume-accent: hotpink;\n\
              --plume-thickness: 2;\n\
            }\n\
        ";
        let vars = extract_plume_vars(css);
        assert_eq!(
            vars,
            vec![
                ("gap".into(), "30".into()),
                ("accent".into(), "hotpink".into()),
                ("thickness".into(), "2".into()),
            ]
        );
    }

    #[test]
    fn ignores_non_plume_vars() {
        let css = "--my-var: 5; --plume-gap: 10;";
        let vars = extract_plume_vars(css);
        assert_eq!(vars, vec![("gap".into(), "10".into())]);
    }

    #[test]
    fn handles_missing_semicolon() {
        let css = "--plume-gap: 30";
        let vars = extract_plume_vars(css);
        assert_eq!(vars, vec![("gap".into(), "30".into())]);
    }

    #[test]
    fn skips_inline_block_comments() {
        let css = "--plume-gap: 30; /* a comment */";
        let vars = extract_plume_vars(css);
        assert_eq!(vars, vec![("gap".into(), "30".into())]);
    }
}
