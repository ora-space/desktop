use std::ffi::c_void;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::{CloseHandle, ERROR_SUCCESS, GENERIC_ALL, HANDLE, LocalFree};
use windows_sys::Win32::Security::Authorization::{
    EXPLICIT_ACCESS_W, NO_MULTIPLE_TRUSTEE, SE_FILE_OBJECT, SET_ACCESS, SetEntriesInAclW,
    SetNamedSecurityInfoW, TRUSTEE_IS_SID, TRUSTEE_IS_USER, TRUSTEE_W,
};
use windows_sys::Win32::Security::{
    ACL, DACL_SECURITY_INFORMATION, GetTokenInformation, PROTECTED_DACL_SECURITY_INFORMATION,
    SUB_CONTAINERS_AND_OBJECTS_INHERIT, TOKEN_QUERY, TOKEN_USER, TokenUser,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

/// Selects whether the user-only access rule should flow into child files and directories.
pub(super) enum AccessControlTarget {
    Directory,
    File,
}

struct OwnedHandle(HANDLE);

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        // SAFETY: OpenProcessToken returned this non-null owned handle and Drop runs once.
        unsafe {
            CloseHandle(self.0);
        }
    }
}

struct OwnedAcl(*mut ACL);

impl Drop for OwnedAcl {
    fn drop(&mut self) {
        // SAFETY: SetEntriesInAclW allocates the ACL with LocalAlloc for LocalFree ownership.
        unsafe {
            LocalFree(self.0.cast());
        }
    }
}

/// Replaces inherited Windows permissions with one protected current-user-only DACL.
pub(super) fn restrict_to_current_user(
    path: &Path,
    target: AccessControlTarget,
) -> std::io::Result<()> {
    let mut token = null_mut();
    // SAFETY: GetCurrentProcess is a pseudo-handle; `&mut token` is the out-parameter the API
    // writes on success, and OwnedHandle closes it exactly once.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(windows_step_error("OpenProcessToken"));
    }
    let token = OwnedHandle(token);
    let mut required = 0;
    // SAFETY: The first probe must pass a null TokenInformation with TokenInformationLength 0 so
    // the API reports the required byte count through ReturnLength.
    unsafe {
        GetTokenInformation(
            token.0,
            TokenUser,
            null_mut(),
            /*TokenInformationLength*/ 0,
            &mut required,
        );
    }
    if required == 0 {
        return Err(windows_step_error("GetTokenInformation size probe"));
    }
    let words = (required as usize).div_ceil(std::mem::size_of::<usize>());
    let mut token_information = vec![0_usize; words];
    // SAFETY: `token_information` is a writable buffer of at least `required` bytes and stays
    // alive for the call; ReturnLength points at a local u32.
    if unsafe {
        GetTokenInformation(
            token.0,
            TokenUser,
            token_information.as_mut_ptr().cast::<c_void>(),
            required,
            &mut required,
        )
    } == 0
    {
        return Err(windows_step_error("GetTokenInformation"));
    }
    // SAFETY: GetTokenInformation succeeded with TokenUser, so the buffer starts with TOKEN_USER
    // and the SID pointer it contains remains valid until `token_information` is dropped.
    let token_user = unsafe { &*token_information.as_ptr().cast::<TOKEN_USER>() };
    let entry = EXPLICIT_ACCESS_W {
        grfAccessPermissions: GENERIC_ALL,
        grfAccessMode: SET_ACCESS,
        grfInheritance: match target {
            AccessControlTarget::Directory => SUB_CONTAINERS_AND_OBJECTS_INHERIT,
            AccessControlTarget::File => 0,
        },
        Trustee: TRUSTEE_W {
            pMultipleTrustee: null_mut(),
            MultipleTrusteeOperation: NO_MULTIPLE_TRUSTEE,
            TrusteeForm: TRUSTEE_IS_SID,
            TrusteeType: TRUSTEE_IS_USER,
            ptstrName: token_user.User.Sid.cast::<u16>(),
        },
    };
    let mut acl = null_mut();
    // SAFETY: `entry` and `acl` outlive the call; OldAcl is null because this DACL is constructed
    // from one explicit entry rather than merged with an existing ACL.
    let status = unsafe {
        SetEntriesInAclW(/*cCountOfExplicitEntries*/ 1, &entry, null(), &mut acl)
    };
    if status != ERROR_SUCCESS {
        return Err(windows_status_error("SetEntriesInAclW", status));
    }
    let acl = OwnedAcl(acl);
    let mut wide_path = path
        .as_os_str()
        .encode_wide()
        .chain([0])
        .collect::<Vec<_>>();
    // SAFETY: `wide_path` is a NUL-terminated Win32 path, `acl.0` is the ACL SetEntriesInAclW
    // allocated, and owner/group/SACL pointers are null because this call only replaces the DACL.
    let status = unsafe {
        SetNamedSecurityInfoW(
            wide_path.as_mut_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            null_mut(),
            null_mut(),
            acl.0,
            null(),
        )
    };
    if status != ERROR_SUCCESS {
        return Err(windows_status_error("SetNamedSecurityInfoW", status));
    }
    Ok(())
}

/// Names the Win32 call that failed so permission-setup errors are diagnosable.
fn windows_step_error(step: &str) -> std::io::Error {
    std::io::Error::new(
        std::io::Error::last_os_error().kind(),
        format!("{step}: {}", std::io::Error::last_os_error()),
    )
}

/// Names a Win32 call that reports its failure as a status code instead of `GetLastError`.
fn windows_status_error(step: &str, status: u32) -> std::io::Error {
    let source = std::io::Error::from_raw_os_error(status as i32);
    std::io::Error::new(source.kind(), format!("{step}: {source}"))
}
