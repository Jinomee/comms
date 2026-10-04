//! Input validation utilities for message routing.
//!
//! - Scope and intent validation

use crate::shared::SenderIdentity;
use crate::shared::identity::SenderKind;

/// Valid scope values for message routing.
pub const VALID_SCOPES: &[&str] = &["broadcast", "mentions"];

/// Valid intent values for message envelope.
pub const VALID_INTENTS: &[&str] = &["ack", "inform", "request"];

/// Validate that scope is a valid value.
pub fn validate_scope(scope: &str) -> Result<(), String> {
    if VALID_SCOPES.contains(&scope) {
        Ok(())
    } else {
        Err(format!(
            "Invalid scope '{}'. Must be one of: {}",
            scope,
            VALID_SCOPES.join(", ")
        ))
    }
}

/// Validate that intent is a valid value.
pub fn validate_intent(intent: &str) -> Result<(), String> {
    if VALID_INTENTS.contains(&intent) {
        Ok(())
    } else {
        Err(format!(
            "Invalid intent '{}'. Must be one of: {}",
            intent,
            VALID_INTENTS.join(", ")
        ))
    }
}

/// Get bundle instance name from a SenderIdentity.
pub fn get_bundle_instance_name(identity: &SenderIdentity) -> String {
    match identity.kind {
        SenderKind::External => format!("ext_{}", identity.name),
        SenderKind::System => format!("sys_{}", identity.name),
        SenderKind::Instance => identity.name.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ===== Scope/Intent Validation =====

    #[test]
    fn test_validate_scope_valid() {
        assert!(validate_scope("broadcast").is_ok());
        assert!(validate_scope("mentions").is_ok());
    }

    #[test]
    fn test_validate_scope_invalid() {
        let err = validate_scope("unicast").unwrap_err();
        assert!(err.contains("Invalid scope 'unicast'"));
    }

    #[test]
    fn test_validate_intent_valid() {
        assert!(validate_intent("request").is_ok());
        assert!(validate_intent("inform").is_ok());
        assert!(validate_intent("ack").is_ok());
    }

    #[test]
    fn test_validate_intent_invalid() {
        let err = validate_intent("demand").unwrap_err();
        assert!(err.contains("Invalid intent 'demand'"));
    }

    // ===== get_bundle_instance_name =====

    #[test]
    fn test_bundle_instance_name_instance() {
        let id = SenderIdentity {
            kind: SenderKind::Instance,
            name: "luna".into(),
            instance_data: None,
            session_id: None,
        };
        assert_eq!(get_bundle_instance_name(&id), "luna");
    }

    #[test]
    fn test_bundle_instance_name_external() {
        let id = SenderIdentity {
            kind: SenderKind::External,
            name: "user".into(),
            instance_data: None,
            session_id: None,
        };
        assert_eq!(get_bundle_instance_name(&id), "ext_user");
    }

    #[test]
    fn test_bundle_instance_name_system() {
        let id = SenderIdentity {
            kind: SenderKind::System,
            name: "hcom".into(),
            instance_data: None,
            session_id: None,
        };
        assert_eq!(get_bundle_instance_name(&id), "sys_hcom");
    }
}
