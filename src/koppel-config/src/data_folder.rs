//! Makes Koppel's data folder (`%ProgramData%\Koppel`) usable by SYSTEM and administrators only, or
//! checks that it still is. This is what the installer runs, as `koppel prepare-data-folder`, and it
//! replaces the PowerShell script the installer used to carry.
//!
//! `%ProgramData%` lets any standard user create folders in it. Koppel keeps its key ring and config
//! there and its service runs as LocalSystem, so a folder a standard user created first (owned by
//! them, or a junction into somewhere else) would let that user read the keys or steer what SYSTEM
//! writes.
//!
//! [`prepare`] refuses a folder that is a file, a link, or owned by an account that is not an
//! administrator. An administrator is SYSTEM, Administrators, TrustedInstaller, an account that is a
//! direct member of the local Administrators group, or one that is a member through a group. The
//! last case matters: on many machines whatever an administrator creates is owned by that
//! administrator, so an earlier install may have left the folder owned by one. Such a folder is
//! taken over: its owner becomes Administrators, and its ACL and the ACL of everything under it
//! become a protected one that grants only SYSTEM and Administrators full control. A link found
//! under it is deleted (the link only, never what it points to), and a file or folder under it that
//! no administrator owns is renamed aside (`<name>.not-admin-owned-<time>`) and left as it is. A
//! missing folder is created the same way. [`check`] only verifies and changes nothing.
//!
//! The decisions are here and run on every system. What touches the machine (an owner, an ACL, the
//! members of a group) is behind [`Accounts`] and [`Files`], so the tests can name any owner and any
//! group membership. `windows_acl` has the Windows ones.

use std::cell::OnceCell;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use time::OffsetDateTime;

use crate::utc_stamp;

/// SYSTEM.
pub const SYSTEM: &str = "S-1-5-18";
/// BUILTIN\Administrators.
pub const ADMINISTRATORS: &str = "S-1-5-32-544";
/// NT SERVICE\TrustedInstaller, which owns much of the system and is as trusted as SYSTEM.
pub const TRUSTED_INSTALLER: &str =
    "S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464";

/// Why a folder was not prepared.
#[derive(Debug)]
pub enum DataFolderError {
    /// `check` found no folder.
    Missing(PathBuf),
    /// A file is where the folder should be.
    NotAFolder(PathBuf),
    /// A junction or symbolic link is where the folder should be.
    Link(PathBuf),
    /// The folder belongs to an account that is not an administrator.
    Owner { path: PathBuf, owner: String },
    /// Something the machine refused or could not answer.
    Io(PathBuf, io::Error),
}

impl fmt::Display for DataFolderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing(path) => write!(f, "{} does not exist.", path.display()),
            Self::NotAFolder(path) => write!(
                f,
                "{} is a file. Koppel needs a folder there. Move the file away and run setup again.",
                path.display()
            ),
            Self::Link(path) => write!(
                f,
                "{} is a junction or symbolic link. Koppel keeps its keys there and will not follow it. Delete the link and run setup again.",
                path.display()
            ),
            Self::Owner { path, owner } => write!(
                f,
                "{} is owned by {owner}, who is not an administrator. Koppel keeps its keys there and will not use a folder a standard user controls. Rename or delete the folder (it may hold keys from an earlier install: copy them out first if you need them) and run setup again.",
                path.display()
            ),
            Self::Io(path, e) => write!(f, "{}: {e}", path.display()),
        }
    }
}

impl std::error::Error for DataFolderError {}

/// What something in the folder is. Anything that is a reparse point on Windows or a symbolic link
/// elsewhere is a [`Kind::Link`], and is never followed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    File,
    Folder,
    /// A link, and whether it presents as a folder (which decides how it is deleted).
    Link {
        folder: bool,
    },
}

/// One entry of a folder.
#[derive(Debug, Clone)]
pub struct Child {
    pub name: String,
    pub kind: Kind,
}

/// Who counts as an administrator on this machine.
pub trait Accounts {
    /// The SIDs directly in the local Administrators group, accounts and groups alike.
    fn administrator_members(&self) -> io::Result<Vec<String>>;
    /// The SIDs of the groups the account belongs to. Empty when they cannot be told, which means
    /// "not shown to be an administrator".
    fn groups_of(&self, sid: &str) -> Vec<String>;
    /// The account's name for a message, when it has one.
    fn name_of(&self, sid: &str) -> Option<String>;
}

/// What the preparation does to the folder and what it reads from it. Nothing here follows a link.
pub trait Files {
    /// What `path` is, or `None` when nothing is there.
    fn kind(&self, path: &Path) -> io::Result<Option<Kind>>;
    /// The SID of the owner.
    fn owner(&self, path: &Path) -> io::Result<String>;
    /// Makes `path` owned by Administrators with a protected ACL that grants SYSTEM and
    /// Administrators full control (inherited by what is under it when `container`).
    fn protect(&self, path: &Path, container: bool) -> io::Result<()>;
    /// Makes a folder that does not exist yet. Fails when something is there, so a success means it
    /// is ours.
    fn create_folder(&self, path: &Path) -> io::Result<()>;
    /// The entries directly in a folder, hidden ones too.
    fn children(&self, path: &Path) -> io::Result<Vec<Child>>;
    /// Deletes a link and nothing it points to.
    fn remove_link(&self, path: &Path, folder: bool) -> io::Result<()>;
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()>;
}

/// The administrator test, with the group list read at most once.
struct Administrators<'a> {
    accounts: &'a dyn Accounts,
    members: OnceCell<Vec<String>>,
}

impl<'a> Administrators<'a> {
    fn new(accounts: &'a dyn Accounts) -> Self {
        Self {
            accounts,
            members: OnceCell::new(),
        }
    }

