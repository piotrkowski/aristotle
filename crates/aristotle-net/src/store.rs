// SPDX-License-Identifier: AGPL-3.0-or-later
//! The network settings on disk, and reading them from files the reader
//! writes by hand.
//!
//! Everything lives in one file, `accounts.json`, owner-only (mode 0600) in
//! a folder only the app's user owns and may write ([`private_dir`]): on
//! the device `/var/local/aristotle`, root's, on the ext3 partition, never
//! the FAT userstore, which has no permissions and is readable by anyone
//! with a USB cable. The sync password is not stored, only its MD5 key
//! (which still works as a password for that one server). Catalog
//! passwords are stored as given, because Basic and Digest need them.
//!
//! Until the on-screen keyboard lands, settings come from two files next to
//! the app, `sync.json` and `catalogs.json` (formats in `docs/library.md`).
//! They are read when they change, and their passwords are removed from
//! them once stored.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{self, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::auth::{md5_hex, Credentials};
use crate::kosync::{self, Account};
use crate::policy::entry_for;
use crate::url::Url;

pub const FILE: &str = "accounts.json";
/// Allowlist tags (`# aristotle:<feature>`).
pub const SYNC_FEATURE: &str = "sync";
pub const OPDS_FEATURE: &str = "opds";

fn yes() -> bool {
    true
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncSettings {
    pub server: String,
    pub username: String,
    /// `md5(password)`; empty until a password was given.
    #[serde(default)]
    pub key: String,
    /// Pull on open, push on close.
    #[serde(default = "yes")]
    pub auto: bool,
}

impl std::fmt::Debug for SyncSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SyncSettings")
            .field("server", &self.server)
            .field("username", &self.username)
            .field("auto", &self.auto)
            .finish_non_exhaustive()
    }
}

impl SyncSettings {
    /// The account, once server, user and key are all there.
    pub fn account(&self) -> Option<Account> {
        (!self.server.is_empty() && !self.username.is_empty() && !self.key.is_empty()).then(|| {
            Account {
                server: self.server.clone(),
                username: self.username.clone(),
                key: self.key.clone(),
            }
        })
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogSettings {
    pub name: String,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
}

impl std::fmt::Debug for CatalogSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CatalogSettings")
            .field("name", &self.name)
            .field("url", &self.url)
            .field("username", &self.username)
            .finish_non_exhaustive()
    }
}

impl CatalogSettings {
    pub fn credentials(&self) -> Option<Credentials> {
        Some(Credentials {
            username: self.username.clone().filter(|u| !u.is_empty())?,
            password: self.password.clone().unwrap_or_default(),
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Accounts {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sync: Option<SyncSettings>,
    #[serde(default)]
    pub catalogs: Vec<CatalogSettings>,
    /// This reader's KOSync device ID, made once.
    #[serde(default)]
    pub device_id: String,
    /// MD5 of each import file as last read, so a file is read once per
    /// change.
    #[serde(default)]
    pub imported: BTreeMap<String, String>,
}

/// What reading an import file did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Imported {
    /// There is no such file.
    Missing,
    /// Read before and unchanged since.
    Unchanged,
    Loaded,
}

#[derive(Debug)]
pub enum ImportError {
    Io(io::Error),
    Json(String),
    /// A field the settings cannot use, such as a bad server address.
    Invalid(String),
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImportError::Io(e) => e.fmt(f),
            ImportError::Json(e) => write!(f, "not valid JSON: {e}"),
            ImportError::Invalid(e) => f.write_str(e),
        }
    }
}

impl std::error::Error for ImportError {}

impl From<io::Error> for ImportError {
    fn from(e: io::Error) -> Self {
        ImportError::Io(e)
    }
}

#[derive(Deserialize, Serialize)]
struct SyncFile {
    server: String,
    username: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    password: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    auto: Option<bool>,
}

#[derive(Deserialize, Serialize)]
struct CatalogsFile {
    catalogs: Vec<CatalogSettings>,
}

/// Make sure `dir` can hold secrets: create it if missing, and refuse it
/// unless it is a real directory (not a symlink) owned by this process's
/// user with no permission bits for group or others.
///
/// On the device the app runs as root, and the folder is made mode 000 as
/// the agent's install makes `/var/local/aristotle`: the stock framework
/// runs `chgrp -R` and `chmod -R g=u` over `/var/local` on every start
/// (`device/README.md`, "Security"), so the group would get whatever the
/// owner has, and root needs no bits. Elsewhere (tests, a developer's
/// folder) it is made 0700.
pub fn private_dir(dir: &Path) -> io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt};
    // SAFETY: geteuid has no preconditions and cannot fail.
    let me = unsafe { libc::geteuid() };
    if !dir.exists() {
        let mode = if me == 0 { 0o000 } else { 0o700 };
        fs::DirBuilder::new().recursive(true).mode(mode).create(dir)?;
    }
    let m = fs::symlink_metadata(dir)?;
    let bad = |why: &str| {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("{}: {why}; not storing passwords there", dir.display()),
        ))
    };
    if !m.file_type().is_dir() {
        return bad("not a directory");
    }
    if m.uid() != me {
        return bad("owned by another user");
    }
    if m.mode() & 0o077 != 0 {
        return bad("open to other users");
    }
    Ok(())
}

