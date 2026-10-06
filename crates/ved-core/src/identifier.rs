//! Pure validation of specification-derived path components.
use crate::error::{Result, ValidationError, ValidationReason};

pub(crate) fn validate_identifier(kind: &str, name: &str) -> Result<()> {
    if !name
        .as_bytes()
        .first()
        .is_some_and(u8::is_ascii_alphanumeric)
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
    {
        return Err(ValidationError::new(
            ValidationReason::InvalidIdentifier,
            None,
            None,
            None,
            kind,
        )
        .with_detail("expected a single filesystem component")
        .into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_require_a_single_safe_ascii_component() {
        for name in ["web", "api-1", "Service_2", "0.cache"] {
            validate_identifier("workload", name).unwrap();
        }
        for name in ["", ".", "..", ".hidden", "-name", "a/b", "a\\b", "café"] {
            assert!(validate_identifier("workload", name).is_err(), "{name}");
        }
    }
}
