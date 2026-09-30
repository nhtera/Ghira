// SPDX-License-Identifier: Apache-2.0
//! Secret header values (API keys) that never show up in logs or errors.

use std::fmt;

use zeroize::Zeroizing;

/// A secret string, zeroed on drop. `Debug` and `Display` print `[redacted]`.
#[derive(Clone)]
pub struct Secret(Zeroizing<String>);

impl Secret {
    pub fn new(value: impl Into<String>) -> Secret {
        Secret(Zeroizing::new(value.into()))
    }

    /// The raw value. Only for handing to the HTTP layer.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[redacted]")
    }
}

impl fmt::Display for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[redacted]")
    }
}

/// Request headers. Every value is held as a [`Secret`], so the set can be
/// printed (names only) without leaking a key.
#[derive(Clone, Default)]
pub struct Headers(Vec<(String, Secret)>);

impl Headers {
    pub fn new() -> Headers {
        Headers::default()
    }

    /// A header whose value is not secret (content type, API version).
    pub fn plain(mut self, name: &str, value: &str) -> Headers {
        self.0.push((name.to_owned(), Secret::new(value)));
        self
    }

    /// A header carrying a key or token.
    pub fn secret(mut self, name: &str, value: Secret) -> Headers {
        self.0.push((name.to_owned(), value));
        self
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &Secret)> {
        self.0.iter().map(|(n, v)| (n.as_str(), v))
    }

    pub fn names(&self) -> Vec<&str> {
        self.0.iter().map(|(n, _)| n.as_str()).collect()
    }
}

impl fmt::Debug for Headers {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.names()).finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_never_print() {
        let s = Secret::new("test-key-not-real");
        assert_eq!(format!("{s:?} {s}"), "[redacted] [redacted]");
        let h = Headers::new()
            .plain("content-type", "application/json")
            .secret("authorization", Secret::new("Bearer test-key-not-real"));
        let shown = format!("{h:?}");
        assert!(!shown.contains("test-key-not-real") && !shown.contains("json"));
        assert!(shown.contains("authorization"));
        assert_eq!(s.expose(), "test-key-not-real");
    }
}