/// Write `data` to `path` owner-only, through a synced temporary file.
pub fn write_private(path: &Path, data: &[u8]) -> io::Result<()> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let tmp = path.with_extension("json.tmp");
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp)?;
    f.write_all(data)?;
    f.sync_all()?;
    drop(f);
    // A file that existed with wider permissions keeps them through the
    // open; narrow them (a no-op on FAT).
    let _ = fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600));
    fs::rename(&tmp, path)
}

fn valid_url(s: &str, what: &str) -> Result<Url, ImportError> {
    Url::parse(s).map_err(|_| {
        ImportError::Invalid(format!(
            "{what}: {s:?} is not an http:// or https:// address"
        ))
    })
}

impl Accounts {
    pub fn load(dir: &Path) -> Accounts {
        fs::read(dir.join(FILE))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    /// Save into `dir`, which [`private_dir`] must accept.
    pub fn save(&self, dir: &Path) -> io::Result<()> {
        private_dir(dir)?;
        let data = serde_json::to_vec_pretty(self).map_err(io::Error::other)?;
        write_private(&dir.join(FILE), &data)
    }

    /// The KOSync device ID: 32 upper-case hex digits, as KOReader makes
    /// them, generated on first use.
    pub fn device_id(&mut self) -> String {
        if self.device_id.len() != 32 {
            self.device_id = crate::auth::random_hex().to_ascii_uppercase();
        }
        self.device_id.clone()
    }

    /// The `net-allow` entries these settings need, with their feature.
    pub fn allow_entries(&self) -> BTreeSet<(String, &'static str)> {
        let mut out = BTreeSet::new();
        if let Some(e) = self
            .sync
            .as_ref()
            .and_then(|s| Url::parse(&s.server).ok())
            .and_then(|u| entry_for(&u))
        {
            out.insert((e, SYNC_FEATURE));
        }
        for c in &self.catalogs {
            if let Some(e) = Url::parse(&c.url).ok().and_then(|u| entry_for(&u)) {
                out.insert((e, OPDS_FEATURE));
            }
        }
        out
    }

    /// Set the sync server, user or password from typed text (the
    /// keyboard's `TextInput`). A new server or user forgets the key.
    pub fn set_sync_field(&mut self, field: SyncField, value: &str) -> Result<(), ImportError> {
        let value = value.trim();
        let s = self.sync.get_or_insert_with(|| SyncSettings {
            server: String::new(),
            username: String::new(),
            key: String::new(),
            auto: true,
        });
        match field {
            SyncField::Server => {
                let url = valid_url(value, "server")?;
                let url = url.to_string().trim_end_matches('/').to_string();
                if url != s.server {
                    s.server = url;
                    s.key.clear();
                }
            }
            SyncField::Username => {
                if value != s.username {
                    s.username = value.to_string();
                    s.key.clear();
                }
            }
            SyncField::Password => s.key = kosync::key(value),
        }
        Ok(())
    }

    /// Add a catalog by address (typed); its name is the host until the
    /// first feed gives a title.
    pub fn add_catalog(&mut self, url: &str) -> Result<usize, ImportError> {
        let u = valid_url(url.trim(), "catalog")?;
        if let Some(i) = self.catalogs.iter().position(|c| c.url == u.to_string()) {
            return Ok(i);
        }
        self.catalogs.push(CatalogSettings {
            name: u.host().to_string(),
            url: u.to_string(),
            username: None,
            password: None,
        });
        Ok(self.catalogs.len() - 1)
    }

    /// Read `sync.json` if it changed (or always, with `force`).
    pub fn import_sync(&mut self, file: &Path, force: bool) -> Result<Imported, ImportError> {
        let Some(bytes) = read_if_changed(&self.imported, file, force)? else {
            return Ok(if file.exists() {
                Imported::Unchanged
            } else {
                Imported::Missing
            });
        };
        let mut f: SyncFile =
            serde_json::from_slice(&bytes).map_err(|e| ImportError::Json(e.to_string()))?;
        let url = valid_url(&f.server, "server")?;
        let server = url.to_string().trim_end_matches('/').to_string();
        let username = f.username.trim().to_string();
        if username.is_empty() {
            return Err(ImportError::Invalid("username is empty".into()));
        }
        let old = self.sync.take();
        let same = old
            .as_ref()
            .is_some_and(|o| o.server == server && o.username == username);
        let key = match &f.password {
            Some(p) => kosync::key(p),
            None if same => old.as_ref().map(|o| o.key.clone()).unwrap_or_default(),
            None => String::new(),
        };
        self.sync = Some(SyncSettings {
            server,
            username,
            key,
            auto: f.auto.or(old.map(|o| o.auto)).unwrap_or(true),
        });
        let mut stored = bytes;
        if f.password.take().is_some() {
            stored = pretty(&f)?;
            write_private(file, &stored)?;
        }
        self.imported.insert(file_key(file), md5_hex(&stored));
        Ok(Imported::Loaded)
    }

    /// Read `catalogs.json` if it changed (or always, with `force`).
    /// Catalogs are matched by name: a listed one replaces ours (keeping
    /// our password when the file has none for the same address and user),
    /// a new one is added, and ours that the file doesn't list stay.
    pub fn import_catalogs(&mut self, file: &Path, force: bool) -> Result<Imported, ImportError> {
        let Some(bytes) = read_if_changed(&self.imported, file, force)? else {
            return Ok(if file.exists() {
                Imported::Unchanged
            } else {
                Imported::Missing
            });
        };
        let mut f: CatalogsFile =
            serde_json::from_slice(&bytes).map_err(|e| ImportError::Json(e.to_string()))?;
        let mut had_passwords = false;
        for c in &mut f.catalogs {
            let url = valid_url(&c.url, &c.name)?;
            let mut new = CatalogSettings {
                name: if c.name.trim().is_empty() {
                    url.host().to_string()
                } else {
                    c.name.trim().to_string()
                },
                url: url.to_string(),
                username: c.username.clone().filter(|u| !u.trim().is_empty()),
                password: c.password.take(),
            };
            had_passwords |= new.password.is_some();
            let at = self
                .catalogs
                .iter()
                .position(|o| o.name.to_lowercase() == new.name.to_lowercase());
            match at {
                Some(i) => {
                    let o = &self.catalogs[i];
                    if new.password.is_none() && o.url == new.url && o.username == new.username {
                        new.password = o.password.clone();
                    }
                    self.catalogs[i] = new;
                }
                None => self.catalogs.push(new),
            }
        }
        let mut stored = bytes;
        if had_passwords {
            stored = pretty(&f)?;
            write_private(file, &stored)?;
        }
        self.imported.insert(file_key(file), md5_hex(&stored));
        Ok(Imported::Loaded)
    }
}

/// Which sync setting a line of typed text is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyncField {
    Server,
    Username,
    Password,
}