    fn is_administrator(&self, sid: &str) -> io::Result<bool> {
        if [SYSTEM, ADMINISTRATORS, TRUSTED_INSTALLER]
            .iter()
            .any(|trusted| trusted.eq_ignore_ascii_case(sid))
        {
            return Ok(true);
        }
        if self.members.get().is_none() {
            let _ = self.members.set(self.accounts.administrator_members()?);
        }
        let members = self.members.get().expect("set above");
        let listed = |other: &str| members.iter().any(|m| m.eq_ignore_ascii_case(other));
        if listed(sid) {
            return Ok(true);
        }
        // A member through a group.
        Ok(self
            .accounts
            .groups_of(sid)
            .iter()
            .any(|group| listed(group) || group.eq_ignore_ascii_case(ADMINISTRATORS)))
    }
}

struct Run<'a> {
    files: &'a dyn Files,
    administrators: Administrators<'a>,
    say: &'a mut dyn FnMut(&str),
    now: fn() -> OffsetDateTime,
}

/// Refuses what is not Koppel's to use: a file, a link or a folder a standard user owns.
fn test_folder(
    files: &dyn Files,
    administrators: &Administrators<'_>,
    accounts: &dyn Accounts,
    path: &Path,
) -> Result<(), DataFolderError> {
    let io = |e| DataFolderError::Io(path.to_path_buf(), e);
    match files.kind(path).map_err(io)? {
        None => return Err(DataFolderError::Missing(path.to_path_buf())),
        Some(Kind::File) => return Err(DataFolderError::NotAFolder(path.to_path_buf())),
        Some(Kind::Link { .. }) => return Err(DataFolderError::Link(path.to_path_buf())),
        Some(Kind::Folder) => {}
    }
    let owner = files.owner(path).map_err(io)?;
    if !administrators.is_administrator(&owner).map_err(io)? {
        let owner = accounts.name_of(&owner).unwrap_or(owner);
        return Err(DataFolderError::Owner {
            path: path.to_path_buf(),
            owner,
        });
    }
    Ok(())
}

impl Run<'_> {
    fn protect_tree(&mut self, folder: &Path) -> Result<(), DataFolderError> {
        let io = |path: &Path| {
            let path = path.to_path_buf();
            move |e| DataFolderError::Io(path, e)
        };
        self.files.protect(folder, true).map_err(io(folder))?;
        for child in self.files.children(folder).map_err(io(folder))? {
            let path = folder.join(&child.name);
            match child.kind {
                Kind::Link { folder: is_folder } => {
                    // Deleting a link removes the link and not its target. Never follow it.
                    (self.say)(&format!(
                        "Removing the link {} (it points somewhere else; Koppel does not use links).",
                        path.display()
                    ));
                    self.files
                        .remove_link(&path, is_folder)
                        .map_err(io(&path))?;
                }
                _ if child.name.contains(".not-admin-owned-") => {}
                kind => {
                    let owner = self.files.owner(&path).map_err(io(&path))?;
                    if !self
                        .administrators
                        .is_administrator(&owner)
                        .map_err(io(&path))?
                    {
                        // Something a standard user made (or was handed) inside the folder.
                        // Re-owning it would hand its content to SYSTEM as if it were ours, so it
                        // is moved out of Koppel's way instead, and its ACL is left alone.
                        let aside =
                            format!("{}.not-admin-owned-{}", child.name, utc_stamp((self.now)()));
                        (self.say)(&format!(
                            "{} is not owned by an administrator. Renaming it to {aside}; Koppel does not use it.",
                            path.display()
                        ));
                        self.files
                            .rename(&path, &folder.join(&aside))
                            .map_err(io(&path))?;
                    } else if kind == Kind::Folder {
                        self.protect_tree(&path)?;
                    } else {
                        self.files.protect(&path, false).map_err(io(&path))?;
                    }
                }
            }
        }
        Ok(())
    }
}

/// Makes `path` Koppel's: creates it when it is missing, takes it over when an administrator owns
/// it, refuses it otherwise. What it does that the person should know of goes to `say`.
pub fn prepare(
    path: &Path,
    accounts: &dyn Accounts,
    files: &dyn Files,
    say: &mut dyn FnMut(&str),
) -> Result<(), DataFolderError> {
    prepare_at(path, accounts, files, say, OffsetDateTime::now_utc)
}

fn prepare_at(
    path: &Path,
    accounts: &dyn Accounts,
    files: &dyn Files,
    say: &mut dyn FnMut(&str),
    now: fn() -> OffsetDateTime,
) -> Result<(), DataFolderError> {
    let io = |e| DataFolderError::Io(path.to_path_buf(), e);
    let mut run = Run {
        files,
        administrators: Administrators::new(accounts),
        say,
        now,
    };
    let exists = files.kind(path).map_err(io)?.is_some();
    if !exists {
        files.create_folder(path).map_err(io)?;
    } else {
        test_folder(files, &run.administrators, accounts, path)?;
    }
    run.protect_tree(path)?;
    test_folder(files, &run.administrators, accounts, path)?;
    (run.say)(&format!(
        "{} is owned by Administrators and open to SYSTEM and Administrators only.",
        path.display()
    ));
    Ok(())
}

/// Verifies that `path` is a folder Koppel may use, and changes nothing.
pub fn check(
    path: &Path,
    accounts: &dyn Accounts,
    files: &dyn Files,
) -> Result<(), DataFolderError> {
    test_folder(files, &Administrators::new(accounts), accounts, path)
}

#[cfg(windows)]
pub use windows_impl::{SystemAccounts, SystemFiles};

#[cfg(windows)]
mod windows_impl {
    use std::io;
    use std::path::Path;

    use super::{Accounts, Child, Files, Kind};
    use crate::harden::windows_acl;

    /// The accounts of this machine.
    pub struct SystemAccounts;

