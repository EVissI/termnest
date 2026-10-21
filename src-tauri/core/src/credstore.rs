//! sudo/su passwords kept in the Windows Credential Manager (encrypted with DPAPI for
//! the current Windows account). They never touch termnest's own files.

use crate::error::{CoreError, CoreResult, ErrorKind};
use crate::secret::SecretString;

/// Credential name of a server user's password.
fn target(host_id: &str, user: &str) -> String {
    format!("termnest:switch:{host_id}:{user}")
}

pub fn save(host_id: &str, user: &str, password: &SecretString) -> CoreResult<()> {
    if password.is_empty() {
        return Err(CoreError::invalid("Пустой пароль"));
    }
    imp::write(&target(host_id, user), user, password.expose().as_bytes())
}

pub fn load(host_id: &str, user: &str) -> Option<SecretString> {
    imp::read(&target(host_id, user))
}

pub fn exists(host_id: &str, user: &str) -> bool {
    load(host_id, user).is_some()
}

pub fn delete(host_id: &str, user: &str) -> CoreResult<()> {
    imp::delete(&target(host_id, user))
}

/// Removes every saved password of a server (when the server is deleted).
pub fn delete_host<'a>(host_id: &str, users: impl IntoIterator<Item = &'a str>) {
    for u in users {
        let _ = delete(host_id, u);
    }
}

#[cfg_attr(not(windows), allow(dead_code))]
fn failed(what: &str, code: u32) -> CoreError {
    CoreError::new(
        ErrorKind::Storage,
        format!("{what} в диспетчере учётных данных Windows (код {code})"),
    )
}

#[cfg(windows)]
mod imp {
    use std::ptr;

    use windows_sys::Win32::Foundation::{GetLastError, ERROR_NOT_FOUND};
    use windows_sys::Win32::Security::Credentials::{
        CredDeleteW, CredFree, CredReadW, CredWriteW, CREDENTIALW, CRED_PERSIST_LOCAL_MACHINE,
        CRED_TYPE_GENERIC,
    };
    use zeroize::Zeroize;

    use super::failed;
    use crate::error::CoreResult;
    use crate::secret::SecretString;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }

    pub fn write(target: &str, user: &str, secret: &[u8]) -> CoreResult<()> {
        let mut name = wide(target);
        let mut user = wide(user);
        let mut blob = secret.to_vec();
        let cred = CREDENTIALW {
            Type: CRED_TYPE_GENERIC,
            TargetName: name.as_mut_ptr(),
            CredentialBlobSize: blob.len() as u32,
            CredentialBlob: blob.as_mut_ptr(),
            Persist: CRED_PERSIST_LOCAL_MACHINE,
            UserName: user.as_mut_ptr(),
            ..Default::default()
        };
        // SAFETY: every pointer refers to a buffer alive for the duration of the call.
        let ok = unsafe { CredWriteW(&cred, 0) };
        blob.zeroize();
        if ok == 0 {
            // SAFETY: plain FFI call without arguments.
            return Err(failed(
                "Не удалось сохранить пароль",
                unsafe { GetLastError() },
            ));
        }
        Ok(())
    }

    pub fn read(target: &str) -> Option<SecretString> {
        let name = wide(target);
        let mut cred: *mut CREDENTIALW = ptr::null_mut();
        // SAFETY: `name` is NUL-terminated; on success `cred` is freed with CredFree below.
        let ok = unsafe { CredReadW(name.as_ptr(), CRED_TYPE_GENERIC, 0, &mut cred) };
        if ok == 0 || cred.is_null() {
            return None;
        }
        // SAFETY: CredReadW returned a valid credential with a blob of the given size.
        let mut bytes = unsafe {
            let c = &*cred;
            std::slice::from_raw_parts(c.CredentialBlob, c.CredentialBlobSize as usize).to_vec()
        };
        // SAFETY: allocated by CredReadW.
        unsafe { CredFree(cred as *const _) };
        let value = String::from_utf8(bytes.clone()).ok();
        bytes.zeroize();
        value.map(SecretString::new)
    }

    pub fn delete(target: &str) -> CoreResult<()> {
        let name = wide(target);
        // SAFETY: `name` is NUL-terminated.
        if unsafe { CredDeleteW(name.as_ptr(), CRED_TYPE_GENERIC, 0) } == 0 {
            // SAFETY: plain FFI call without arguments.
            let code = unsafe { GetLastError() };
            if code != ERROR_NOT_FOUND {
                return Err(failed("Не удалось удалить пароль", code));
            }
        }
        Ok(())
    }
}

/// Other platforms (the Linux build container) only run tests: nothing is stored.
#[cfg(not(windows))]
mod imp {
    use crate::error::{CoreError, CoreResult, ErrorKind};
    use crate::secret::SecretString;

    pub fn write(_target: &str, _user: &str, _secret: &[u8]) -> CoreResult<()> {
        Err(CoreError::new(
            ErrorKind::Storage,
            "Хранение паролей доступно только в Windows",
        ))
    }

    pub fn read(_target: &str) -> Option<SecretString> {
        None
    }

    pub fn delete(_target: &str) -> CoreResult<()> {
        Ok(())
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let host = format!("test-{}", crate::ids::new_id());
        assert!(!exists(&host, "root"));
        save(&host, "root", &SecretString::new("пароль 1".into())).unwrap();
        assert_eq!(load(&host, "root").unwrap().expose(), "пароль 1");
        save(&host, "root", &SecretString::new("p2".into())).unwrap();
        assert_eq!(load(&host, "root").unwrap().expose(), "p2");
        delete(&host, "root").unwrap();
        assert!(!exists(&host, "root"));
        delete(&host, "root").unwrap();
        assert!(save(&host, "root", &SecretString::new(String::new())).is_err());
    }
}
