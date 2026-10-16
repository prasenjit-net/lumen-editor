//! Interpreting the opening exchange: version negotiation and sign-in errors.
use crate::{requests::PROTOCOL_VERSION, rpc::RpcError, types::InitializeResult};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HandshakeError {
    /// The agent only speaks a protocol version we do not.
    UnsupportedVersion(u32),
}

impl std::fmt::Display for HandshakeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedVersion(version) => write!(
                formatter,
                "the agent speaks ACP v{version}; Lumen supports v{PROTOCOL_VERSION}"
            ),
        }
    }
}

/// The client sent its latest version; the agent answers with the same one or
/// with the latest it supports. Anything else means we must close the session.
pub fn check_version(result: &InitializeResult) -> Result<(), HandshakeError> {
    if result.protocol_version == PROTOCOL_VERSION {
        Ok(())
    } else {
        Err(HandshakeError::UnsupportedVersion(result.protocol_version))
    }
}

/// Both adapters report a missing login as JSON-RPC `-32000`
/// "Authentication required" - Claude on `session/prompt`, Codex on
/// `session/new` - rather than advertising it in `initialize`.
#[must_use]
pub fn is_auth_required(error: &RpcError) -> bool {
    error.code == -32000
        && error
            .message
            .to_lowercase()
            .contains("authentication required")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::parse_initialize;
    use serde_json::json;

    #[test]
    fn only_the_supported_protocol_version_proceeds() {
        let v1 = parse_initialize(&json!({"protocolVersion": 1})).unwrap();
        assert_eq!(check_version(&v1), Ok(()));
        let v2 = parse_initialize(&json!({"protocolVersion": 2})).unwrap();
        assert_eq!(
            check_version(&v2),
            Err(HandshakeError::UnsupportedVersion(2))
        );
        assert!(parse_initialize(&json!({})).is_err());
    }

    #[test]
    fn authentication_errors_are_recognised_but_other_errors_are_not() {
        let error = |code, message: &str| RpcError {
            code,
            message: message.into(),
            data: None,
        };
        assert!(is_auth_required(&error(-32000, "Authentication required")));
        assert!(is_auth_required(&error(
            -32000,
            "authentication required: please log in"
        )));
        assert!(!is_auth_required(&error(-32000, "Something else")));
        assert!(!is_auth_required(&error(-32603, "Authentication required")));
    }
}