    impl Accounts for SystemAccounts {
        fn administrator_members(&self) -> io::Result<Vec<String>> {
            windows_acl::administrator_members()
        }

        fn groups_of(&self, sid: &str) -> Vec<String> {
            windows_acl::groups_of(sid).unwrap_or_default()
        }

        fn name_of(&self, sid: &str) -> Option<String> {
            windows_acl::name_of(sid)
        }
    }

    /// The real folders, owners and ACLs.
    pub struct SystemFiles {
        privileged: bool,
    }

    impl SystemFiles {
        /// For a process that is SYSTEM or an elevated administrator, which can give an owner away.
        pub fn new() -> io::Result<Self> {
            Ok(Self {
                privileged: windows_acl::is_privileged()?,
            })
        }

        /// Whether this process can give ownership to Administrators. Without that, `protect`
        /// leaves the owner and keeps the current user's access, as `harden_directory` does.
        pub fn privileged(&self) -> bool {
            self.privileged
        }

        /// As [`SystemFiles::new`] but saying whether the process is privileged, for the tests that
        /// run unelevated.
        pub fn with_privilege(privileged: bool) -> Self {
            Self { privileged }
        }
    }

    impl Files for SystemFiles {
        fn kind(&self, path: &Path) -> io::Result<Option<Kind>> {
            match std::fs::symlink_metadata(path) {
                Ok(metadata) => Ok(Some(windows_acl::kind_of(&metadata))),
                Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
                Err(e) => Err(e),
            }
        }

        fn owner(&self, path: &Path) -> io::Result<String> {
            windows_acl::owner(path)
        }

        fn protect(&self, path: &Path, container: bool) -> io::Result<()> {
            windows_acl::protect(path, container, self.privileged)
        }

        fn create_folder(&self, path: &Path) -> io::Result<()> {
            std::fs::create_dir(path)
        }

        fn children(&self, path: &Path) -> io::Result<Vec<Child>> {
            std::fs::read_dir(path)?
                .map(|entry| {
                    let entry = entry?;
                    Ok(Child {
                        name: entry.file_name().to_string_lossy().into_owned(),
                        kind: windows_acl::kind_of(&std::fs::symlink_metadata(entry.path())?),
                    })
                })
                .collect()
        }

        fn remove_link(&self, path: &Path, folder: bool) -> io::Result<()> {
            if folder {
                std::fs::remove_dir(path)
            } else {
                std::fs::remove_file(path)
            }
        }

        fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
            std::fs::rename(from, to)
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::cell::RefCell;
    use std::collections::HashMap;

    use super::*;

    const USER: &str = "S-1-5-21-1-2-3-1001";
    const OTHER: &str = "S-1-5-21-1-2-3-1002";
    const ADMIN_USER: &str = "S-1-5-21-1-2-3-1000";
    const ADMIN_GROUP: &str = "S-1-5-21-1-2-3-513";
    const DEPARTMENT: &str = "S-1-5-21-1-2-3-514";

    /// Named owners and memberships, as a machine could have them.
    pub(crate) struct FakeAccounts {
        pub members: Vec<String>,
        pub groups: HashMap<String, Vec<String>>,
        pub fail_members: bool,
    }

    impl FakeAccounts {
        pub(crate) fn nobody() -> Self {
            Self {
                members: Vec::new(),
                groups: HashMap::new(),
                fail_members: false,
            }
        }

        fn machine() -> Self {
            // ADMIN_USER and ADMIN_GROUP are listed in Administrators. OTHER belongs to
            // ADMIN_GROUP, and one more account belongs to Administrators itself. USER belongs
            // to groups that are not administrators.
            let mut groups = HashMap::new();
            groups.insert(OTHER.to_owned(), vec![ADMIN_GROUP.to_owned()]);
            groups.insert(
                "S-1-5-21-1-2-3-2000".to_owned(),
                vec![ADMINISTRATORS.to_owned()],
            );
            groups.insert(
                USER.to_owned(),
                vec![DEPARTMENT.to_owned(), "S-1-1-0".to_owned()],
            );
            Self {
                members: vec![ADMIN_USER.to_owned(), ADMIN_GROUP.to_owned()],
                groups,
                fail_members: false,
            }
        }
    }

    impl Accounts for FakeAccounts {
        fn administrator_members(&self) -> io::Result<Vec<String>> {
            if self.fail_members {
                return Err(io::Error::other("the group cannot be read"));
            }
            Ok(self.members.clone())
        }

        fn groups_of(&self, sid: &str) -> Vec<String> {
            self.groups.get(sid).cloned().unwrap_or_default()
        }

        fn name_of(&self, sid: &str) -> Option<String> {
            (sid == USER).then(|| "PC\\standard".to_owned())
        }
    }

    /// Real folders and links under a temp folder, with owners from a table and a log of the ACLs
    /// applied. An entry with no owner in the table belongs to `default_owner`.
    pub(crate) struct FakeFiles {
        pub owners: RefCell<HashMap<PathBuf, String>>,
        pub default_owner: String,
        pub protected: RefCell<Vec<(PathBuf, bool)>>,
    }

    impl FakeFiles {
        pub(crate) fn new(default_owner: &str) -> Self {
            Self {
                owners: RefCell::new(HashMap::new()),
                default_owner: default_owner.to_owned(),
                protected: RefCell::new(Vec::new()),
            }
        }

        pub(crate) fn own(&self, path: &Path, owner: &str) {
            self.owners
                .borrow_mut()
                .insert(path.to_path_buf(), owner.to_owned());
        }
    }

