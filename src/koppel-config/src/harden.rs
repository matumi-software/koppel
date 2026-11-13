//! Restricts a directory that holds config or keys, as 0.1.x's `DirectoryHardening` does. Safe to
//! run on every start.
//!
//! - **Windows:** the owner becomes BUILTIN\Administrators and the DACL is protected (nothing
//!   inherited, so the broad Users grant `%ProgramData%` hands down is dropped) with full control
//!   for SYSTEM and Administrators only. A junction or symbolic link is refused, because the ACL
//!   would land on its target. A process that is not elevated cannot give the owner away and would
//!   lock itself out, so it also gets full control and the owner is left alone: that is a developer
//!   at a console, while the service runs as LocalSystem.
//! - **Elsewhere:** a symbolic link is refused as on Windows, and so is a directory owned by
//!   anyone but the user Koppel runs as, since its owner could swap what is in it. Then `chmod 700`.
//!   A directory that cannot be changed, such as a bind mount owned by someone else, is an error.

use std::io;
use std::path::Path;

/// Restricts `directory` to administrators (Windows) or its owner (elsewhere).
pub fn harden_directory(directory: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        windows_acl::harden(directory)
    }
    #[cfg(not(windows))]
    {
        harden_unix(directory)
    }
}

#[cfg(not(windows))]
fn harden_unix(directory: &Path) -> io::Result<()> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let metadata = std::fs::symlink_metadata(directory)?;
    if metadata.file_type().is_symlink() {
        return Err(io::Error::other(format!(
            "{} is a symbolic link. Koppel will not keep its files behind a link: point Paths:Config and Paths:Keys at a real directory.",
            directory.display()
        )));
    }
    // SAFETY: geteuid takes no arguments and cannot fail.
    let user = unsafe { libc::geteuid() };
    if metadata.uid() != user {
        return Err(io::Error::other(format!(
            "{} belongs to user {}, not to user {user}, which Koppel runs as. Its owner could swap what is in it: give it to the user Koppel runs as.",
            directory.display(),
            metadata.uid()
        )));
    }
    std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700))
}

#[cfg(windows)]
pub(crate) mod windows_acl {
    use std::io;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::fs::MetadataExt;
    use std::path::Path;

    use windows::Win32::Foundation::{CloseHandle, HANDLE, HLOCAL, LocalFree, WIN32_ERROR};
    use windows::Win32::NetworkManagement::NetManagement::{
        LG_INCLUDE_INDIRECT, LOCALGROUP_MEMBERS_INFO_0, LOCALGROUP_USERS_INFO_0,
        MAX_PREFERRED_LENGTH, NetApiBufferFree, NetLocalGroupGetMembers, NetUserGetLocalGroups,
    };
    use windows::Win32::Security::Authorization::{
        ConvertSidToStringSidW, ConvertStringSidToSidW, EXPLICIT_ACCESS_W, GetNamedSecurityInfoW,
        NO_MULTIPLE_TRUSTEE, SE_FILE_OBJECT, SET_ACCESS, SetEntriesInAclW, SetNamedSecurityInfoW,
        TRUSTEE_IS_SID, TRUSTEE_IS_UNKNOWN, TRUSTEE_W,
    };
    use windows::Win32::Security::{
        ACE_FLAGS, ACL, CheckTokenMembership, CreateWellKnownSid, DACL_SECURITY_INFORMATION,
        GetTokenInformation, LookupAccountNameW, LookupAccountSidW, NO_INHERITANCE,
        OBJECT_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION,
        PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID, SID_NAME_USE,
        SUB_CONTAINERS_AND_OBJECTS_INHERIT, TOKEN_QUERY, TOKEN_USER, TokenUser,
        WELL_KNOWN_SID_TYPE, WinBuiltinAdministratorsSid, WinLocalSystemSid,
    };
    use windows::Win32::Storage::FileSystem::FILE_ALL_ACCESS;
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    use windows::core::{BOOL, PCWSTR, PWSTR};

    use crate::data_folder::Kind;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;

    /// A SID in a buffer of its own.
    pub(crate) struct Sid(Vec<u8>);

