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
    // SAFETY: Every pointer passed below either targets initialized writable storage or remains
    // valid for the duration of the call. Owned wrappers close API-allocated resources exactly once.
    unsafe {
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return Err(std::io::Error::last_os_error());
        }
        let token = OwnedHandle(token);
        let mut required = 0;
        GetTokenInformation(
            token.0,
            TokenUser,
            null_mut(),
            /*tokeninformationlength*/ 0,
            &mut required,
        );
        if required == 0 {
            return Err(std::io::Error::last_os_error());
        }
        let words = (required as usize).div_ceil(std::mem::size_of::<usize>());
        let mut token_information = vec![0_usize; words];
        if GetTokenInformation(
            token.0,
            TokenUser,
            token_information.as_mut_ptr().cast::<c_void>(),
            required,
            &mut required,
        ) == 0
        {
            return Err(std::io::Error::last_os_error());
        }
        let token_user = &*(token_information.as_ptr().cast::<TOKEN_USER>());
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
        let status = SetEntriesInAclW(/*ccountofexplicitentries*/ 1, &entry, null(), &mut acl);
        if status != ERROR_SUCCESS {
            return Err(std::io::Error::from_raw_os_error(status as i32));
        }
        let acl = OwnedAcl(acl);
        let mut wide_path = path
            .as_os_str()
            .encode_wide()
            .chain([0])
            .collect::<Vec<_>>();
        let status = SetNamedSecurityInfoW(
            wide_path.as_mut_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            null_mut(),
            null_mut(),
            acl.0,
            null(),
        );
        if status != ERROR_SUCCESS {
            return Err(std::io::Error::from_raw_os_error(status as i32));
        }
    }
    Ok(())
}