    pub(crate) fn kind_of(metadata: &std::fs::Metadata) -> Kind {
        #[cfg(windows)]
        {
            crate::harden::windows_acl::kind_of(metadata)
        }
        #[cfg(not(windows))]
        {
            if metadata.file_type().is_symlink() {
                Kind::Link {
                    folder: false, // a symbolic link is removed as a file on Unix
                }
            } else if metadata.is_dir() {
                Kind::Folder
            } else {
                Kind::File
            }
        }
    }

    impl Files for FakeFiles {
        fn kind(&self, path: &Path) -> io::Result<Option<Kind>> {
            match std::fs::symlink_metadata(path) {
                Ok(m) => Ok(Some(kind_of(&m))),
                Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
                Err(e) => Err(e),
            }
        }

        fn owner(&self, path: &Path) -> io::Result<String> {
            Ok(self
                .owners
                .borrow()
                .get(path)
                .cloned()
                .unwrap_or_else(|| self.default_owner.clone()))
        }

        fn protect(&self, path: &Path, container: bool) -> io::Result<()> {
            self.protected
                .borrow_mut()
                .push((path.to_path_buf(), container));
            self.own(path, ADMINISTRATORS);
            Ok(())
        }

        fn create_folder(&self, path: &Path) -> io::Result<()> {
            std::fs::create_dir(path)
        }

        fn children(&self, path: &Path) -> io::Result<Vec<Child>> {
            let mut children = std::fs::read_dir(path)?
                .map(|entry| {
                    let entry = entry?;
                    Ok(Child {
                        name: entry.file_name().to_string_lossy().into_owned(),
                        kind: kind_of(&std::fs::symlink_metadata(entry.path())?),
                    })
                })
                .collect::<io::Result<Vec<_>>>()?;
            children.sort_by(|a, b| a.name.cmp(&b.name));
            Ok(children)
        }

        fn remove_link(&self, path: &Path, folder: bool) -> io::Result<()> {
            if folder {
                std::fs::remove_dir(path)
            } else {
                std::fs::remove_file(path)
            }
        }

        fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
            std::fs::rename(from, to)
        }
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("koppel-data-{name}-{}", crate::random_hex(6)));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn run(
        path: &Path,
        accounts: &FakeAccounts,
        files: &FakeFiles,
    ) -> (Result<(), DataFolderError>, Vec<String>) {
        let mut said = Vec::new();
        let result = prepare(path, accounts, files, &mut |line| {
            said.push(line.to_owned())
        });
        (result, said)
    }

    // The administrator rules.

    fn admin(accounts: &FakeAccounts, sid: &str) -> bool {
        Administrators::new(accounts).is_administrator(sid).unwrap()
    }

    #[test]
    fn system_administrators_and_trusted_installer_are_administrators_with_no_lookup() {
        let mut accounts = FakeAccounts::nobody();
        accounts.fail_members = true; // proves the group is not read for these
        for sid in [SYSTEM, ADMINISTRATORS, TRUSTED_INSTALLER] {
            assert!(admin(&accounts, sid), "{sid}");
        }
    }

    #[test]
    fn a_direct_member_and_a_member_through_a_group_are_administrators() {
        let accounts = FakeAccounts::machine();
        assert!(admin(&accounts, ADMIN_USER), "direct member");
        assert!(admin(&accounts, ADMIN_GROUP), "a group in the group");
        assert!(admin(&accounts, OTHER), "member of a listed group");
        assert!(
            admin(&accounts, "S-1-5-21-1-2-3-2000"),
            "member of Administrators itself"
        );
    }

    #[test]
    fn a_standard_user_and_an_unknown_account_are_not() {
        let accounts = FakeAccounts::machine();
        assert!(!admin(&accounts, USER));
        assert!(!admin(&accounts, "S-1-1-0"), "Everyone");
        assert!(!admin(&accounts, "S-1-5-21-9-9-9-500"), "not resolvable");
    }

    #[test]
    fn an_unreadable_administrators_group_is_an_error_not_a_yes() {
        let mut accounts = FakeAccounts::machine();
        accounts.fail_members = true;
        assert!(
            Administrators::new(&accounts)
                .is_administrator(USER)
                .is_err()
        );
    }

    // The folder itself.