    impl Sid {
        pub(crate) fn well_known(kind: WELL_KNOWN_SID_TYPE) -> io::Result<Self> {
            let mut size = 68u32; // SECURITY_MAX_SID_SIZE
            let mut buffer = vec![0u8; size as usize];
            // SAFETY: `buffer` is `size` bytes long.
            unsafe {
                CreateWellKnownSid(
                    kind,
                    None,
                    Some(PSID(buffer.as_mut_ptr().cast())),
                    &mut size,
                )
            }
            .map_err(io::Error::other)?;
            buffer.truncate(size as usize);
            Ok(Self(buffer))
        }

        pub(crate) fn system() -> io::Result<Self> {
            Self::well_known(WinLocalSystemSid)
        }

        pub(crate) fn administrators() -> io::Result<Self> {
            Self::well_known(WinBuiltinAdministratorsSid)
        }

        /// The user this process runs as.
        pub(crate) fn current_user() -> io::Result<Self> {
            let mut token = HANDLE::default();
            // SAFETY: plain calls on this process's own token, closed below.
            unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) }
                .map_err(io::Error::other)?;
            let result = (|| {
                let mut size = 0u32;
                // SAFETY: the first call only asks for the size.
                let _ = unsafe { GetTokenInformation(token, TokenUser, None, 0, &mut size) };
                let mut buffer = vec![0u64; (size as usize).div_ceil(8)];
                // SAFETY: `buffer` holds `size` bytes, aligned for TOKEN_USER.
                unsafe {
                    GetTokenInformation(
                        token,
                        TokenUser,
                        Some(buffer.as_mut_ptr().cast()),
                        size,
                        &mut size,
                    )
                }
                .map_err(io::Error::other)?;
                // SAFETY: the call filled the buffer with a TOKEN_USER whose SID points into it.
                let user = unsafe { &*(buffer.as_ptr() as *const TOKEN_USER) };
                let sid = user.User.Sid;
                // SAFETY: a valid SID's length is 8 bytes plus 4 per sub-authority.
                let length = 8 + 4 * usize::from(unsafe { *(sid.0 as *const u8).add(1) });
                // SAFETY: the SID is `length` bytes inside `buffer`.
                Ok(Self(
                    unsafe { std::slice::from_raw_parts(sid.0 as *const u8, length) }.to_vec(),
                ))
            })();
            // SAFETY: the handle came from OpenProcessToken.
            let _ = unsafe { CloseHandle(token) };
            result
        }

        pub(crate) fn psid(&self) -> PSID {
            PSID(self.0.as_ptr() as *mut _)
        }

        #[cfg(test)]
        pub(crate) fn bytes(&self) -> &[u8] {
            &self.0
        }
    }

    fn is_member(sid: &Sid) -> bool {
        let mut member = BOOL(0);
        // SAFETY: `sid` is a valid SID for the length of the call.
        unsafe { CheckTokenMembership(None, sid.psid(), &mut member) }.is_ok() && member.as_bool()
    }

    fn check(code: WIN32_ERROR) -> io::Result<()> {
        if code.0 == 0 {
            Ok(())
        } else {
            Err(io::Error::from_raw_os_error(code.0 as i32))
        }
    }

    fn full_control(sid: &Sid, inheritance: ACE_FLAGS) -> EXPLICIT_ACCESS_W {
        EXPLICIT_ACCESS_W {
            grfAccessPermissions: FILE_ALL_ACCESS.0,
            grfAccessMode: SET_ACCESS,
            grfInheritance: inheritance,
            Trustee: TRUSTEE_W {
                pMultipleTrustee: std::ptr::null_mut(),
                MultipleTrusteeOperation: NO_MULTIPLE_TRUSTEE,
                TrusteeForm: TRUSTEE_IS_SID,
                TrusteeType: TRUSTEE_IS_UNKNOWN,
                ptstrName: PWSTR(sid.psid().0.cast()),
            },
        }
    }

    fn wide(path: &Path) -> Vec<u16> {
        path.as_os_str().encode_wide().chain([0]).collect()
    }

    fn wide_str(text: &str) -> Vec<u16> {
        text.encode_utf16().chain([0]).collect()
    }

    /// Whether this process is SYSTEM or an elevated administrator, which can give an owner away.
    /// .NET's `IsSystem || IsInRole(Administrator)`: membership counts only when it is in force, so
    /// an unelevated administrator is not privileged.
    pub(crate) fn is_privileged() -> io::Result<bool> {
        Ok(is_member(&Sid::system()?) || is_member(&Sid::administrators()?))
    }

    /// What a path is, without following it: a reparse point (a junction, a symbolic link, a mount
    /// point) is a link.
    pub(crate) fn kind_of(metadata: &std::fs::Metadata) -> Kind {
        let attributes = metadata.file_attributes();
        if attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 || metadata.file_type().is_symlink() {
            Kind::Link {
                folder: attributes & FILE_ATTRIBUTE_DIRECTORY != 0,
            }
        } else if metadata.is_dir() {
            Kind::Folder
        } else {
            Kind::File
        }
    }

    /// Replaces the ACL of `path` with a protected one for SYSTEM and Administrators, full control,
    /// and when `privileged` gives the owner to Administrators. An unprivileged process cannot give
    /// the owner away and would lock itself out, so it also gets full control and the owner is left
    /// alone: that is a developer at a console, while the service runs as LocalSystem. A folder's
    /// entries are inherited by what is in it.
    pub(crate) fn protect(path: &Path, container: bool, privileged: bool) -> io::Result<()> {
        let system = Sid::system()?;
        let administrators = Sid::administrators()?;
        let user = Sid::current_user()?;
        let inheritance = if container {
            SUB_CONTAINERS_AND_OBJECTS_INHERIT
        } else {
            NO_INHERITANCE
        };

        let mut entries = vec![
            full_control(&system, inheritance),
            full_control(&administrators, inheritance),
        ];
        if !privileged {
            entries.push(full_control(&user, inheritance));
        }
        let mut acl: *mut ACL = std::ptr::null_mut();
        // SAFETY: the entries point at SIDs that live until the end of this function.
        check(unsafe { SetEntriesInAclW(Some(&entries), None, &mut acl) })?;

        let mut information: OBJECT_SECURITY_INFORMATION =
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION;
        if privileged {
            information |= OWNER_SECURITY_INFORMATION;
        }
        let wide = wide(path);
        // SAFETY: `wide` is a terminated path, and the ACL and SID are valid for the call.
        let result = check(unsafe {
            SetNamedSecurityInfoW(
                PCWSTR(wide.as_ptr()),
                SE_FILE_OBJECT,
                information,
                privileged.then(|| administrators.psid()),
                None,
                Some(acl),
                None,
            )
        });
        // SAFETY: SetEntriesInAclW allocated the ACL with LocalAlloc.
        unsafe { LocalFree(Some(HLOCAL(acl.cast()))) };
        result
    }

    /// The text form of a SID (`S-1-5-18`).
    pub(crate) fn sid_text(sid: PSID) -> io::Result<String> {
        let mut text = PWSTR::null();
        // SAFETY: `sid` is a valid SID; the string is read once and freed with LocalFree.
        unsafe { ConvertSidToStringSidW(sid, &mut text) }.map_err(io::Error::other)?;
        // SAFETY: the call returned a terminated string.
        let result = unsafe { text.to_string() }.map_err(io::Error::other);
        // SAFETY: ConvertSidToStringSidW allocated the string with LocalAlloc.
        unsafe { LocalFree(Some(HLOCAL(text.0.cast()))) };
        result
    }

    /// A SID read from its text form, in a buffer of its own.
    fn sid_from_text(text: &str) -> io::Result<Sid> {
        let wide = wide_str(text);
        let mut sid = PSID::default();
        // SAFETY: `wide` is a terminated string; the SID is copied out and freed below.
        unsafe { ConvertStringSidToSidW(PCWSTR(wide.as_ptr()), &mut sid) }
            .map_err(io::Error::other)?;
        // SAFETY: a valid SID's length is 8 bytes plus 4 per sub-authority.
        let length = 8 + 4 * usize::from(unsafe { *(sid.0 as *const u8).add(1) });
        // SAFETY: the SID is `length` bytes long.
        let bytes = unsafe { std::slice::from_raw_parts(sid.0 as *const u8, length) }.to_vec();
        // SAFETY: ConvertStringSidToSidW allocated the SID with LocalAlloc.
        unsafe { LocalFree(Some(HLOCAL(sid.0.cast()))) };
        Ok(Sid(bytes))
    }

    /// The SID of the owner of `path`, in text form. A link is not followed.
    pub(crate) fn owner(path: &Path) -> io::Result<String> {
        let wide = wide(path);
        let mut owner = PSID::default();
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        // SAFETY: `wide` is a terminated path; the owner points into the descriptor, which is
        // freed after the text is made.
        check(unsafe {
            GetNamedSecurityInfoW(
                PCWSTR(wide.as_ptr()),
                SE_FILE_OBJECT,
                OWNER_SECURITY_INFORMATION,
                Some(&mut owner),
                None,
                None,
                None,
                &mut descriptor,
            )
        })?;
        let text = if owner.0.is_null() {
            Err(io::Error::other("the path has no owner"))
        } else {
            sid_text(owner)
        };
        // SAFETY: GetNamedSecurityInfoW allocated the descriptor with LocalAlloc.
        unsafe { LocalFree(Some(HLOCAL(descriptor.0))) };
        text
    }

    /// `DOMAIN\name` for a SID in text form, when Windows knows it.
    pub(crate) fn name_of(sid: &str) -> Option<String> {
        let sid = sid_from_text(sid).ok()?;
        let (name, domain) = account_name(&sid)?;
        Some(if domain.is_empty() {
            name
        } else {
            format!("{domain}\\{name}")
        })
    }

    /// The name and the domain of an account.
    pub(crate) fn account_name(sid: &Sid) -> Option<(String, String)> {
        let (mut name_len, mut domain_len) = (0u32, 0u32);
        let mut usage = SID_NAME_USE::default();
        // SAFETY: the first call only asks for the lengths and fails with an insufficient buffer.
        let _ = unsafe {
            LookupAccountSidW(
                PCWSTR::null(),
                sid.psid(),
                None,
                &mut name_len,
                None,
                &mut domain_len,
                &mut usage,
            )
        };
        if name_len == 0 {
            return None;
        }
        let mut name = vec![0u16; name_len as usize];
        let mut domain = vec![0u16; domain_len.max(1) as usize];
        // SAFETY: the buffers are as long as the lengths say.
        unsafe {
            LookupAccountSidW(
                PCWSTR::null(),
                sid.psid(),
                Some(PWSTR(name.as_mut_ptr())),
                &mut name_len,
                Some(PWSTR(domain.as_mut_ptr())),
                &mut domain_len,
                &mut usage,
            )
        }
        .ok()?;
        Some((
            String::from_utf16_lossy(&name[..name_len as usize]),
            String::from_utf16_lossy(&domain[..domain_len as usize]),
        ))
    }

    /// The SID, in text form, of an account or group named `name`.
    fn sid_of_name(name: &str) -> Option<String> {
        let wide = wide_str(name);
        let (mut sid_len, mut domain_len) = (0u32, 0u32);
        let mut usage = SID_NAME_USE::default();
        // SAFETY: the first call only asks for the lengths.
        let _ = unsafe {
            LookupAccountNameW(
                PCWSTR::null(),
                PCWSTR(wide.as_ptr()),
                None,
                &mut sid_len,
                None,
                &mut domain_len,
                &mut usage,
            )
        };
        if sid_len == 0 {
            return None;
        }
        let mut sid = vec![0u8; sid_len as usize];
        let mut domain = vec![0u16; domain_len.max(1) as usize];
        // SAFETY: the buffers are as long as the lengths say.
        unsafe {
            LookupAccountNameW(
                PCWSTR::null(),
                PCWSTR(wide.as_ptr()),
                Some(PSID(sid.as_mut_ptr().cast())),
                &mut sid_len,
                Some(PWSTR(domain.as_mut_ptr())),
                &mut domain_len,
                &mut usage,
            )
        }
        .ok()?;
        sid_text(PSID(sid.as_mut_ptr().cast())).ok()
    }

    /// The accounts and groups directly in the local Administrators group, by SID. The group's name
    /// is looked up from its well-known SID, as it is localized. The SIDs are read as they are, so
    /// an AzureAD or domain member that has no name here is listed all the same.
    pub(crate) fn administrator_members() -> io::Result<Vec<String>> {
        let (name, _) = account_name(&Sid::administrators()?)
            .ok_or_else(|| io::Error::other("the Administrators group has no name"))?;
        let name = wide_str(&name);
        let mut buffer: *mut u8 = std::ptr::null_mut();
        let (mut read, mut total) = (0u32, 0u32);
        // SAFETY: the buffer the call allocates is read below and freed with NetApiBufferFree.
        let code = unsafe {
            NetLocalGroupGetMembers(
                PCWSTR::null(),
                PCWSTR(name.as_ptr()),
                0,
                &mut buffer,
                MAX_PREFERRED_LENGTH,
                &mut read,
                &mut total,
                None,
            )
        };
        if code != 0 {
            return Err(io::Error::from_raw_os_error(code as i32));
        }
        // SAFETY: at level 0 the buffer holds `read` LOCALGROUP_MEMBERS_INFO_0 entries.
        let entries = unsafe {
            std::slice::from_raw_parts(buffer as *const LOCALGROUP_MEMBERS_INFO_0, read as usize)
        };
        let members = entries.iter().map(|e| sid_text(e.lgrmi0_sid)).collect();
        // SAFETY: the buffer came from NetLocalGroupGetMembers.
        unsafe { NetApiBufferFree(Some(buffer as *const _)) };
        members
    }

    /// The SIDs of the local groups an account belongs to, directly or through other groups.
    pub(crate) fn groups_of(sid: &str) -> io::Result<Vec<String>> {
        let (name, domain) = account_name(&sid_from_text(sid)?)
            .ok_or_else(|| io::Error::other("the account has no name"))?;
        let account = if domain.is_empty() {
            name
        } else {
            format!("{domain}\\{name}")
        };
        let account = wide_str(&account);
        let mut buffer: *mut u8 = std::ptr::null_mut();
        let (mut read, mut total) = (0u32, 0u32);
        // SAFETY: the buffer the call allocates is read below and freed with NetApiBufferFree.
        let code = unsafe {
            NetUserGetLocalGroups(
                PCWSTR::null(),
                PCWSTR(account.as_ptr()),
                0,
                LG_INCLUDE_INDIRECT,
                &mut buffer,
                MAX_PREFERRED_LENGTH,
                &mut read,
                &mut total,
            )
        };
        if code != 0 {
            return Err(io::Error::from_raw_os_error(code as i32));
        }
        // SAFETY: at level 0 the buffer holds `read` LOCALGROUP_USERS_INFO_0 entries.
        let entries = unsafe {
            std::slice::from_raw_parts(buffer as *const LOCALGROUP_USERS_INFO_0, read as usize)
        };
        let groups = entries
            .iter()
            // SAFETY: each name is a terminated string inside the buffer.
            .filter_map(|e| unsafe { e.lgrui0_name.to_string() }.ok())
            .filter_map(|name| sid_of_name(&name))
            .collect();
        // SAFETY: the buffer came from NetUserGetLocalGroups.
        unsafe { NetApiBufferFree(Some(buffer as *const _)) };
        Ok(groups)
    }

    pub(crate) fn harden(directory: &Path) -> io::Result<()> {
        let metadata = std::fs::symlink_metadata(directory)?;
        if matches!(kind_of(&metadata), Kind::Link { .. }) {
            return Err(io::Error::other(format!(
                "{} is a junction or symbolic link. Koppel will not keep its files behind a link: point Paths:Config and Paths:Keys at a real directory.",
                directory.display()
            )));
        }
        protect(directory, true, is_privileged()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("koppel-harden-{name}-{}", crate::random_hex(6)));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[cfg(unix)]
    #[test]
    fn the_directory_becomes_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("unix");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        harden_directory(&dir).unwrap();
        assert_eq!(
            std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
            0o700
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_symbolic_link_is_refused() {
        let dir = temp_dir("symlink");
        let target = dir.join("target");
        std::fs::create_dir(&target).unwrap();
        let link = dir.join("link");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let refused = harden_directory(&link).unwrap_err();
        assert!(refused.to_string().contains("symbolic link"), "{refused}");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Needs root to give a directory away, which the test container is.
    #[cfg(unix)]
    #[test]
    fn a_directory_owned_by_someone_else_is_refused() {
        // SAFETY: geteuid takes no arguments and cannot fail.
        if unsafe { libc::geteuid() } != 0 {
            return;
        }
        let dir = temp_dir("owner");
        std::os::unix::fs::chown(&dir, Some(12345), None).unwrap();
        let refused = harden_directory(&dir).unwrap_err();
        assert!(
            refused.to_string().contains("belongs to user 12345"),
            "{refused}"
        );
        std::os::unix::fs::chown(&dir, Some(0), None).unwrap();
        harden_directory(&dir).unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(windows)]
    #[test]
    fn the_acl_is_protected_and_lists_only_system_administrators_and_an_unelevated_user() {
        use windows::Win32::Foundation::{HLOCAL, LocalFree};
        use windows::Win32::Security::Authorization::{GetNamedSecurityInfoW, SE_FILE_OBJECT};
        use windows::Win32::Security::{
            ACCESS_ALLOWED_ACE, ACL, ACL_SIZE_INFORMATION, AclSizeInformation,
            DACL_SECURITY_INFORMATION, GetAce, GetAclInformation, GetSecurityDescriptorControl,
            PSECURITY_DESCRIPTOR,
        };
        use windows_acl::Sid;

        let dir = temp_dir("acl");
        harden_directory(&dir).unwrap();
        harden_directory(&dir).unwrap();

        let wide: Vec<u16> = {
            use std::os::windows::ffi::OsStrExt;
            dir.as_os_str().encode_wide().chain([0]).collect()
        };
        let mut acl: *mut ACL = std::ptr::null_mut();
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        // SAFETY: test-only reads of the ACL just written; the descriptor is freed below.
        unsafe {
            let code = GetNamedSecurityInfoW(
                windows::core::PCWSTR(wide.as_ptr()),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                None,
                None,
                Some(&mut acl),
                None,
                &mut descriptor,
            );
            assert_eq!(code.0, 0);
            let (mut control, mut revision) = (0u16, 0u32);
            GetSecurityDescriptorControl(descriptor, &mut control, &mut revision).unwrap();
            assert_ne!(control & 0x1000, 0, "SE_DACL_PROTECTED");

            let mut size = ACL_SIZE_INFORMATION::default();
            GetAclInformation(
                acl,
                (&mut size as *mut ACL_SIZE_INFORMATION).cast(),
                size_of::<ACL_SIZE_INFORMATION>() as u32,
                AclSizeInformation,
            )
            .unwrap();
            let mut sids = Vec::new();
            for index in 0..size.AceCount {
                let mut ace = std::ptr::null_mut();
                GetAce(acl, index, &mut ace).unwrap();
                let ace = &*(ace as *const ACCESS_ALLOWED_ACE);
                assert_eq!(ace.Header.AceType, 0, "allow only");
                assert_eq!(ace.Mask, 0x1F01FF, "full control");
                let start = &ace.SidStart as *const u32 as *const u8;
                let length = 8 + 4 * usize::from(*start.add(1));
                sids.push(std::slice::from_raw_parts(start, length).to_vec());
            }
            LocalFree(Some(HLOCAL(descriptor.0)));

            let system = Sid::system().unwrap().bytes().to_vec();
            let administrators = Sid::administrators().unwrap().bytes().to_vec();
            let user = Sid::current_user().unwrap().bytes().to_vec();
            assert!(sids.contains(&system));
            assert!(sids.contains(&administrators));
            assert!(
                sids.iter()
                    .all(|sid| *sid == system || *sid == administrators || *sid == user),
                "nobody else"
            );
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(windows)]
    #[test]
    fn a_junction_is_refused() {
        let dir = temp_dir("link");
        let target = dir.join("target");
        let link = dir.join("link");
        std::fs::create_dir(&target).unwrap();
        let made = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&link)
            .arg(&target)
            .output()
            .unwrap();
        assert!(made.status.success());
        let refused = harden_directory(&link).unwrap_err();
        assert!(refused.to_string().contains("junction or symbolic link"));
        let _ = std::fs::remove_dir(&link);
        let _ = std::fs::remove_dir_all(dir);
    }
}
