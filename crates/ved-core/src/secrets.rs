pub fn looks_like_secret(key: &str) -> bool {
    let upper = key.to_uppercase();
    upper.ends_with("_PASSWORD")
        || upper.ends_with("_SECRET")
        || upper.ends_with("_KEY")
        || upper.ends_with("_TOKEN")
        || upper.ends_with("_CREDENTIAL")
        || upper.ends_with("_PRIVATE_KEY")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_looks_like_secret() {
        assert!(looks_like_secret("POSTGRES_PASSWORD"));
        assert!(looks_like_secret("API_SECRET"));
        assert!(looks_like_secret("AUTH_TOKEN"));
        assert!(looks_like_secret("SSH_PRIVATE_KEY"));
        assert!(!looks_like_secret("POSTGRES_DB"));
        assert!(!looks_like_secret("POSTGRES_USER"));
        assert!(!looks_like_secret("MONKEY_JOCKEY"));
    }
}
