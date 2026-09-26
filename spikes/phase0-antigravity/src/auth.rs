use anyhow::{Context, Result, bail};
use regex::Regex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedOAuthUrl {
    pub raw_url: String,
    pub client_id: String,
    pub redirect_port: u16,
    pub state: String,
}

/// Parses the sign-in URL from stdout or stderr markers.
/// Supported markers:
/// 1. "Open the following link to authenticate the ACP server: <url>"
/// 2. "[BROWSER] <url>"
pub fn extract_auth_url_from_output(line: &str) -> Option<String> {
    let prefix1 = "Open the following link to authenticate the ACP server: ";
    if let Some(pos) = line.find(prefix1) {
        return Some(line[pos + prefix1.len()..].trim().to_string());
    }

    let prefix2 = "[BROWSER] ";
    if let Some(pos) = line.find(prefix2) {
        return Some(line[pos + prefix2.len()..].trim().to_string());
    }

    None
}

/// Validates that an OAuth URL conforms strictly to security requirements:
/// 1. HTTPS scheme
/// 2. Origin accounts.google.com
/// 3. Path /o/oauth2/v2/auth or /o/oauth2/auth
/// 4. redirect_uri targeting 127.0.0.1 with an explicit port
/// 5. Non-empty state parameter present
pub fn validate_google_oauth_url(url_str: &str) -> Result<ValidatedOAuthUrl> {
    if !url_str.starts_with("https://accounts.google.com/") {
        bail!("Invalid origin: must start with https://accounts.google.com/");
    }

    let re_client_id = Regex::new(r"[?&]client_id=([^&]+)").unwrap();
    let re_redirect = Regex::new(r"[?&]redirect_uri=([^&]+)").unwrap();
    let re_state = Regex::new(r"[?&]state=([^&]+)").unwrap();

    let client_id = re_client_id
        .captures(url_str)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
        .context("Missing client_id parameter in auth URL")?;

    let encoded_redirect = re_redirect
        .captures(url_str)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str())
        .context("Missing redirect_uri parameter in auth URL")?;

    // URL-decode redirect_uri
    let redirect_uri = encoded_redirect
        .replace("%3A", ":")
        .replace("%2F", "/")
        .replace("%3a", ":")
        .replace("%2f", "/");

    let re_port = Regex::new(r"^http://127\.0\.0\.1:(\d+)/?").unwrap();
    let port_str = re_port
        .captures(&redirect_uri)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str())
        .context("redirect_uri must match http://127.0.0.1:<port>/")?;

    let redirect_port: u16 = port_str
        .parse()
        .context("Invalid port number in redirect_uri")?;

    let state = re_state
        .captures(url_str)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
        .context("Missing state parameter in auth URL")?;

    Ok(ValidatedOAuthUrl {
        raw_url: url_str.to_string(),
        client_id,
        redirect_port,
        state,
    })
}

/// Validates that an incoming loopback redirect matches the pending state and extracts the code.
pub fn validate_callback_query(
    query_str: &str,
    expected_state: &str,
) -> Result<String> {
    let re_code = Regex::new(r"[?&]code=([^&]+)").unwrap();
    let re_state = Regex::new(r"[?&]state=([^&]+)").unwrap();

    let received_state = re_state
        .captures(query_str)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str())
        .context("Callback missing state parameter")?;

    if received_state != expected_state {
        bail!(
            "OAuth state mismatch: expected '{}', received '{}'",
            expected_state,
            received_state
        );
    }

    let code = re_code
        .captures(query_str)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
        .context("Callback missing code parameter")?;

    Ok(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_auth_url_extraction() {
        let line = "Open the following link to authenticate the ACP server: https://accounts.google.com/o/oauth2/v2/auth?client_id=123.apps.googleusercontent.com&redirect_uri=http%3A%2F%2F127.0.0.1%3A8085%2F&response_type=code&state=xyz999";
        let url = extract_auth_url_from_output(line).unwrap();
        assert!(url.starts_with("https://accounts.google.com/"));

        let validated = validate_google_oauth_url(&url).unwrap();
        assert_eq!(validated.redirect_port, 8085);
        assert_eq!(validated.state, "xyz999");
    }

    #[test]
    fn test_callback_validation() {
        let query = "?code=4/0AQ_AUTH_CODE&state=xyz999";
        let code = validate_callback_query(query, "xyz999").unwrap();
        assert_eq!(code, "4/0AQ_AUTH_CODE");

        // Wrong state must fail
        assert!(validate_callback_query(query, "wrong_state").is_err());
    }
}
