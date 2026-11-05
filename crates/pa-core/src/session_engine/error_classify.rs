//! Classify a failed model call into a fixed category, subtype, code, and
//! HTTP status, with reviewed diagnostics kept separate from raw provider text.

/// The fixed provider transport disclosures pa-ai emits when the turn's
/// auto-retries exhaust (`ProviderConnectionError::message` and the
/// AWS h2 failure texts, `crates/pa-ai/src/utils_inner/stream_failure.rs`):
/// a recognized disclosure prefix is typed evidence, unlike a bare
/// keyword. Prefix-matched because some families append a cause
/// (`Request timed out: <cause>`) or the SDK's wrapper
/// (`Unexpected HTTP client error: TypeError: <raw>`).
const TRANSPORT_DISCLOSURES: &[(&str, &str)] = &[
    ("Connection error.", "network_error"),
    ("Request timed out", "timeout"),
    (
        "Unable to connect. Is the computer able to access the url?",
        "network_error",
    ),
    (
        "Unexpected HTTP client error: TypeError: Unable to connect",
        "network_error",
    ),
    ("Protocol error", "network_error"),
    ("Stream closed with error code", "network_error"),
    ("Session closed with error code", "network_error"),
    ("The pending stream has been canceled", "network_error"),
];

/// Reviewed fixed-string error messages that may be retained as safe messages.
const REVIEWED_MESSAGES: &[(&str, &str)] = &[
    ("cancelled", "Request was aborted"),
    ("cancelled", "The operation was aborted."),
    ("cancelled", "The operation was aborted"),
    ("network_error", "fetch failed"),
    ("provider_unavailable", "Provider overloaded"),
    ("rate_limited", "Provider rate limit exceeded"),
    (
        "malformed_response",
        "Provider returned a malformed response",
    ),
    ("refusal", "Model refused to respond"),
    ("refusal", "Response blocked by provider safety filters"),
    ("refusal", "Provider finish_reason: content_filter"),
    ("network_error", "Provider finish_reason: network_error"),
    ("provider_unavailable", "Provider server error"),
    ("authentication_rejected", "Provider authentication failed"),
    (
        "permission_denied",
        "Provider denied access to the requested resource",
    ),
    ("malformed_response", "Provider rejected the request"),
    ("network_error", "Provider stream failed"),
    ("filesystem_error", "Failed to acquire auth storage lock"),
    ("filesystem_error", "Auth storage lock was compromised"),
];

/// The safe error codes and their subtypes (#2117 `CODE_SUBTYPES`): a code
/// token classifies the error; the message text itself never uploads.
const CODE_SUBTYPES: &[(&str, &str)] = &[
    ("invalid_api_key", "credential_invalid"),
    ("invalid_token", "credential_invalid"),
    ("invalid_grant", "credential_invalid"),
    ("token_expired", "credential_expired"),
    ("expired_token", "credential_expired"),
    ("missing_api_key", "credential_missing"),
    ("authentication_error", "authentication_rejected"),
    ("unauthorized", "authentication_rejected"),
    ("permission_error", "permission_denied"),
    ("permission_denied", "permission_denied"),
    ("access_denied", "permission_denied"),
    ("forbidden", "permission_denied"),
    ("model_not_found", "model_access_denied"),
    ("model_access_denied", "model_access_denied"),
    ("usage_not_included", "model_access_denied"),
    ("insufficient_funds", "insufficient_balance"),
    ("insufficient_balance", "insufficient_balance"),
    ("insufficient_quota", "quota_exceeded"),
    ("quota_exceeded", "quota_exceeded"),
    ("resource_exhausted", "quota_exceeded"),
    ("rate_limit_error", "rate_limited"),
    ("rate_limit_exceeded", "rate_limited"),
    ("too_many_requests", "rate_limited"),
    ("overloaded_error", "provider_unavailable"),
    ("server_error", "provider_unavailable"),
    ("api_error", "provider_unavailable"),
    ("service_unavailable", "provider_unavailable"),
    ("refusal", "refusal"),
    ("content_filter", "refusal"),
    ("safety", "refusal"),
    ("malformed_response", "malformed_response"),
    ("stream_drop", "stream_drop"),
    ("context_length_exceeded", "context_limit"),
    ("context_window_exceeded", "context_limit"),
    ("ECONNRESET", "network_error"),
    ("ECONNREFUSED", "network_error"),
    ("EHOSTUNREACH", "network_error"),
    ("ENETUNREACH", "network_error"),
    ("ENOTFOUND", "network_error"),
    ("EAI_AGAIN", "network_error"),
    ("EPIPE", "network_error"),
    ("ETIMEDOUT", "timeout"),
    ("UND_ERR_CONNECT_TIMEOUT", "timeout"),
    ("UND_ERR_HEADERS_TIMEOUT", "timeout"),
    ("UND_ERR_BODY_TIMEOUT", "timeout"),
    ("UND_ERR_SOCKET", "network_error"),
    ("ENOENT", "filesystem_error"),
    ("EACCES", "filesystem_error"),
    ("EPERM", "filesystem_error"),
    ("ENOSPC", "filesystem_error"),
    ("EMFILE", "filesystem_error"),
    ("EROFS", "filesystem_error"),
    ("ELOCKED", "filesystem_error"),
];

