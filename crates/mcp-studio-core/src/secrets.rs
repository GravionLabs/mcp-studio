//! Secrets live in the OS keyring. The database only ever stores `keyring:<name>` references.

use std::{
    collections::{BTreeMap, HashMap},
    sync::Mutex,
};

use crate::{
    db::{DbError, DbResult},
    registry::ServerInput,
};

/// Prefix that marks a stored value as a keyring reference.
pub const REFERENCE_PREFIX: &str = "keyring:";

/// What replaces secret values in logs, recorded messages, and error text.
pub const MASK: &str = "••••••••";

/// Where secret values are kept.
pub trait SecretStore: Send + Sync {
    fn get(&self, name: &str) -> DbResult<Option<String>>;
    fn set(&self, name: &str, value: &str) -> DbResult<()>;
    fn delete(&self, name: &str) -> DbResult<()>;
}

/// The OS keyring (Keychain, Credential Manager, Secret Service).
pub struct KeyringStore {
    service: String,
}

impl KeyringStore {
    pub fn new(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
        }
    }

    fn entry(&self, name: &str) -> DbResult<keyring::Entry> {
        keyring::Entry::new(&self.service, name).map_err(secret_error)
    }
}

fn secret_error(error: keyring::Error) -> DbError {
    DbError::Invalid(format!("keyring: {error}"))
}

impl SecretStore for KeyringStore {
    fn get(&self, name: &str) -> DbResult<Option<String>> {
        match self.entry(name)?.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(secret_error(error)),
        }
    }

    fn set(&self, name: &str, value: &str) -> DbResult<()> {
        self.entry(name)?.set_password(value).map_err(secret_error)
    }

    fn delete(&self, name: &str) -> DbResult<()> {
        match self.entry(name)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(secret_error(error)),
        }
    }
}

/// In-memory store for tests and for machines without a keyring.
#[derive(Default)]
pub struct MemoryStore {
    values: Mutex<HashMap<String, String>>,
}

impl SecretStore for MemoryStore {
    fn get(&self, name: &str) -> DbResult<Option<String>> {
        Ok(self.values.lock().unwrap().get(name).cloned())
    }

    fn set(&self, name: &str, value: &str) -> DbResult<()> {
        self.values
            .lock()
            .unwrap()
            .insert(name.to_owned(), value.to_owned());
        Ok(())
    }

    fn delete(&self, name: &str) -> DbResult<()> {
        self.values.lock().unwrap().remove(name);
        Ok(())
    }
}

/// The secret name inside a reference, if `value` is one.
pub fn reference_name(value: &str) -> Option<&str> {
    value
        .strip_prefix(REFERENCE_PREFIX)
        .filter(|name| !name.is_empty())
}

/// Builds the reference for a secret name.
pub fn reference(name: &str) -> String {
    format!("{REFERENCE_PREFIX}{name}")
}

/// Collects every secret name referenced by a server definition.
pub fn references_in(input: &ServerInput) -> Vec<String> {
    input
        .env
        .values()
        .chain(input.headers.values())
        .filter_map(|v| reference_name(v).map(str::to_owned))
        .collect()
}

/// Environment and headers with references replaced by their secret values.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Resolved {
    pub env: BTreeMap<String, String>,
    pub headers: BTreeMap<String, String>,
    redactor: Redactor,
}

impl Resolved {
    /// Masks every resolved secret value in `text`.
    pub fn redact(&self, text: &str) -> String {
        self.redactor.redact(text)
    }

    pub fn redactor(&self) -> &Redactor {
        &self.redactor
    }
}

/// Replaces known secret values in arbitrary text.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Redactor {
    secrets: Vec<String>,
}

impl Redactor {
    pub fn new(secrets: impl IntoIterator<Item = String>) -> Self {
        let mut secrets: Vec<String> = secrets.into_iter().filter(|s| !s.is_empty()).collect();
        // Longest first so a secret containing another one is masked completely.
        secrets.sort_by_key(|s| std::cmp::Reverse(s.len()));
        secrets.dedup();
        Self { secrets }
    }

    pub fn redact(&self, text: &str) -> String {
        self.secrets
            .iter()
            .fold(text.to_owned(), |acc, secret| acc.replace(secret, MASK))
    }
}