    #[test]
    fn a_missing_folder_is_created_and_protected() {
        let base = temp_dir("missing");
        let folder = base.join("Koppel");
        let files = FakeFiles::new(USER);
        let (result, said) = run(&folder, &FakeAccounts::machine(), &files);
        result.unwrap();
        assert!(folder.is_dir());
        assert_eq!(*files.protected.borrow(), vec![(folder.clone(), true)]);
        assert_eq!(files.owner(&folder).unwrap(), ADMINISTRATORS);
        assert_eq!(
            said.last().unwrap(),
            &format!(
                "{} is owned by Administrators and open to SYSTEM and Administrators only.",
                folder.display()
            )
        );
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn check_on_a_missing_folder_fails_and_creates_nothing() {
        let base = temp_dir("check-missing");
        let folder = base.join("Koppel");
        let error = check(&folder, &FakeAccounts::machine(), &FakeFiles::new(USER)).unwrap_err();
        assert_eq!(
            error.to_string(),
            format!("{} does not exist.", folder.display())
        );
        assert!(!folder.exists());
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn a_file_where_the_folder_should_be_is_refused_untouched() {
        let base = temp_dir("file");
        let file = base.join("Koppel");
        std::fs::write(&file, b"mine").unwrap();
        let files = FakeFiles::new(ADMINISTRATORS);
        let (result, _) = run(&file, &FakeAccounts::machine(), &files);
        let error = result.unwrap_err();
        assert!(matches!(error, DataFolderError::NotAFolder(_)));
        assert!(error.to_string().contains("is a file"), "{error}");
        assert_eq!(std::fs::read(&file).unwrap(), b"mine");
        assert!(files.protected.borrow().is_empty());
        assert!(check(&file, &FakeAccounts::machine(), &files).is_err());
        let _ = std::fs::remove_dir_all(base);
    }

    /// A link to a folder, made as a junction on Windows and a symbolic link elsewhere.
    pub(crate) fn make_folder_link(link: &Path, target: &Path) -> bool {
        #[cfg(windows)]
        {
            std::process::Command::new("cmd")
                .args(["/C", "mklink", "/J"])
                .arg(link)
                .arg(target)
                .output()
                .is_ok_and(|out| out.status.success())
        }
        #[cfg(not(windows))]
        {
            std::os::unix::fs::symlink(target, link).is_ok()
        }
    }

    #[test]
    fn a_link_where_the_folder_should_be_is_refused_and_its_target_untouched() {
        let base = temp_dir("link-root");
        let target = base.join("elsewhere");
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("keep.txt"), b"keep").unwrap();
        let link = base.join("Koppel");
        assert!(make_folder_link(&link, &target));
        let files = FakeFiles::new(ADMINISTRATORS);
        let (result, said) = run(&link, &FakeAccounts::machine(), &files);
        let error = result.unwrap_err();
        assert!(matches!(error, DataFolderError::Link(_)), "{error}");
        assert!(error.to_string().contains("junction or symbolic link"));
        assert!(said.is_empty() && files.protected.borrow().is_empty());
        assert!(target.join("keep.txt").exists());
        assert!(check(&link, &FakeAccounts::machine(), &files).is_err());
        let _ = std::fs::remove_dir(&link);
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn a_folder_owned_by_a_standard_user_is_refused_by_name_and_left_alone() {
        let base = temp_dir("user-owned");
        let folder = base.join("Koppel");
        std::fs::create_dir(&folder).unwrap();
        std::fs::write(folder.join("secrets.json"), b"x").unwrap();
        let files = FakeFiles::new(USER);
        let (result, _) = run(&folder, &FakeAccounts::machine(), &files);
        let error = result.unwrap_err();
        let text = error.to_string();
        assert!(
            text.contains("is owned by PC\\standard, who is not an administrator"),
            "{text}"
        );
        assert!(text.contains("Rename or delete the folder"), "{text}");
        assert!(files.protected.borrow().is_empty(), "no ACL was changed");
        assert!(folder.join("secrets.json").exists());
        assert!(check(&folder, &FakeAccounts::machine(), &files).is_err());
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn an_owner_with_no_name_is_shown_as_its_sid() {
        let base = temp_dir("sid-owned");
        let folder = base.join("Koppel");
        std::fs::create_dir(&folder).unwrap();
        let error = check(&folder, &FakeAccounts::machine(), &FakeFiles::new(OTHER));
        assert!(error.is_ok(), "OTHER is in a listed group");
        let error = check(
            &folder,
            &FakeAccounts::machine(),
            &FakeFiles::new("S-1-5-21-9-9-9-500"),
        )
        .unwrap_err();
        assert!(error.to_string().contains("owned by S-1-5-21-9-9-9-500,"));
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn check_accepts_an_administrator_owned_folder_and_changes_nothing() {
        let base = temp_dir("check-ok");
        let folder = base.join("Koppel");
        std::fs::create_dir(&folder).unwrap();
        std::fs::write(folder.join("a.json"), b"x").unwrap();
        let files = FakeFiles::new(ADMIN_USER);
        check(&folder, &FakeAccounts::machine(), &files).unwrap();
        assert!(files.protected.borrow().is_empty());
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn an_administrator_owned_folder_is_taken_over_down_the_tree() {
        let base = temp_dir("takeover");
        let folder = base.join("Koppel");
        let config = folder.join("config");
        std::fs::create_dir_all(&config).unwrap();
        std::fs::write(folder.join("driver-state.json"), b"{}").unwrap();
        std::fs::write(config.join("settings.json"), b"{}").unwrap();
        let files = FakeFiles::new(ADMIN_USER);
        let (result, said) = run(&folder, &FakeAccounts::machine(), &files);
        result.unwrap();
        let protected = files.protected.borrow();
        assert!(protected.contains(&(folder.clone(), true)));
        assert!(protected.contains(&(config.clone(), true)));
        assert!(protected.contains(&(folder.join("driver-state.json"), false)));
        assert!(protected.contains(&(config.join("settings.json"), false)));
        assert_eq!(protected.len(), 4);
        assert_eq!(said.len(), 1, "only the closing line: {said:?}");
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn a_link_inside_is_deleted_and_what_it_points_to_is_not() {
        let base = temp_dir("inner-link");
        let outside = base.join("outside");
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(outside.join("precious.txt"), b"p").unwrap();
        let folder = base.join("Koppel");
        std::fs::create_dir(&folder).unwrap();
        let link = folder.join("logs");
        assert!(make_folder_link(&link, &outside));
        let files = FakeFiles::new(ADMIN_USER);
        let (result, said) = run(&folder, &FakeAccounts::machine(), &files);
        result.unwrap();
        assert!(
            std::fs::symlink_metadata(&link).is_err(),
            "the link is gone"
        );
        assert!(outside.join("precious.txt").exists(), "the target is not");
        assert!(
            said.iter().any(|l| l.starts_with("Removing the link ")
                && l.ends_with("(it points somewhere else; Koppel does not use links).")),
            "{said:?}"
        );
        let protected = files.protected.borrow();
        assert!(
            protected
                .iter()
                .all(|(p, _)| !p.starts_with(&link) || *p == folder),
            "nothing behind the link was protected: {protected:?}"
        );
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn a_file_link_inside_is_deleted_too() {
        let base = temp_dir("inner-file-link");
        let outside = base.join("outside.txt");
        std::fs::write(&outside, b"p").unwrap();
        let folder = base.join("Koppel");
        std::fs::create_dir(&folder).unwrap();
        let link = folder.join("settings.json");
        #[cfg(unix)]
        let made = std::os::unix::fs::symlink(&outside, &link).is_ok();
        #[cfg(windows)]
        let made = std::os::windows::fs::symlink_file(&outside, &link).is_ok();
        if !made {
            // A symbolic link needs developer mode or elevation on Windows.
            let _ = std::fs::remove_dir_all(base);
            return;
        }
        let files = FakeFiles::new(ADMIN_USER);
        run(&folder, &FakeAccounts::machine(), &files).0.unwrap();
        assert!(std::fs::symlink_metadata(&link).is_err());
        assert_eq!(std::fs::read(&outside).unwrap(), b"p");
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn what_a_standard_user_owns_inside_is_renamed_aside_and_not_protected() {
        let base = temp_dir("aside");
        let folder = base.join("Koppel");
        let theirs = folder.join("planted");
        let their_file = folder.join("secrets.json");
        std::fs::create_dir_all(&theirs).unwrap();
        std::fs::write(theirs.join("inner.txt"), b"i").unwrap();
        std::fs::write(&their_file, b"s").unwrap();
        let ours = folder.join("config");
        std::fs::create_dir(&ours).unwrap();
        let files = FakeFiles::new(ADMIN_USER);
        files.own(&theirs, USER);
        files.own(&their_file, USER);
        let (result, said) = run(&folder, &FakeAccounts::machine(), &files);
        result.unwrap();

        assert!(!theirs.exists() && !their_file.exists());
        let names: Vec<String> = std::fs::read_dir(&folder)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        let aside_dir = names
            .iter()
            .find(|n| n.starts_with("planted.not-admin-owned-"))
            .expect("the folder was renamed");
        assert!(
            names
                .iter()
                .any(|n| n.starts_with("secrets.json.not-admin-owned-")),
            "{names:?}"
        );
        assert!(folder.join(aside_dir).join("inner.txt").exists());
        // Its ACL was left alone: it is neither protected nor re-owned.
        let protected = files.protected.borrow();
        assert!(protected.iter().all(|(p, _)| p == &folder || p == &ours));
        assert_eq!(files.owner(&theirs).unwrap(), USER);
        assert!(
            said.iter().any(|l| l.contains(
                "planted is not owned by an administrator. Renaming it to planted.not-admin-owned-"
            ) && l.ends_with("; Koppel does not use it.")),
            "{said:?}"
        );
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn something_already_renamed_aside_is_left_where_it_is() {
        let base = temp_dir("already-aside");
        let folder = base.join("Koppel");
        let aside = folder.join("old.not-admin-owned-20250101000000000");
        std::fs::create_dir_all(&aside).unwrap();
        let files = FakeFiles::new(ADMIN_USER);
        files.own(&aside, USER);
        run(&folder, &FakeAccounts::machine(), &files).0.unwrap();
        assert!(aside.exists(), "not renamed again");
        assert!(
            !files.protected.borrow().iter().any(|(p, _)| p == &aside),
            "and not protected"
        );
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn a_second_run_changes_nothing_more() {
        let base = temp_dir("again");
        let folder = base.join("Koppel");
        std::fs::create_dir_all(folder.join("config")).unwrap();
        let files = FakeFiles::new(ADMIN_USER);
        run(&folder, &FakeAccounts::machine(), &files).0.unwrap();
        let (result, said) = run(&folder, &FakeAccounts::machine(), &files);
        result.unwrap();
        assert_eq!(said.len(), 1, "{said:?}");
        let _ = std::fs::remove_dir_all(base);
    }
}

/// The same rules on real folders, real ACLs and the real accounts of this machine. They run
/// unelevated: a process that cannot give an owner away keeps its own full control (the test
/// folders are the current user's), and a folder cannot be made SYSTEM's, so the owners the rules
/// turn on are named through `Accounts` and an owner table, as in the tests above.
#[cfg(all(test, windows))]
mod windows_tests {
    use std::cell::RefCell;

    use windows::Win32::Foundation::{HLOCAL, LocalFree};
    use windows::Win32::Security::Authorization::{GetNamedSecurityInfoW, SE_FILE_OBJECT};
    use windows::Win32::Security::{
        ACCESS_ALLOWED_ACE, ACL, ACL_SIZE_INFORMATION, AclSizeInformation,
        DACL_SECURITY_INFORMATION, GetAce, GetAclInformation, GetSecurityDescriptorControl,
        PSECURITY_DESCRIPTOR, PSID,
    };

    use super::tests::{FakeAccounts, make_folder_link};
    use super::*;
    use crate::harden::windows_acl::{self, Sid};

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("koppel-data-w-{name}-{}", crate::random_hex(6)));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn me() -> String {
        windows_acl::sid_text(Sid::current_user().unwrap().psid()).unwrap()
    }

    /// Accounts where only the current user is an administrator.
    fn only_me() -> FakeAccounts {
        let mut accounts = FakeAccounts::nobody();
        accounts.members.push(me());
        accounts
    }

    struct Acl {
        protected: bool,
        /// SID, mask, inheritance flags and type of each entry.
        entries: Vec<(String, u32, u8, u8)>,
    }

    fn read_acl(path: &Path) -> Acl {
        let wide: Vec<u16> = {
            use std::os::windows::ffi::OsStrExt;
            path.as_os_str().encode_wide().chain([0]).collect()
        };
        let mut acl: *mut ACL = std::ptr::null_mut();
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        // SAFETY: test-only reads of an ACL; the descriptor is freed at the end.
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
            let mut size = ACL_SIZE_INFORMATION::default();
            GetAclInformation(
                acl,
                (&mut size as *mut ACL_SIZE_INFORMATION).cast(),
                size_of::<ACL_SIZE_INFORMATION>() as u32,
                AclSizeInformation,
            )
            .unwrap();
            let mut entries = Vec::new();
            for index in 0..size.AceCount {
                let mut ace = std::ptr::null_mut();
                GetAce(acl, index, &mut ace).unwrap();
                let ace = &*(ace as *const ACCESS_ALLOWED_ACE);
                let start = &ace.SidStart as *const u32 as *const u8;
                let sid = windows_acl::sid_text(PSID(start as *mut _)).unwrap();
                entries.push((sid, ace.Mask, ace.Header.AceFlags, ace.Header.AceType));
            }
            LocalFree(Some(HLOCAL(descriptor.0)));
            Acl {
                protected: control & 0x1000 != 0,
                entries,
            }
        }
    }

    /// Full control for SYSTEM, Administrators and this user, nothing else, nothing inherited.
    fn assert_protected(path: &Path, inherits: bool) {
        let acl = read_acl(path);
        assert!(acl.protected, "{}: SE_DACL_PROTECTED", path.display());
        let expected_flags = if inherits { 0x03 } else { 0 }; // OBJECT_INHERIT | CONTAINER_INHERIT
        let mut who: Vec<&str> = acl.entries.iter().map(|e| e.0.as_str()).collect();
        who.sort_unstable();
        let mut want = vec![SYSTEM.to_owned(), ADMINISTRATORS.to_owned(), me()];
        want.sort_unstable();
        assert_eq!(who, want, "{}", path.display());
        for (sid, mask, flags, kind) in &acl.entries {
            assert_eq!(*kind, 0, "{sid}: allow only");
            assert_eq!(*mask, 0x1F01FF, "{sid}: full control");
            assert_eq!(*flags & 0x03, expected_flags, "{sid}: inheritance");
        }
    }

    fn run_real(
        path: &Path,
        accounts: &dyn Accounts,
        files: &dyn Files,
    ) -> (Result<(), DataFolderError>, Vec<String>) {
        let mut said = Vec::new();
        let result = prepare(path, accounts, files, &mut |line| {
            said.push(line.to_owned())
        });
        (result, said)
    }

    /// The real files, with the owner of named paths told apart (nothing unelevated can give a
    /// path away).
    struct Overridden {
        inner: SystemFiles,
        owners: Vec<(PathBuf, String)>,
        protected: RefCell<Vec<PathBuf>>,
    }

    impl Files for Overridden {
        fn kind(&self, path: &Path) -> io::Result<Option<Kind>> {
            self.inner.kind(path)
        }
        fn owner(&self, path: &Path) -> io::Result<String> {
            match self.owners.iter().find(|(p, _)| p == path) {
                Some((_, owner)) => Ok(owner.clone()),
                None => self.inner.owner(path),
            }
        }
        fn protect(&self, path: &Path, container: bool) -> io::Result<()> {
            self.protected.borrow_mut().push(path.to_path_buf());
            self.inner.protect(path, container)
        }
        fn create_folder(&self, path: &Path) -> io::Result<()> {
            self.inner.create_folder(path)
        }
        fn children(&self, path: &Path) -> io::Result<Vec<Child>> {
            self.inner.children(path)
        }
        fn remove_link(&self, path: &Path, folder: bool) -> io::Result<()> {
            self.inner.remove_link(path, folder)
        }
        fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
            self.inner.rename(from, to)
        }
    }

    #[test]
    fn the_real_owner_of_a_new_folder_is_read_as_the_current_user_or_administrators() {
        let dir = temp_dir("owner");
        let owner = SystemFiles::with_privilege(false).owner(&dir).unwrap();
        assert!(owner == me() || owner == ADMINISTRATORS, "{owner}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_missing_folder_is_created_and_its_whole_tree_gets_the_protected_acl() {
        let base = temp_dir("tree");
        let folder = base.join("Koppel");
        let files = SystemFiles::with_privilege(false);
        run_real(&folder, &only_me(), &files).0.unwrap();
        assert_protected(&folder, true);

        // Now a tree under an existing folder.
        let inner = folder.join("config").join("deeper");
        std::fs::create_dir_all(&inner).unwrap();
        std::fs::write(folder.join("driver-state.json"), b"{}").unwrap();
        std::fs::write(inner.join("secrets.json"), b"s").unwrap();
        let (result, said) = run_real(&folder, &only_me(), &files);
        result.unwrap();
        assert_protected(&folder, true);
        assert_protected(&folder.join("config"), true);
        assert_protected(&inner, true);
        assert_protected(&folder.join("driver-state.json"), false);
        assert_protected(&inner.join("secrets.json"), false);
        assert_eq!(said.len(), 1, "{said:?}");
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn a_folder_the_current_user_owns_is_refused_when_that_user_is_no_administrator() {
        let base = temp_dir("refused");
        let folder = base.join("Koppel");
        std::fs::create_dir(&folder).unwrap();
        std::fs::write(folder.join("keys.bin"), b"k").unwrap();
        let before = read_acl(&folder);
        let files = SystemFiles::with_privilege(false);
        let nobody = FakeAccounts::nobody();
        if files.owner(&folder).unwrap() == ADMINISTRATORS {
            // A machine that gives new folders to the Administrators group: nothing to refuse.
            let _ = std::fs::remove_dir_all(base);
            return;
        }
        let error = run_real(&folder, &nobody, &files).0.unwrap_err();
        assert!(
            matches!(&error, DataFolderError::Owner { owner, .. } if !owner.is_empty()),
            "{error}"
        );
        assert!(error.to_string().contains("who is not an administrator"));
        let after = read_acl(&folder);
        assert_eq!(before.protected, after.protected, "the ACL was not touched");
        assert_eq!(before.entries, after.entries);
        assert!(check(&folder, &nobody, &files).is_err());
        assert!(check(&folder, &only_me(), &files).is_ok());
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn a_real_junction_inside_is_deleted_and_its_target_survives() {
        let base = temp_dir("junction");
        let outside = base.join("outside");
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(outside.join("precious.txt"), b"p").unwrap();
        let folder = base.join("Koppel");
        std::fs::create_dir(&folder).unwrap();
        let link = folder.join("logs");
        assert!(make_folder_link(&link, &outside));
        let files = SystemFiles::with_privilege(false);
        let (result, said) = run_real(&folder, &only_me(), &files);
        result.unwrap();
        assert!(std::fs::symlink_metadata(&link).is_err());
        assert_eq!(std::fs::read(outside.join("precious.txt")).unwrap(), b"p");
        assert!(
            said.iter().any(|l| l.starts_with("Removing the link ")),
            "{said:?}"
        );
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn a_real_junction_where_the_folder_should_be_is_refused() {
        let base = temp_dir("junction-root");
        let outside = base.join("outside");
        std::fs::create_dir(&outside).unwrap();
        let link = base.join("Koppel");
        assert!(make_folder_link(&link, &outside));
        let files = SystemFiles::with_privilege(false);
        let error = run_real(&link, &only_me(), &files).0.unwrap_err();
        assert!(matches!(error, DataFolderError::Link(_)), "{error}");
        assert!(matches!(
            check(&link, &only_me(), &files).unwrap_err(),
            DataFolderError::Link(_)
        ));
        let _ = std::fs::remove_dir(&link);
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn a_real_file_symbolic_link_inside_is_deleted_when_this_user_may_make_one() {
        let base = temp_dir("filelink");
        let outside = base.join("outside.txt");
        std::fs::write(&outside, b"p").unwrap();
        let folder = base.join("Koppel");
        std::fs::create_dir(&folder).unwrap();
        let link = folder.join("settings.json");
        if std::os::windows::fs::symlink_file(&outside, &link).is_err() {
            // Needs developer mode or elevation.
            let _ = std::fs::remove_dir_all(base);
            return;
        }
        run_real(&folder, &only_me(), &SystemFiles::with_privilege(false))
            .0
            .unwrap();
        assert!(std::fs::symlink_metadata(&link).is_err());
        assert_eq!(std::fs::read(&outside).unwrap(), b"p");
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn a_real_item_owned_by_a_standard_user_is_renamed_aside_and_left_unprotected() {
        let base = temp_dir("aside");
        let folder = base.join("Koppel");
        let theirs = folder.join("planted");
        let ours = folder.join("config");
        std::fs::create_dir_all(&theirs).unwrap();
        std::fs::create_dir_all(&ours).unwrap();
        std::fs::write(theirs.join("inner.txt"), b"i").unwrap();
        let files = Overridden {
            inner: SystemFiles::with_privilege(false),
            owners: vec![(theirs.clone(), "S-1-5-21-1-2-3-1001".to_owned())],
            protected: RefCell::new(Vec::new()),
        };
        let (result, said) = run_real(&folder, &only_me(), &files);
        result.unwrap();
        assert!(!theirs.exists());
        let aside = std::fs::read_dir(&folder)
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| {
                p.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("planted.not-admin-owned-")
            })
            .expect("renamed aside");
        assert!(aside.join("inner.txt").exists());
        // Windows pushes the parent's inheritable entries down on its own, so the ACL text can
        // change; what the run must not do is protect it or give it away.
        assert!(
            files
                .protected
                .borrow()
                .iter()
                .all(|p| !p.starts_with(folder.join("planted")))
        );
        assert_eq!(
            SystemFiles::with_privilege(false).owner(&aside).unwrap(),
            me(),
            "still its owner's"
        );
        assert_protected(&ours, true);
        assert!(
            said.iter()
                .any(|l| l.contains("planted is not owned by an administrator"))
        );
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn a_file_where_the_real_folder_should_be_is_refused() {
        let base = temp_dir("file");
        let file = base.join("Koppel");
        std::fs::write(&file, b"mine").unwrap();
        let files = SystemFiles::with_privilege(false);
        let error = run_real(&file, &only_me(), &files).0.unwrap_err();
        assert!(matches!(error, DataFolderError::NotAFolder(_)), "{error}");
        assert_eq!(std::fs::read(&file).unwrap(), b"mine");
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn the_machines_own_accounts_answer_as_the_rules_expect() {
        let accounts = SystemAccounts;
        let administrators = Administrators::new(&accounts);
        for sid in [SYSTEM, ADMINISTRATORS, TRUSTED_INSTALLER] {
            assert!(administrators.is_administrator(sid).unwrap(), "{sid}");
        }
        assert!(
            !administrators.is_administrator("S-1-1-0").unwrap(),
            "Everyone"
        );
        assert!(
            !administrators
                .is_administrator("S-1-5-21-111-222-333-9999")
                .unwrap(),
            "an account that does not exist"
        );
        let members = accounts.administrator_members().unwrap();
        assert!(!members.is_empty(), "the built-in administrator is in it");
        assert!(
            members
                .iter()
                .all(|m| m.starts_with("S-1-5-") || m.starts_with("S-1-12-1-")),
            "{members:?}"
        );
        assert!(
            accounts
                .groups_of(SYSTEM)
                .iter()
                .all(|g| g.starts_with("S-1-")),
            "no panic and well-formed SIDs"
        );
        assert!(
            accounts
                .name_of(SYSTEM)
                .is_some_and(|n| n.to_uppercase().contains("SYSTEM"))
        );
        assert!(accounts.name_of("S-1-5-21-111-222-333-9999").is_none());
        // An elevated administrator is an administrator by the group list as well.
        if windows_acl::is_privileged().unwrap() {
            assert!(administrators.is_administrator(&me()).unwrap());
        }
    }
}
