use std::io::{self, BufRead, Write};

pub(crate) fn confirm<R: BufRead, W: Write>(
    prompt: &str,
    input: &mut R,
    out: &mut W,
) -> io::Result<bool> {
    write!(out, "{prompt}")?;
    out.flush()?;
    let mut line = String::new();
    input.read_line(&mut line)?;
    let trimmed = line.trim().to_lowercase();
    Ok(trimmed == "y" || trimmed == "yes")
}

pub(crate) fn confirm_interactive(prompt: &str) -> io::Result<bool> {
    let stdin = io::stdin();
    let mut reader = stdin.lock();
    let mut writer = io::stdout();
    confirm(prompt, &mut reader, &mut writer)
}

pub(crate) fn section(title: &str, trail: usize) -> String {
    format!("── {} {}", title, "─".repeat(trail))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_confirm_accepts_yes_variants() {
        let inputs = vec!["y\n", "Y\n", "yes\n", "YES\n", " yes \n"];
        for input in inputs {
            let mut reader = input.as_bytes();
            let mut writer = Vec::new();
            assert!(confirm("Prompt", &mut reader, &mut writer).unwrap());
        }
    }

    #[test]
    fn test_confirm_rejects_other_input() {
        let inputs = vec!["n\n", "\n", "", "maybe\n"];
        for input in inputs {
            let mut reader = input.as_bytes();
            let mut writer = Vec::new();
            assert!(!confirm("Prompt", &mut reader, &mut writer).unwrap());
        }
    }

    #[test]
    fn test_confirm_flushes_prompt_before_read() {
        let mut reader = "y\n".as_bytes();
        let mut writer = Vec::new();
        confirm("Are you sure?", &mut reader, &mut writer).unwrap();
        assert_eq!(String::from_utf8_lossy(&writer), "Are you sure?");
    }

    #[test]
    fn test_section_matches_legacy_headers() {
        assert_eq!(
            section("Kubernetes pods (default)", 26),
            "── Kubernetes pods (default) ──────────────────────────"
        );
        assert_eq!(
            section("Service URLs", 41),
            "── Service URLs ─────────────────────────────────────────"
        );
        assert_eq!(
            section("Docker Compose status", 32),
            "── Docker Compose status ────────────────────────────────"
        );
        assert_eq!(
            section("Detected Modules (2)", 31),
            "── Detected Modules (2) ───────────────────────────────"
        );
    }
}