/// The subtype's legacy category: structured evidence names the
/// category, never the raw message wording.
fn subtype_category(subtype: &str) -> &'static str {
    match subtype {
        "credential_missing"
        | "credential_invalid"
        | "credential_expired"
        | "authentication_rejected" => "authentication",
        "quota_exceeded" | "rate_limited" => "rate_limit",
        "network_error" | "stream_drop" => "network",
        "timeout" => "timeout",
        "provider_unavailable" => "provider_unavailable",
        "context_limit" => "context_limit",
        _ => "other",
    }
}

/// One failed call's classification. No raw message text is carried.
#[derive(Debug, Clone, PartialEq)]
pub struct ErrorClassification {
    /// `category` is the legacy `error_category` vocabulary.
    pub category: &'static str,
    pub subtype: &'static str,
    /// A bounded HTTP status observed in the text.
    pub http_status: Option<u64>,
}

impl ErrorClassification {
    /// The nano-rlm #200 ACP-boundary view: only the exhausted model
    /// connection family is `model_transport` (the one class a host may
    /// re-prompt on). HTTP status errors keep their provider categories:
    /// a host retry is a fresh prompt, not an idempotent replay.
    #[must_use]
    pub fn is_model_transport(&self) -> bool {
        // An HTTP-status classification (a 408 reads as `timeout`) is a
        // provider answer over a live connection, not a model-connection
        // failure: the status verdicts keep their provider categories.
        matches!(self.subtype, "network_error" | "stream_drop" | "timeout")
            && self.http_status.is_none()
    }
}

