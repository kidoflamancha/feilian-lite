use keyring::{Entry, Error};

const SERVICE: &str = "dev.feilian.lite";
pub const WIREGUARD_PRIVATE_KEY: &str = "profile.wireguard-private-key";
pub const TOTP_SECRET: &str = "profile.totp-secret";
pub const ACCOUNT_PASSWORD: &str = "profile.account-password";
pub const SOCKS5_PASSWORD: &str = "profile.socks5-password";

pub trait SecretStore: Send + Sync {
    fn get(&self, name: &str) -> Result<Option<String>, String>;
    fn set(&self, name: &str, value: &str) -> Result<(), String>;
    fn delete(&self, name: &str) -> Result<(), String>;
}

pub struct SystemSecretStore;

impl SecretStore for SystemSecretStore {
    fn get(&self, name: &str) -> Result<Option<String>, String> {
        match entry(name)?.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(Error::NoEntry) => Ok(None),
            Err(error) => Err(error.to_string()),
        }
    }

    fn set(&self, name: &str, value: &str) -> Result<(), String> {
        entry(name)?
            .set_password(value)
            .map_err(|error| error.to_string())
    }

    fn delete(&self, name: &str) -> Result<(), String> {
        match entry(name)?.delete_credential() {
            Ok(()) | Err(Error::NoEntry) => Ok(()),
            Err(error) => Err(error.to_string()),
        }
    }
}

fn entry(name: &str) -> Result<Entry, String> {
    Entry::new(SERVICE, name).map_err(|error| error.to_string())
}

#[cfg(test)]
pub struct MemorySecretStore {
    values: std::sync::Mutex<std::collections::HashMap<String, String>>,
}

#[cfg(test)]
impl MemorySecretStore {
    pub fn new() -> Self {
        Self {
            values: std::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }
}

#[cfg(test)]
impl SecretStore for MemorySecretStore {
    fn get(&self, name: &str) -> Result<Option<String>, String> {
        Ok(self.values.lock().unwrap().get(name).cloned())
    }

    fn set(&self, name: &str, value: &str) -> Result<(), String> {
        self.values
            .lock()
            .unwrap()
            .insert(name.to_string(), value.to_string());
        Ok(())
    }

    fn delete(&self, name: &str) -> Result<(), String> {
        self.values.lock().unwrap().remove(name);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    #[test]
    #[ignore = "requires an unlocked desktop keyring"]
    fn system_secret_store_round_trip() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let name = format!("test.{unique}");
        let store = SystemSecretStore;

        store.set(&name, "temporary-secret").unwrap();
        assert_eq!(
            store.get(&name).unwrap().as_deref(),
            Some("temporary-secret")
        );
        store.delete(&name).unwrap();
        assert_eq!(store.get(&name).unwrap(), None);
    }
}