fn file_key(file: &Path) -> String {
    file.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_string()
}

fn read_if_changed(
    seen: &BTreeMap<String, String>,
    file: &Path,
    force: bool,
) -> Result<Option<Vec<u8>>, ImportError> {
    let bytes = match fs::read(file) {
        Ok(b) => b,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    if !force && seen.get(&file_key(file)) == Some(&md5_hex(&bytes)) {
        return Ok(None);
    }
    Ok(Some(bytes))
}

fn pretty<T: Serialize>(v: &T) -> Result<Vec<u8>, ImportError> {
    let mut b = serde_json::to_vec_pretty(v).map_err(|e| ImportError::Json(e.to_string()))?;
    b.push(b'\n');
    Ok(b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn testdir(name: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let d = std::env::temp_dir()
            .join("aristotle-net-tests")
            .join(format!("{name}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&d).unwrap();
        // Owner-only, as private_dir wants a folder for secrets.
        fs::set_permissions(&d, fs::Permissions::from_mode(0o700)).unwrap();
        d
    }

    #[test]
    fn sync_json_is_read_once_and_loses_its_password() {
        let dir = testdir("sync-import");
        let file = dir.join("sync.json");
        fs::write(
            &file,
            r#"{"server": "http://192.168.15.201:17200/", "username": "anna", "password": "test"}"#,
        )
        .unwrap();
        let mut a = Accounts::default();
        assert_eq!(a.import_sync(&file, false).unwrap(), Imported::Loaded);
        let s = a.sync.clone().unwrap();
        assert_eq!(s.server, "http://192.168.15.201:17200");
        assert_eq!(s.key, "098f6bcd4621d373cade4e832627b4f6");
        assert!(s.auto);
        let left = fs::read_to_string(&file).unwrap();
        assert!(
            !left.contains("password") && left.contains("anna"),
            "{left}"
        );
        // The rewritten file counts as read; the key stays.
        assert_eq!(a.import_sync(&file, false).unwrap(), Imported::Unchanged);
        assert_eq!(a.import_sync(&file, true).unwrap(), Imported::Loaded);
        assert_eq!(a.sync.as_ref().unwrap().key, s.key);
        // Another user without a password: no key.
        fs::write(
            &file,
            r#"{"server": "http://h:1", "username": "jan", "auto": false}"#,
        )
        .unwrap();
        a.import_sync(&file, false).unwrap();
        let s = a.sync.as_ref().unwrap();
        assert_eq!((s.key.as_str(), s.auto), ("", false));
        assert!(s.account().is_none());
        fs::write(&file, r#"{"server": "ftp://h", "username": "jan"}"#).unwrap();
        assert!(matches!(
            a.import_sync(&file, false),
            Err(ImportError::Invalid(_))
        ));
        fs::write(&file, "{").unwrap();
        assert!(matches!(
            a.import_sync(&file, false),
            Err(ImportError::Json(_))
        ));
        assert_eq!(
            a.import_sync(&dir.join("none.json"), true).unwrap(),
            Imported::Missing
        );
    }

    #[test]
    fn catalogs_merge_by_name_and_keep_stored_passwords() {
        let dir = testdir("catalogs-import");
        let file = dir.join("catalogs.json");
        fs::write(
            &file,
            r#"{"catalogs": [
                {"name": "Calibre", "url": "http://192.168.15.201:8080/opds", "username": "anna", "password": "lalka"},
                {"name": "", "url": "https://m.gutenberg.org/ebooks.opds/"}
            ]}"#,
        )
        .unwrap();
        let mut a = Accounts::default();
        a.add_catalog("http://books.lan/feed").unwrap();
        assert_eq!(a.import_catalogs(&file, false).unwrap(), Imported::Loaded);
        let names: Vec<&str> = a.catalogs.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["books.lan", "Calibre", "m.gutenberg.org"]);
        assert_eq!(a.catalogs[1].credentials().unwrap().password, "lalka");
        assert!(!format!("{:?}", a.catalogs[1]).contains("lalka"));
        let left = fs::read_to_string(&file).unwrap();
        assert!(!left.contains("lalka") && left.contains("anna"));
        // Re-reading the stripped file keeps the password.
        a.import_catalogs(&file, true).unwrap();
        assert_eq!(a.catalogs[1].password.as_deref(), Some("lalka"));
        assert_eq!(a.catalogs.len(), 3);
        let entries: Vec<String> = a
            .allow_entries()
            .into_iter()
            .map(|(e, f)| format!("{e} {f}"))
            .collect();
        assert_eq!(
            entries,
            [
                "192.168.15.201:8080 opds",
                "books.lan:80 opds",
                "m.gutenberg.org:443 opds"
            ]
        );
    }

    #[test]
    fn secrets_go_only_to_a_private_folder() {
        use std::os::unix::fs::PermissionsExt;
        let base = testdir("private");
        // Created owner-only when missing.
        let dir = base.join("var-local-aristotle");
        private_dir(&dir).unwrap();
        assert_eq!(fs::metadata(&dir).unwrap().permissions().mode() & 0o777, 0o700);
        let a = Accounts {
            device_id: "X".into(),
            ..Accounts::default()
        };
        a.save(&dir).unwrap();
        // A folder others may write (as FAT shows itself, 0777) is refused,
        // and nothing is written into it.
        let open = base.join("userstore");
        fs::create_dir_all(&open).unwrap();
        fs::set_permissions(&open, fs::Permissions::from_mode(0o777)).unwrap();
        assert_eq!(
            a.save(&open).unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
        assert!(!open.join(FILE).exists());
        // And one the framework's `chmod -R g=u` has opened to its group.
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o770)).unwrap();
        assert!(a.save(&dir).is_err());
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
        // So is a symlink to a good folder.
        std::os::unix::fs::symlink(&dir, base.join("link")).unwrap();
        assert!(private_dir(&base.join("link")).is_err());
    }

    #[test]
    fn saved_owner_only_with_typed_fields() {
        let dir = testdir("accounts");
        let mut a = Accounts::default();
        a.set_sync_field(SyncField::Server, "http://192.168.15.201:17200")
            .unwrap();
        a.set_sync_field(SyncField::Username, "anna").unwrap();
        a.set_sync_field(SyncField::Password, "test").unwrap();
        assert!(a.sync.as_ref().unwrap().account().is_some());
        assert!(a.set_sync_field(SyncField::Server, "not a url").is_err());
        // A new user forgets the key.
        a.set_sync_field(SyncField::Username, "jan").unwrap();
        assert!(a.sync.as_ref().unwrap().account().is_none());
        let id = a.device_id();
        assert_eq!(id.len(), 32);
        assert!(id
            .bytes()
            .all(|b| b.is_ascii_digit() || b.is_ascii_uppercase()));
        assert_eq!(a.device_id(), id);
        a.save(&dir).unwrap();
        let mode = fs::metadata(dir.join(FILE)).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        assert_eq!(Accounts::load(&dir), a);
        assert!(a
            .allow_entries()
            .contains(&("192.168.15.201:17200".to_string(), SYNC_FEATURE)));
    }
}
