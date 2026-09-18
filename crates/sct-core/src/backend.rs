//! Provider-specific settings terminate at the registry. M1 has test adapters only.
use crate::error::{Error, Result, require};
use sct_protocol::{AdapterDocumentation, BackendInspection, ErrorCode, ObservedHealth};
use serde_json::Value;
use std::{collections::BTreeMap, sync::Arc};

pub trait Adapter: Send + Sync {
    fn documentation(&self) -> AdapterDocumentation;
    /// Returns a stable destination identity, including provider namespace.
    fn validate(&self, settings: &Value) -> Result<String>;
    fn probe(&self, settings: &Value) -> ObservedHealth;
}
#[derive(Clone, Default)]
pub struct Registry(BTreeMap<String, Arc<dyn Adapter>>);
impl Registry {
    pub fn register(&mut self, adapter: Arc<dyn Adapter>) -> Result<()> {
        let kind = adapter.documentation().kind;
        require(!self.0.contains_key(&kind), ErrorCode::InvalidRequest)?;
        self.0.insert(kind, adapter);
        Ok(())
    }
    pub fn get(&self, kind: &str) -> Result<&dyn Adapter> {
        self.0
            .get(kind)
            .map(|a| a.as_ref())
            .ok_or(Error(ErrorCode::InvalidRequest))
    }
    pub fn validate(&self, inspection: &BackendInspection, settings: &Value) -> Result<String> {
        self.get(&inspection.adapter_kind)?.validate(settings)
    }
}
/// Explicitly opt-in development adapter, never a production payload store.
pub struct TestAdapter;
impl Adapter for TestAdapter {
    fn documentation(&self) -> AdapterDocumentation {
        AdapterDocumentation {
            kind: "test".into(),
            display_name: "Test metadata adapter".into(),
            acknowledgement: "No payload operations".into(),
            consistency: "No payload operations".into(),
            durability: "No content storage".into(),
            deletion_and_capacity: "No physical capacity reporting".into(),
            limitations: vec!["Foundation tests only; cannot upload or download".into()],
        }
    }
    fn validate(&self, settings: &Value) -> Result<String> {
        let destination = settings
            .get("destination")
            .and_then(Value::as_str)
            .ok_or(Error(ErrorCode::InvalidRequest))?;
        require(
            settings.get("offline").is_none_or(Value::is_boolean)
                && !destination.is_empty()
                && destination.len() <= 256
                && settings.as_object().is_some_and(|o| {
                    o.keys()
                        .all(|k| matches!(k.as_str(), "destination" | "offline"))
                }),
            ErrorCode::InvalidRequest,
        )?;
        Ok(format!("test:{destination}"))
    }
    fn probe(&self, settings: &Value) -> ObservedHealth {
        if settings.get("offline") == Some(&Value::Bool(true)) {
            ObservedHealth::Unavailable
        } else {
            ObservedHealth::Reachable
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Other;
    impl Adapter for Other {
        fn documentation(&self) -> AdapterDocumentation {
            let mut docs = TestAdapter.documentation();
            docs.kind = "another_provider".into();
            docs
        }
        fn validate(&self, settings: &Value) -> Result<String> {
            Ok(format!("other:{}", TestAdapter.validate(settings)?))
        }
        fn probe(&self, _: &Value) -> ObservedHealth {
            ObservedHealth::Unknown
        }
    }
    #[test]
    fn registry_extension_and_validation() {
        let mut registry = Registry::default();
        registry.register(Arc::new(TestAdapter)).unwrap();
        registry.register(Arc::new(Other)).unwrap();
        assert_ne!(
            registry
                .get("test")
                .unwrap()
                .validate(&serde_json::json!({"destination":"a"}))
                .unwrap(),
            registry
                .get("another_provider")
                .unwrap()
                .validate(&serde_json::json!({"destination":"a"}))
                .unwrap()
        );
        assert!(registry.get("unregistered").is_err());
        assert!(registry.register(Arc::new(TestAdapter)).is_err());
        assert!(
            TestAdapter
                .validate(&serde_json::json!({"destination":"a","offline":"wrong"}))
                .is_err()
        );
    }
}