/// Classify one failed call's error message without exposing raw provider text.
#[must_use]
pub fn classify_error_message(message: &str) -> ErrorClassification {
    // 1. A bounded HTTP status is the strongest structured evidence.
    if let Some(status) = bounded_http_status(message) {
        let subtype = match status {
            401 | 403 => "authentication_rejected",
            402 => "insufficient_balance",
            404 => "model_access_denied",
            408 => "timeout",
            413 => "context_limit",
            429 => "rate_limited",
            500..=599 => "provider_unavailable",
            _ => "unknown",
        };
        return ErrorClassification {
            category: subtype_category(subtype),
            subtype,
            http_status: Some(status),
        };
    }
    // 2. A recognized safe error code token names the subtype. The match
    // runs case-insensitively in both directions; the code keeps its
    // canonical spelling.
    let lowered = message.to_ascii_lowercase();
    if let Some((_, subtype)) = CODE_SUBTYPES.iter().find(|(code, _)| {
        let lowered_code = code.to_ascii_lowercase();
        lowered.contains(&lowered_code) || message.contains(*code)
    }) {
        return ErrorClassification {
            category: subtype_category(subtype),
            subtype,
            http_status: None,
        };
    }
    // 3. A fixed provider transport disclosure (prefix match): the
    // exhausted-connection family pa-ai itself discloses, so the most
    // common real connection failures classify instead of reading as
    // `unknown`.
    if let Some((_, subtype)) = TRANSPORT_DISCLOSURES
        .iter()
        .find(|(disclosure, _)| message.starts_with(disclosure))
    {
        return ErrorClassification {
            category: subtype_category(subtype),
            subtype,
            http_status: None,
        };
    }
    // 4. A reviewed fixed-string message (exact match).
    if let Some((subtype, _)) = REVIEWED_MESSAGES
        .iter()
        .find(|(_, reviewed)| reviewed == &message)
    {
        return ErrorClassification {
            category: subtype_category(subtype),
            subtype,
            http_status: None,
        };
    }
    // 5. An unmatched message receives the generic category.
    ErrorClassification {
        category: "other",
        subtype: "unknown",
        http_status: None,
    }
}

