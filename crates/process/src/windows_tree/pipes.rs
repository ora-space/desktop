use super::{owned_handle, wide_nul};
use std::ffi::OsStr;
use std::io;
use std::mem::{size_of, zeroed};
use std::os::windows::io::{AsRawHandle, IntoRawHandle, OwnedHandle};
use std::ptr::{null, null_mut};
use tokio::net::windows::named_pipe::NamedPipeServer;
use windows_sys::Win32::Foundation::{
    ERROR_IO_PENDING, ERROR_PIPE_CONNECTED, GENERIC_READ, GENERIC_WRITE, GetLastError, HANDLE,
    LocalFree,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED,
    OPEN_EXISTING, PIPE_ACCESS_INBOUND, PIPE_ACCESS_OUTBOUND,
};
use windows_sys::Win32::System::IO::{GetOverlappedResult, OVERLAPPED};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS,
    PIPE_TYPE_BYTE, PIPE_WAIT,
};
use windows_sys::Win32::System::Threading::CreateEventW;

const PIPE_BUFFER_BYTES: u32 = 64 * 1024;

pub(super) struct PipeSecurityDescriptor {
    descriptor: PSECURITY_DESCRIPTOR,
}

impl PipeSecurityDescriptor {
    /// Restricts pipe access to SYSTEM and the token that owns the newly created pipe object.
    pub(super) fn current_owner_and_system() -> io::Result<Self> {
        // OW is the SDDL Owner Rights SID. The kernel assigns the new object's owner from the
        // current token, avoiding a username/SID lookup race while still excluding other users.
        let sddl = wide_nul(OsStr::new("D:P(A;;GA;;;SY)(A;;GA;;;OW)"), "pipe DACL")?;
        let mut descriptor = null_mut();
        // SAFETY: the SDDL buffer is NUL terminated and the returned allocation is freed by Drop.
        let success = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                null_mut(),
            )
        };
        if success == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self { descriptor })
    }

    fn security_attributes(&self, inheritable: bool) -> SECURITY_ATTRIBUTES {
        SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: self.descriptor,
            bInheritHandle: i32::from(inheritable),
        }
    }
}

impl Drop for PipeSecurityDescriptor {
    fn drop(&mut self) {
        // SAFETY: this allocation came from ConvertStringSecurityDescriptor... and is non-null.
        unsafe {
            LocalFree(self.descriptor);
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) enum PipeDirection {
    HostWrites,
    HostReads,
}

/// Creates a connected local byte-mode pipe with an overlapped Host end and synchronous child end.
pub(super) fn create_pipe_pair(
    name: &str,
    direction: PipeDirection,
    descriptor: &PipeSecurityDescriptor,
) -> io::Result<(OwnedHandle, OwnedHandle)> {
    let name = wide_nul(OsStr::new(name), "pipe name")?;
    let server_access = match direction {
        PipeDirection::HostWrites => PIPE_ACCESS_OUTBOUND,
        PipeDirection::HostReads => PIPE_ACCESS_INBOUND,
    } | FILE_FLAG_OVERLAPPED
        | FILE_FLAG_FIRST_PIPE_INSTANCE;
    let pipe_mode = PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS;
    let server_attributes = descriptor.security_attributes(false);

    // SAFETY: all pointers reference live immutable data for the duration of the call.
    let server_raw = unsafe {
        CreateNamedPipeW(
            name.as_ptr(),
            server_access,
            pipe_mode,
            1,
            PIPE_BUFFER_BYTES,
            PIPE_BUFFER_BYTES,
            0,
            &server_attributes,
        )
    };
    let server = owned_handle(server_raw, "CreateNamedPipeW")?;

    let child_access = match direction {
        PipeDirection::HostWrites => GENERIC_READ,
        PipeDirection::HostReads => GENERIC_WRITE,
    };
    let child_attributes = descriptor.security_attributes(true);
    // SAFETY: the child handle is opened synchronously and explicitly marked inheritable.
    let child_raw = unsafe {
        CreateFileW(
            name.as_ptr(),
            child_access,
            0,
            &child_attributes,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            null_mut(),
        )
    };
    let child = owned_handle(child_raw, "CreateFileW named-pipe client")?;
    finish_pipe_connection(server.as_raw_handle() as HANDLE)?;
    Ok((server, child))
}

/// Completes ConnectNamedPipe before CreateProcess so startup has no unresolved pipe state.
fn finish_pipe_connection(server: HANDLE) -> io::Result<()> {
    // SAFETY: this creates a private non-inheritable manual-reset event owned by this scope.
    let event = owned_handle(
        unsafe { CreateEventW(null(), 1, 0, null()) },
        "CreateEventW for ConnectNamedPipe",
    )?;
    let mut overlapped: OVERLAPPED = unsafe { zeroed() };
    overlapped.hEvent = event.as_raw_handle() as HANDLE;

    // SAFETY: the OVERLAPPED and its event remain alive until completion is observed.
    let connected = unsafe { ConnectNamedPipe(server, &mut overlapped) };
    if connected != 0 {
        return Ok(());
    }
    match unsafe { GetLastError() } {
        ERROR_PIPE_CONNECTED => Ok(()),
        ERROR_IO_PENDING => {
            let mut transferred = 0;
            // SAFETY: the child endpoint is already open, so this only converges the connection.
            let success = unsafe { GetOverlappedResult(server, &overlapped, &mut transferred, 1) };
            if success == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        }
        _ => Err(io::Error::last_os_error()),
    }
}

/// Generates a cryptographically unpredictable pipe namespace component.
pub(super) fn pipe_nonce() -> io::Result<String> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes)
        .map_err(|error| io::Error::other(format!("failed to create pipe nonce: {error}")))?;
    let mut nonce = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write;
        write!(&mut nonce, "{byte:02x}")
            .map_err(|error| io::Error::other(format!("failed to encode pipe nonce: {error}")))?;
    }
    Ok(nonce)
}

/// Transfers an already-connected overlapped server handle into Tokio's IOCP registration.
pub(super) fn named_pipe_server(handle: OwnedHandle) -> io::Result<NamedPipeServer> {
    let raw = handle.into_raw_handle();
    // SAFETY: ownership of the unique overlapped named-pipe handle is transferred to Tokio.
    unsafe { NamedPipeServer::from_raw_handle(raw) }
}