fn resolve_value(
    store: &dyn SecretStore,
    value: &str,
    secrets: &mut Vec<String>,
) -> DbResult<String> {
    let Some(name) = reference_name(value) else {
        return Ok(value.to_owned());
    };
    let secret = store.get(name)?.ok_or_else(|| {
        DbError::NotFound(format!("secret \"{name}\" is missing from the keyring"))
    })?;
    secrets.push(secret.clone());
    Ok(secret)
}

/// Resolves `keyring:` references in a name/value map. Returns the plain map and the secret values
/// that were substituted (for redaction).
pub fn resolve_values(
    store: &dyn SecretStore,
    values: &BTreeMap<String, String>,
) -> DbResult<(BTreeMap<String, String>, Vec<String>)> {
    let mut secrets = Vec::new();
    let mut resolved = BTreeMap::new();
    for (key, value) in values {
        resolved.insert(key.clone(), resolve_value(store, value, &mut secrets)?);
    }
    Ok((resolved, secrets))
}

/// Resolves all `keyring:` references in a server's environment and headers.
pub fn resolve(store: &dyn SecretStore, input: &ServerInput) -> DbResult<Resolved> {
    let mut secrets = Vec::new();
    let mut map = |source: &BTreeMap<String, String>| -> DbResult<BTreeMap<String, String>> {
        source
            .iter()
            .map(|(k, v)| Ok((k.clone(), resolve_value(store, v, &mut secrets)?)))
            .collect()
    };
    let env = map(&input.env)?;
    let headers = map(&input.headers)?;
    Ok(Resolved {
        env,
        headers,
        redactor: Redactor::new(secrets),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::TransportKind;

    fn input() -> ServerInput {
        ServerInput {
            name: "s".into(),
            transport: TransportKind::Stdio,
            command: Some("x".into()),
            args: vec![],
            env: BTreeMap::from([
                ("API_KEY".into(), "keyring:abc".into()),
                ("PLAIN".into(), "visible".into()),
            ]),
            cwd: None,
            url: None,
            headers: BTreeMap::from([("Authorization".into(), "keyring:def".into())]),
            tags: vec![],
        }
    }

    fn store() -> MemoryStore {
        let store = MemoryStore::default();
        store.set("abc", "sk-secret-1").unwrap();
        store.set("def", "Bearer sk-secret-1-long").unwrap();
        store
    }

    #[test]
    fn memory_store_roundtrip() {
        let store = MemoryStore::default();
        assert_eq!(store.get("a").unwrap(), None);
        store.set("a", "1").unwrap();
        assert_eq!(store.get("a").unwrap().as_deref(), Some("1"));
        store.delete("a").unwrap();
        store.delete("a").unwrap();
        assert_eq!(store.get("a").unwrap(), None);
    }

    #[test]
    fn parses_references() {
        assert_eq!(reference_name("keyring:abc"), Some("abc"));
        assert_eq!(reference_name("keyring:"), None);
        assert_eq!(reference_name("plain"), None);
        assert_eq!(reference("x"), "keyring:x");
    }

    #[test]
    fn collects_references_from_env_and_headers() {
        let mut names = references_in(&input());
        names.sort();
        assert_eq!(names, ["abc", "def"]);
    }

    #[test]
    fn resolves_references_and_keeps_plain_values() {
        let resolved = resolve(&store(), &input()).unwrap();
        assert_eq!(resolved.env["API_KEY"], "sk-secret-1");
        assert_eq!(resolved.env["PLAIN"], "visible");
        assert_eq!(resolved.headers["Authorization"], "Bearer sk-secret-1-long");
    }

    #[test]
    fn missing_secret_is_an_error_naming_the_secret() {
        let error = resolve(&MemoryStore::default(), &input()).unwrap_err();
        assert!(error.to_string().contains("abc"), "{error}");
    }

    #[test]
    fn redacts_all_resolved_secrets_longest_first() {
        let resolved = resolve(&store(), &input()).unwrap();
        let text = "sent Bearer sk-secret-1-long and key sk-secret-1; visible stays";
        assert_eq!(
            resolved.redact(text),
            format!("sent {MASK} and key {MASK}; visible stays")
        );
    }

    #[test]
    fn redactor_ignores_empty_secrets() {
        assert_eq!(Redactor::new(["".to_owned()]).redact("abc"), "abc");
    }
}