/// A digit-bounded `[45]dd` HTTP status in the text.
fn bounded_http_status(message: &str) -> Option<u64> {
    let bytes = message.as_bytes();
    for index in 0..bytes.len() {
        if !matches!(bytes[index], b'4' | b'5') || index + 2 >= bytes.len() {
            continue;
        }
        let (b, c) = (bytes[index + 1], bytes[index + 2]);
        if !b.is_ascii_digit() || !c.is_ascii_digit() {
            continue;
        }
        let digit_before = index > 0 && bytes[index - 1].is_ascii_digit();
        let digit_after = index + 3 < bytes.len() && bytes[index + 3].is_ascii_digit();
        // A word boundary, not just a digit boundary: `500ms` / `429s`
        // (durations in the message text) never read as statuses.
        let unit_after = index + 3 < bytes.len() && bytes[index + 3].is_ascii_alphabetic();
        if !digit_before && !digit_after && !unit_after {
            let status = u64::from(bytes[index] - b'0') * 100
                + u64::from(b - b'0') * 10
                + u64::from(c - b'0');
            return Some(status);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn classify(message: &str) -> ErrorClassification {
        classify_error_message(message)
    }

    #[test]
    fn http_status_is_the_strongest_evidence() {
        let classification = classify("API Error: 429 rate limit exceeded");
        assert_eq!(classification.subtype, "rate_limited");
        assert_eq!(classification.http_status, Some(429));
        assert_eq!(classification.category, "rate_limit");
    }

    #[test]
    fn status_digits_are_bounded() {
        assert_eq!(bounded_http_status("error 503 unavailable"), Some(503));
        assert_eq!(bounded_http_status("error 4012 nope"), None);
        assert_eq!(bounded_http_status("14043"), None);
        assert_eq!(bounded_http_status("no status at all"), None);
        assert_eq!(bounded_http_status("timed out after 500ms"), None);
        assert_eq!(bounded_http_status("retry in 429s"), None);
        assert_eq!(bounded_http_status("wait 30m"), None);
    }

    #[test]
    fn safe_codes_classify_without_carrying_raw_text() {
        let classification = classify("ECONNRESET while streaming: user-secret-token");
        assert_eq!(classification.subtype, "network_error");
        assert_eq!(classification.category, "network");
    }

    #[test]
    fn errno_tokens_match_lowercase_messages() {
        let classification = classify("request failed: econnreset during stream");
        assert_eq!(
            classification.subtype, "network_error",
            "the lowercase errno still names the typed subtype"
        );
    }

    #[test]
    fn reviewed_literals_match_exactly() {
        let classification = classify("Provider rate limit exceeded");
        assert_eq!(classification.subtype, "rate_limited");
        // The abort literal's subtype is the catalog vocabulary
        // (`cancelled`, never a message id that sanitize would rewrite).
        let aborted = classify("Request was aborted");
        assert_eq!(aborted.subtype, "cancelled");
        let near_miss = classify("Provider rate limit exceeded for model glm-4.6");
        assert_eq!(near_miss.subtype, "unknown");
    }

    #[test]
    fn stream_drop_messages_classify_as_the_stream_drop_category() {
        // The provider's drop disclosure (pa-ai `stream_drop_failure`):
        // every block detail names the same class.
        let classification = classify(
            "Provider dropped the response stream (stream_drop): the stream ended inside a thinking block before the stop signal"
        );
        assert_eq!(classification.subtype, "stream_drop");
        assert_eq!(classification.category, "network");
        // The raw message never uploads; the fixed diagnostic rides.
        // The empty-stream variant classifies identically.
        let empty = classify(
            "Provider dropped the response stream (stream_drop): the stream ended before any response content or stop signal"
        );
        assert_eq!(empty.subtype, "stream_drop");
    }

    #[test]
    fn unmatched_messages_get_the_unknown_category() {
        let classification = classify("Provider exploded with secret details about /home/user");
        assert_eq!(classification.subtype, "unknown");
        assert_eq!(classification.category, "other");
    }

    #[test]
    fn auth_and_billing_split() {
        let auth = classify("invalid_api_key");
        assert_eq!(auth.subtype, "credential_invalid");
        assert_eq!(auth.category, "authentication");
        let billing = classify("402");
        assert_eq!(billing.subtype, "insufficient_balance");
        assert_eq!(billing.category, "other");
        assert_eq!(billing.http_status, Some(402));
    }

    #[test]
    fn transport_disclosures_classify_the_exhausted_connection_family() {
        // The fixed pa-ai disclosures (`ProviderConnectionError::message`):
        // the most common real connection failures, all typed transport.
        let cases = [
            ("Connection error.", "network_error"),
            ("Request timed out.", "timeout"),
            ("Request timed out: fetch timed out", "timeout"),
            ("Unable to connect. Is the computer able to access the url?", "network_error"),
            (
                "Unexpected HTTP client error: TypeError: Unable to connect. Is the computer able to access the url?",
                "network_error",
            ),
            ("Protocol error", "network_error"),
            ("Stream closed with error code 0x2", "network_error"),
            ("The pending stream has been canceled", "network_error"),
        ];
        for (message, subtype) in cases {
            let classification = classify(message);
            assert_eq!(classification.subtype, subtype, "{message}");
            assert!(classification.is_model_transport(), "{message}");
        }
        // A disclosure embedded mid-message is not evidence: only the
        // provider layer's own prefix form is a typed disclosure.
        let embedded = classify("tool output: Request timed out mid-run");
        assert_eq!(embedded.subtype, "unknown");
        assert!(!embedded.is_model_transport());
    }

    #[test]
    fn model_transport_covers_only_the_connection_family() {
        for message in ["ECONNRESET mid-stream", "Provider dropped the response stream (stream_drop): the stream ended inside a thinking block before the stop signal"] {
            assert!(classify(message).is_model_transport(), "{message}");
        }
        // HTTP status errors keep their provider categories (#200: they
        // retain the existing behavior, so none is boundary-retryable) —
        // including the 408, whose `timeout` subtype is a provider
        // rejection over a live connection, not an exhausted connect.
        for message in [
            "API Error: 429 rate limit exceeded",
            "401 unauthorized",
            "402 insufficient credits",
            "500 Internal Server Error",
            "408 Request Timeout",
            "context_length_exceeded",
        ] {
            let classification = classify(message);
            assert!(!classification.is_model_transport(), "{message}");
        }
        // The SDK timeout disclosure (no status) is the transport family.
        assert!(classify("Request timed out.").is_model_transport());
    }
}
