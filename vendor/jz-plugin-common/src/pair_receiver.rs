//! Local receiver ownership, scoped to the native host session. An interrupted
//! command can outlive its host turn; process existence is not ownership.
use crate::{home::Home, pair};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fs, io, path::PathBuf};

#[derive(Clone, Serialize, Deserialize)]
pub struct Receiver {
    pub session: String,
    pub product: String,
    pub generation: String,
    active: bool,
}

pub struct Lease {
    path: PathBuf,
    pub receiver: Receiver,
}

fn path(plugin: &str, host: &str) -> io::Result<PathBuf> {
    if !matches!(plugin, "anychat" | "plugin-admin") || host.is_empty() || host.len() > 256 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid receiver owner",
        ));
    }
    let home = Home::resolve()?;
    crate::home::guard_credential_paths(&home)?;
    let dir = home.ensure_plugin_runtime(plugin)?.join("pair-receiver");
    crate::home::reject_symlink(&dir)?;
    fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{:x}.json", Sha256::digest(host.as_bytes())));
    crate::home::reject_symlink(&path)?;
    Ok(path)
}

fn lock(path: &std::path::Path) -> io::Result<fs::File> {
    let lock_path = path.with_extension("lock");
    crate::home::reject_symlink(&lock_path)?;
    let file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(lock_path)?;
    file.lock()?;
    Ok(file)
}

pub fn retire_current(plugin: &str) -> io::Result<()> {
    if let Ok(host) =
        std::env::var("CODEX_THREAD_ID").or_else(|_| std::env::var("CODEX_SESSION_ID"))
    {
        revoke(plugin, &host)?;
    }
    Ok(())
}

pub fn begin_current(plugin: &str, product: &str, session: &str) -> io::Result<Lease> {
    let host = std::env::var("CODEX_THREAD_ID")
        .or_else(|_| std::env::var("CODEX_SESSION_ID"))
        .unwrap_or_else(|_| format!("standalone-{}", std::process::id()));
    begin(plugin, &host, product, session)
}

pub fn begin(plugin: &str, host: &str, product: &str, session: &str) -> io::Result<Lease> {
    pair::validate_identifier(session)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid session"))?;
    if !matches!(
        product,
        "anychat" | "anyknow" | "anypdf" | "anydoc" | "anyweb" | "easybooks" | "formbro"
    ) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid product",
        ));
    }
    let path = path(plugin, host)?;
    let _lock = lock(&path)?;
    let receiver = Receiver {
        session: session.into(),
        product: product.into(),
        generation: uuid::Uuid::new_v4().simple().to_string(),
        active: true,
    };
    crate::home::write_private_file(&path, &serde_json::to_vec(&receiver)?)?;
    Ok(Lease { path, receiver })
}

impl Lease {
    pub fn present(
        &self,
        plugin: &str,
        phase: &str,
        expires_at: i64,
        output: serde_json::Value,
    ) -> io::Result<bool> {
        let _lock = lock(&self.path)?;
        if !self.is_current()? {
            return Ok(false);
        }
        let value = crate::pair_host::decorate_generation(
            plugin,
            &self.receiver.product,
            &self.receiver.session,
            phase,
            expires_at,
            output,
            Some(&self.receiver.generation),
        )?;
        println!("{value}");
        Ok(true)
    }

    pub fn is_current(&self) -> io::Result<bool> {
        crate::home::reject_symlink(&self.path)?;
        let current: Receiver = serde_json::from_slice(&fs::read(&self.path)?)?;
        Ok(current.active
            && current.generation == self.receiver.generation
            && current.session == self.receiver.session
            && current.product == self.receiver.product)
    }
}

pub fn revoke(plugin: &str, host: &str) -> io::Result<Option<Receiver>> {
    let path = path(plugin, host)?;
    let _lock = lock(&path)?;
    let mut current: Receiver = match fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes)?,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    if !current.active {
        return Ok(None);
    }
    current.active = false;
    crate::home::write_private_file(&path, &serde_json::to_vec(&current)?)?;
    Ok(Some(current))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn receiver_accepts_every_registered_product() {
        let _env = crate::test_env::lock();
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("JACKYZHANG_APP_HOME", dir.path());
        let session = "a".repeat(32);
        for product in [
            "anychat",
            "anyknow",
            "anypdf",
            "anydoc",
            "anyweb",
            "easybooks",
            "formbro",
        ] {
            assert!(
                begin("anychat", "host-one", product, &session).is_ok(),
                "receiver allowlist rejects {product}"
            );
        }
        std::env::remove_var("JACKYZHANG_APP_HOME");
    }

    #[test]
    fn interrupted_receiver_stays_revoked_after_explicit_replacement() {
        let _env = crate::test_env::lock();
        let dir = tempfile::tempdir().unwrap();
        let previous = std::env::var_os("JACKYZHANG_APP_HOME");
        std::env::set_var("JACKYZHANG_APP_HOME", dir.path());
        let session = "a".repeat(32);
        let first = begin("anychat", "host-one", "anychat", &session).unwrap();
        let other = begin("anychat", "host-two", "anychat", &session).unwrap();
        assert!(first.is_current().unwrap());
        crate::pair_host::handle(
            "anychat",
            &serde_json::json!({"hook_event_name":"Interrupt", "session_id":"host-one"}),
        )
        .unwrap();
        assert!(!first.is_current().unwrap());
        assert!(other.is_current().unwrap());
        let resumed = begin("anychat", "host-one", "anychat", &session).unwrap();
        assert!(resumed.is_current().unwrap());
        assert!(!first.is_current().unwrap());
        let replacement = begin("anychat", "host-one", "anychat", &session).unwrap();
        assert!(!resumed.is_current().unwrap());
        assert!(replacement.is_current().unwrap());
        match previous {
            Some(v) => std::env::set_var("JACKYZHANG_APP_HOME", v),
            None => std::env::remove_var("JACKYZHANG_APP_HOME"),
        }
    }
}
