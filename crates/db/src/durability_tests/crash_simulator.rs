//! A power-cut simulator for SQLite durability tests.
//!
//! Real power loss cannot be reproduced inside a test process: the operating
//! system's page cache survives process death, so killing connections only
//! proves crash safety, not power-loss safety. The missing piece is that the
//! machine itself forgets every write the application never asked it to fsync.
//! This module supplies exactly that at the VFS layer: every file operation is
//! delegated to the platform VFS unchanged, the simulator remembers what each
//! named file held at its last successful sync, and "pulling the plug" makes
//! every later open serve only that synced content. Handles that were live
//! when the plug was pulled fail their I/O like the handles of a dead process,
//! which also prevents their close-time checkpoints from resurrecting unsynced
//! data. Under `synchronous = NORMAL` the write-ahead log is never synced at
//! commit, so a cut rolls acknowledged transactions back; under
//! `synchronous = FULL` every commit syncs the WAL and survives the cut.

use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_void};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use rusqlite::ffi;

/// Distinguishes simulators so parallel tests each get their own machine.
static NEXT_SIMULATOR_ID: AtomicU32 = AtomicU32::new(1);

/// Everything the simulated machine remembers between power-on and power-cut.
struct MachineState {
    /// Flips once when the plug is pulled; never flips back.
    crashed: AtomicBool,
    /// File content as of its last successful sync, by path. This is exactly
    /// what a power cut leaves behind on a real machine.
    durable: Mutex<HashMap<String, Vec<u8>>>,
    /// The world after the cut: content shared per path so every connection of
    /// a reopened pool observes one consistent machine.
    after_crash: Mutex<AfterCrash>,
}

/// Memory-backed state that only exists once the machine has come back up.
struct AfterCrash {
    files: HashMap<String, Arc<Mutex<Vec<u8>>>>,
    /// Wal-index regions per path. Pointers into the boxed slices stay stable
    /// even while the outer vectors grow, which the shared-memory contract of
    /// `xShmMap` requires.
    shm: HashMap<String, Vec<Box<[u64]>>>,
}

impl MachineState {
    /// Reports whether the plug has already been pulled.
    fn is_crashed(&self) -> bool {
        self.crashed.load(Ordering::SeqCst)
    }

    /// Pulls the plug: unsynced writes are gone and live handles are dead.
    fn crash(&self) {
        self.crashed.store(true, Ordering::SeqCst);
    }

    /// Reads what the platform file currently holds, treating an absent file as
    /// empty the way a freshly created file reads back.
    fn current_content(path: &str) -> Vec<u8> {
        std::fs::read(path).unwrap_or_default()
    }

    /// Records the file's current content as the durability baseline, keeping
    /// any baseline a previous sync of this machine already established.
    fn seed_durable(&self, path: &str) {
        let mut durable = self.durable.lock().expect("durable map lock");
        durable
            .entry(path.to_owned())
            .or_insert_with(|| Self::current_content(path));
    }

    /// Refreshes the durability baseline after a successful sync: everything
    /// the file holds right now has reached stable storage.
    fn mark_durable(&self, path: &str) {
        self.durable
            .lock()
            .expect("durable map lock")
            .insert(path.to_owned(), Self::current_content(path));
    }

    /// Returns the post-crash content for one path, seeding it from what the
    /// power cut preserved the first time it is touched.
    fn memory_file(&self, path: &str) -> Arc<Mutex<Vec<u8>>> {
        let mut after_crash = self.after_crash.lock().expect("post-crash lock");
        after_crash
            .files
            .entry(path.to_owned())
            .or_insert_with(|| {
                let preserved = self
                    .durable
                    .lock()
                    .expect("durable map lock")
                    .get(path)
                    .cloned()
                    .unwrap_or_default();
                Arc::new(Mutex::new(preserved))
            })
            .clone()
    }

    /// Resolves one wal-index region, following the platform contract: a
    /// missing region with `extend` unset maps to a null pointer and success,
    /// which is what makes SQLite fall back to rebuilding the wal-index from
    /// the recovered WAL instead of failing the open.
    fn shm_region(
        &self,
        path: &str,
        index: usize,
        size: usize,
        extend: bool,
        out: *mut *mut c_void,
    ) {
        let mut after_crash = self.after_crash.lock().expect("post-crash lock");
        let regions = after_crash.shm.entry(path.to_owned()).or_default();
        if regions.len() > index {
            unsafe { *out = regions[index].as_ptr().cast::<c_void>().cast_mut() };
            return;
        }
        if extend && regions.len() == index {
            regions.push(vec![0u64; size.div_ceil(8)].into_boxed_slice());
            unsafe { *out = regions[index].as_ptr().cast::<c_void>().cast_mut() };
        }
    }
}

/// The registered VFS plus the handle a test uses to drive its machine.
#[repr(C)]
struct CrashVfs {
    /// Must stay first: SQLite treats the leading bytes as `sqlite3_vfs`.
    base: ffi::sqlite3_vfs,
    /// The platform VFS every live operation is delegated to.
    underlying: *mut ffi::sqlite3_vfs,
    /// Registered name, referenced by the `vfs=` URI parameter.
    name: CString,
    state: Arc<MachineState>,
    /// Method table shared by every file this VFS opens.
    io: ffi::sqlite3_io_methods,
}

// The VFS is registered once and then only read; all mutation happens through
// the mutex-protected `MachineState`.
unsafe impl Sync for CrashVfs {}

/// One opened file. SQLite allocates the memory and calls `xClose` exactly
/// once per successful `xOpen`, so the Rust-owned fields are initialized in
/// `xOpen` and dropped in `xClose`.
#[repr(C)]
struct CrashFile {
    /// Must stay first: SQLite treats every `sqlite3_file` as this header.
    base: ffi::sqlite3_file,
    state: Arc<MachineState>,
    /// Empty for SQLite's unnamed temporary files, which hold no durable state.
    path: String,
    /// The platform file this handle wraps; null for post-crash memory files.
    underlying: *mut ffi::sqlite3_file,
    /// Backing allocation of `underlying`; emptied once it is closed.
    underlying_storage: Vec<u64>,
    /// Shared post-crash content; absent while the machine is powered on.
    memory: Option<Arc<Mutex<Vec<u8>>>>,
}

impl CrashFile {
    /// Reports whether this handle still wraps a platform file.
    fn wraps_platform_file(&self) -> bool {
        !self.underlying.is_null()
    }
}

/// Test-facing handle for one simulated machine.
pub(super) struct Simulator {
    vfs_name: String,
    state: Arc<MachineState>,
}

impl Simulator {
    /// Pulls the plug on this machine.
    pub(super) fn crash(&self) {
        self.state.crash();
    }

    /// Builds the SQLite URI that routes `DatabaseLocation::path` and plain
    /// `Connection::open` through this simulator's VFS.
    pub(super) fn database_uri(&self, directory: &Path) -> String {
        let database = directory.join("ora.sqlite3");
        format!(
            "file:{}?vfs={}",
            database.to_string_lossy().replace('\\', "/"),
            self.vfs_name
        )
    }
}

/// Registers a fresh power-cut simulator with a process-unique VFS name.
pub(super) fn register_simulator() -> Simulator {
    let id = NEXT_SIMULATOR_ID.fetch_add(1, Ordering::Relaxed);
    let name = CString::new(format!("ora-power-cut-{id}")).expect("VFS name is valid");
    let state = Arc::new(MachineState {
        crashed: AtomicBool::new(false),
        durable: Mutex::new(HashMap::new()),
        after_crash: Mutex::new(AfterCrash {
            files: HashMap::new(),
            shm: HashMap::new(),
        }),
    });

    // Safety: querying the default VFS only reads SQLite's global registry,
    // which `sqlite3_vfs_find` initializes on demand.
    let underlying = unsafe { ffi::sqlite3_vfs_find(std::ptr::null()) };
    assert!(!underlying.is_null(), "SQLite must expose a default VFS");

    let io = ffi::sqlite3_io_methods {
        iVersion: 2,
        xClose: Some(xclose),
        xRead: Some(xread),
        xWrite: Some(xwrite),
        xTruncate: Some(xtruncate),
        xSync: Some(xsync),
        xFileSize: Some(xfile_size),
        xLock: Some(xlock),
        xUnlock: Some(xunlock),
        xCheckReservedLock: Some(xcheck_reserved_lock),
        xFileControl: Some(xfile_control),
        xSectorSize: Some(xsector_size),
        xDeviceCharacteristics: Some(xdevice_characteristics),
        xShmMap: Some(xshm_map),
        xShmLock: Some(xshm_lock),
        xShmBarrier: Some(xshm_barrier),
        xShmUnmap: Some(xshm_unmap),
        xFetch: None,
        xUnfetch: None,
    };

    // Safety: reading the platform maximum pathname only inspects the
    // registered VFS that was just resolved above.
    let mx_pathname = unsafe { (*underlying).mxPathname };
    let base = ffi::sqlite3_vfs {
        iVersion: 2,
        szOsFile: std::mem::size_of::<CrashFile>() as c_int,
        mxPathname: mx_pathname,
        pNext: std::ptr::null_mut(),
        zName: name.as_ptr(),
        pAppData: std::ptr::null_mut(),
        xOpen: Some(xopen),
        xDelete: Some(xdelete),
        xAccess: Some(xaccess),
        xFullPathname: Some(xfull_pathname),
        xDlOpen: Some(xdl_open),
        xDlError: Some(xdl_error),
        xDlSym: Some(xdl_sym),
        xDlClose: Some(xdl_close),
        xRandomness: Some(xrandomness),
        xSleep: Some(xsleep),
        xCurrentTime: Some(xcurrent_time),
        xGetLastError: Some(xget_last_error),
        xCurrentTimeInt64: Some(xcurrent_time_int64),
        xSetSystemCall: None,
        xGetSystemCall: None,
        xNextSystemCall: None,
    };

    let mut simulator = Box::new(CrashVfs {
        base,
        underlying,
        name,
        state: state.clone(),
        io,
    });
    // Safety: registration only links the fully initialized VFS into
    // SQLite's registry; the struct stays alive for the process lifetime.
    let result = unsafe {
        ffi::sqlite3_vfs_register(&mut simulator.base, /*make_dflt*/ 0)
    };
    assert_eq!(
        result,
        ffi::SQLITE_OK,
        "registering the simulator VFS failed"
    );
    // The registration keeps the raw pointer, so the VFS must outlive it.
    let leaked = Box::leak(simulator);

    Simulator {
        vfs_name: leaked.name.to_string_lossy().into_owned(),
        state,
    }
}

/// Borrows the file handle SQLite handed to an I/O method.
unsafe fn crash_file(file: *mut ffi::sqlite3_file) -> &'static mut CrashFile {
    unsafe { &mut *(file as *mut CrashFile) }
}

/// Reads the method table of the wrapped platform file.
unsafe fn underlying_methods(handle: &CrashFile) -> &'static ffi::sqlite3_io_methods {
    unsafe { &*(*handle.underlying).pMethods }
}

/// Decodes the file name SQLite handed to `xOpen`, using an empty string for
/// its unnamed temporary files.
unsafe fn decode_name(z_name: *const c_char) -> String {
    if z_name.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(z_name) }
            .to_string_lossy()
            .into_owned()
    }
}

/// Copies `amount` bytes at `offset` out of shared memory, zero-filling what
/// lies past the end and reporting the short read SQLite expects.
unsafe fn read_memory(
    memory: &Mutex<Vec<u8>>,
    buffer: *mut c_void,
    amount: c_int,
    offset: i64,
) -> c_int {
    unsafe {
        let content = memory.lock().expect("post-crash file lock");
        let start = usize::try_from(offset).unwrap_or(usize::MAX);
        let amount = usize::try_from(amount).unwrap_or(0);
        let destination = std::slice::from_raw_parts_mut(buffer.cast::<u8>(), amount);
        let available = content.len().saturating_sub(start).min(amount);
        if available > 0 {
            destination[..available].copy_from_slice(&content[start..start + available]);
        }
        if available < amount {
            destination[available..].fill(0);
            return ffi::SQLITE_IOERR_SHORT_READ;
        }
        ffi::SQLITE_OK
    }
}

/// Copies `amount` bytes at `offset` into shared memory, growing it as needed.
unsafe fn write_memory(
    memory: &Mutex<Vec<u8>>,
    buffer: *const c_void,
    amount: c_int,
    offset: i64,
) -> c_int {
    unsafe {
        let mut content = memory.lock().expect("post-crash file lock");
        let start = usize::try_from(offset).unwrap_or(usize::MAX);
        let amount = usize::try_from(amount).unwrap_or(0);
        let source = std::slice::from_raw_parts(buffer.cast::<u8>(), amount);
        if start > content.len() {
            content.resize(start, 0);
        }
        let end = start.saturating_add(amount);
        if end > content.len() {
            content.resize(end, 0);
        }
        content[start..end].copy_from_slice(source);
        ffi::SQLITE_OK
    }
}

/// Opens a platform file and wraps it for a powered-on machine.
unsafe fn open_platform_file(
    simulator: &CrashVfs,
    z_name: *const c_char,
    file: *mut ffi::sqlite3_file,
    flags: c_int,
    out_flags: *mut c_int,
) -> c_int {
    unsafe {
        // The platform file struct needs its own allocation matching the
        // platform VFS's declared size; u64 chunks satisfy its alignment.
        let storage_words = ((*simulator.underlying).szOsFile.max(0) as usize).div_ceil(8);
        let mut storage = vec![0u64; storage_words];
        let underlying = storage.as_mut_ptr().cast::<ffi::sqlite3_file>();
        let result = match (*simulator.underlying).xOpen {
            Some(open) => open(simulator.underlying, z_name, underlying, flags, out_flags),
            None => return ffi::SQLITE_ERROR,
        };
        if result != ffi::SQLITE_OK {
            // SQLite does not call xClose for a failed open, so nothing was
            // wrapped and the storage is simply dropped here.
            return result;
        }
        // The buffer SQLite handed over is uninitialized, so the handle is
        // written wholesale with `ptr::write`: a plain field assignment would
        // first drop the garbage bytes as an Arc/String/Vec.
        std::ptr::write(
            file as *mut CrashFile,
            CrashFile {
                base: ffi::sqlite3_file {
                    pMethods: &simulator.io,
                },
                state: simulator.state.clone(),
                path: decode_name(z_name),
                underlying,
                underlying_storage: storage,
                memory: None,
            },
        );
        ffi::SQLITE_OK
    }
}

unsafe extern "C" fn xopen(
    vfs: *mut ffi::sqlite3_vfs,
    z_name: ffi::sqlite3_filename,
    file: *mut ffi::sqlite3_file,
    flags: c_int,
    out_flags: *mut c_int,
) -> c_int {
    unsafe {
        let simulator = &*(vfs as *const CrashVfs);
        let state = simulator.state.clone();

        // SQLite passes a null name for its own temporary files. They hold no
        // durable database state, so the platform serves them directly whether
        // the machine is on or has come back from a cut.
        if z_name.is_null() {
            return open_platform_file(simulator, z_name, file, flags, out_flags);
        }
        let path = decode_name(z_name);

        if state.is_crashed() {
            let memory = state.memory_file(&path);
            std::ptr::write(
                file as *mut CrashFile,
                CrashFile {
                    base: ffi::sqlite3_file {
                        pMethods: &simulator.io,
                    },
                    state,
                    path,
                    underlying: std::ptr::null_mut(),
                    underlying_storage: Vec::new(),
                    memory: Some(memory),
                },
            );
            if !out_flags.is_null() {
                *out_flags = flags;
            }
            return ffi::SQLITE_OK;
        }

        // Powered on: open through the platform VFS and remember what the
        // file holds right now as the baseline a cut would fall back to.
        let result = open_platform_file(simulator, z_name, file, flags, out_flags);
        if result == ffi::SQLITE_OK {
            state.seed_durable(&path);
        }
        result
    }
}

unsafe extern "C" fn xdelete(
    vfs: *mut ffi::sqlite3_vfs,
    z_name: *const c_char,
    sync_dir: c_int,
) -> c_int {
    unsafe {
        let simulator = &*(vfs as *const CrashVfs);
        if simulator.state.is_crashed() {
            // Close-time cleanups of the dying process must not erase what
            // the cut preserved, and post-crash files only exist in simulator
            // memory.
            return ffi::SQLITE_OK;
        }
        match (*simulator.underlying).xDelete {
            Some(delete) => delete(simulator.underlying, z_name, sync_dir),
            None => ffi::SQLITE_OK,
        }
    }
}

unsafe extern "C" fn xaccess(
    vfs: *mut ffi::sqlite3_vfs,
    z_name: *const c_char,
    flags: c_int,
    out: *mut c_int,
) -> c_int {
    unsafe {
        let simulator = &*(vfs as *const CrashVfs);
        match (*simulator.underlying).xAccess {
            Some(access) => access(simulator.underlying, z_name, flags, out),
            None => ffi::SQLITE_ERROR,
        }
    }
}

unsafe extern "C" fn xfull_pathname(
    vfs: *mut ffi::sqlite3_vfs,
    z_name: *const c_char,
    n_out: c_int,
    z_out: *mut c_char,
) -> c_int {
    unsafe {
        let simulator = &*(vfs as *const CrashVfs);
        match (*simulator.underlying).xFullPathname {
            Some(full_pathname) => full_pathname(simulator.underlying, z_name, n_out, z_out),
            None => ffi::SQLITE_ERROR,
        }
    }
}

unsafe extern "C" fn xdl_open(vfs: *mut ffi::sqlite3_vfs, path: *const c_char) -> *mut c_void {
    unsafe {
        let simulator = &*(vfs as *const CrashVfs);
        match (*simulator.underlying).xDlOpen {
            Some(dl_open) => dl_open(simulator.underlying, path),
            None => std::ptr::null_mut(),
        }
    }
}

unsafe extern "C" fn xdl_error(vfs: *mut ffi::sqlite3_vfs, n_byte: c_int, message: *mut c_char) {
    unsafe {
        let simulator = &*(vfs as *const CrashVfs);
        if let Some(dl_error) = (*simulator.underlying).xDlError {
            dl_error(simulator.underlying, n_byte, message);
        }
    }
}

unsafe extern "C" fn xdl_sym(
    vfs: *mut ffi::sqlite3_vfs,
    handle: *mut c_void,
    symbol: *const c_char,
) -> Option<unsafe extern "C" fn(*mut ffi::sqlite3_vfs, *mut c_void, *const c_char)> {
    unsafe {
        let simulator = &*(vfs as *const CrashVfs);
        (*simulator.underlying)
            .xDlSym
            .and_then(|dl_sym| dl_sym(simulator.underlying, handle, symbol))
    }
}

unsafe extern "C" fn xdl_close(vfs: *mut ffi::sqlite3_vfs, handle: *mut c_void) {
    unsafe {
        let simulator = &*(vfs as *const CrashVfs);
        if let Some(dl_close) = (*simulator.underlying).xDlClose {
            dl_close(simulator.underlying, handle);
        }
    }
}

unsafe extern "C" fn xrandomness(
    vfs: *mut ffi::sqlite3_vfs,
    n_byte: c_int,
    z_out: *mut c_char,
) -> c_int {
    unsafe {
        let simulator = &*(vfs as *const CrashVfs);
        match (*simulator.underlying).xRandomness {
            Some(randomness) => randomness(simulator.underlying, n_byte, z_out),
            None => ffi::SQLITE_ERROR,
        }
    }
}

unsafe extern "C" fn xsleep(vfs: *mut ffi::sqlite3_vfs, microseconds: c_int) -> c_int {
    unsafe {
        let simulator = &*(vfs as *const CrashVfs);
        match (*simulator.underlying).xSleep {
            Some(sleep) => sleep(simulator.underlying, microseconds),
            None => ffi::SQLITE_ERROR,
        }
    }
}

unsafe extern "C" fn xcurrent_time(vfs: *mut ffi::sqlite3_vfs, now: *mut f64) -> c_int {
    unsafe {
        let simulator = &*(vfs as *const CrashVfs);
        match (*simulator.underlying).xCurrentTime {
            Some(current_time) => current_time(simulator.underlying, now),
            None => ffi::SQLITE_ERROR,
        }
    }
}

unsafe extern "C" fn xget_last_error(
    vfs: *mut ffi::sqlite3_vfs,
    n_byte: c_int,
    z_out: *mut c_char,
) -> c_int {
    unsafe {
        let simulator = &*(vfs as *const CrashVfs);
        match (*simulator.underlying).xGetLastError {
            Some(get_last_error) => get_last_error(simulator.underlying, n_byte, z_out),
            None => ffi::SQLITE_ERROR,
        }
    }
}

unsafe extern "C" fn xcurrent_time_int64(vfs: *mut ffi::sqlite3_vfs, now: *mut i64) -> c_int {
    unsafe {
        let simulator = &*(vfs as *const CrashVfs);
        match (*simulator.underlying).xCurrentTimeInt64 {
            Some(current_time) => current_time(simulator.underlying, now),
            None => ffi::SQLITE_ERROR,
        }
    }
}

unsafe extern "C" fn xclose(file: *mut ffi::sqlite3_file) -> c_int {
    unsafe {
        let handle = crash_file(file);
        let mut result = ffi::SQLITE_OK;
        if handle.wraps_platform_file() {
            // Releasing the platform handle is exactly what process death
            // does; a crashed handle simply has nothing left to flush.
            if let Some(close) = underlying_methods(handle).xClose {
                result = close(handle.underlying);
            }
            handle.underlying = std::ptr::null_mut();
        }
        // The handle lives in SQLite-owned memory, so its Rust fields are
        // dropped here rather than by any Rust scope.
        std::ptr::drop_in_place(file as *mut CrashFile);
        result
    }
}

unsafe extern "C" fn xread(
    file: *mut ffi::sqlite3_file,
    buffer: *mut c_void,
    amount: c_int,
    offset: i64,
) -> c_int {
    unsafe {
        let handle = crash_file(file);
        if let Some(memory) = &handle.memory {
            let result = read_memory(memory, buffer, amount, offset);
            return result;
        }
        if handle.path.is_empty() || !handle.state.is_crashed() {
            if let Some(read) = underlying_methods(handle).xRead {
                return read(handle.underlying, buffer, amount, offset);
            }
            return ffi::SQLITE_OK;
        }
        ffi::SQLITE_IOERR_READ
    }
}

unsafe extern "C" fn xwrite(
    file: *mut ffi::sqlite3_file,
    buffer: *const c_void,
    amount: c_int,
    offset: i64,
) -> c_int {
    unsafe {
        let handle = crash_file(file);
        if let Some(memory) = &handle.memory {
            return write_memory(memory, buffer, amount, offset);
        }
        if handle.path.is_empty() || !handle.state.is_crashed() {
            if let Some(write) = underlying_methods(handle).xWrite {
                return write(handle.underlying, buffer, amount, offset);
            }
            return ffi::SQLITE_OK;
        }
        ffi::SQLITE_IOERR_WRITE
    }
}

unsafe extern "C" fn xtruncate(file: *mut ffi::sqlite3_file, size: i64) -> c_int {
    unsafe {
        let handle = crash_file(file);
        if let Some(memory) = &handle.memory {
            let length = usize::try_from(size).unwrap_or(0);
            memory
                .lock()
                .expect("post-crash file lock")
                .resize(length, 0);
            return ffi::SQLITE_OK;
        }
        if handle.path.is_empty() || !handle.state.is_crashed() {
            if let Some(truncate) = underlying_methods(handle).xTruncate {
                return truncate(handle.underlying, size);
            }
            return ffi::SQLITE_OK;
        }
        ffi::SQLITE_IOERR_WRITE
    }
}

unsafe extern "C" fn xsync(file: *mut ffi::sqlite3_file, flags: c_int) -> c_int {
    unsafe {
        let handle = crash_file(file);
        // Post-crash memory already is the machine's only storage; nothing
        // more can be lost, so syncing is a no-op.
        if handle.memory.is_some() {
            return ffi::SQLITE_OK;
        }
        if handle.path.is_empty() || !handle.state.is_crashed() {
            let result = match underlying_methods(handle).xSync {
                Some(sync) => sync(handle.underlying, flags),
                None => ffi::SQLITE_OK,
            };
            if result == ffi::SQLITE_OK && !handle.path.is_empty() {
                handle.state.mark_durable(&handle.path);
            }
            return result;
        }
        ffi::SQLITE_IOERR_FSYNC
    }
}

unsafe extern "C" fn xfile_size(file: *mut ffi::sqlite3_file, size: *mut i64) -> c_int {
    unsafe {
        let handle = crash_file(file);
        if let Some(memory) = &handle.memory {
            *size = memory.lock().expect("post-crash file lock").len() as i64;
            return ffi::SQLITE_OK;
        }
        if handle.path.is_empty() || !handle.state.is_crashed() {
            if let Some(file_size) = underlying_methods(handle).xFileSize {
                return file_size(handle.underlying, size);
            }
            return ffi::SQLITE_OK;
        }
        ffi::SQLITE_IOERR_FSTAT
    }
}

unsafe extern "C" fn xlock(file: *mut ffi::sqlite3_file, lock: c_int) -> c_int {
    unsafe {
        let handle = crash_file(file);
        if handle.memory.is_some() {
            return ffi::SQLITE_OK;
        }
        if handle.path.is_empty() || !handle.state.is_crashed() {
            if let Some(lock_file) = underlying_methods(handle).xLock {
                return lock_file(handle.underlying, lock);
            }
            return ffi::SQLITE_OK;
        }
        ffi::SQLITE_IOERR_LOCK
    }
}

unsafe extern "C" fn xunlock(file: *mut ffi::sqlite3_file, lock: c_int) -> c_int {
    unsafe {
        let handle = crash_file(file);
        if handle.memory.is_some() {
            return ffi::SQLITE_OK;
        }
        if handle.path.is_empty() || !handle.state.is_crashed() {
            if let Some(unlock) = underlying_methods(handle).xUnlock {
                return unlock(handle.underlying, lock);
            }
            return ffi::SQLITE_OK;
        }
        ffi::SQLITE_IOERR_UNLOCK
    }
}

unsafe extern "C" fn xcheck_reserved_lock(file: *mut ffi::sqlite3_file, out: *mut c_int) -> c_int {
    unsafe {
        let handle = crash_file(file);
        if handle.memory.is_some() {
            *out = 0;
            return ffi::SQLITE_OK;
        }
        if handle.path.is_empty() || !handle.state.is_crashed() {
            if let Some(check) = underlying_methods(handle).xCheckReservedLock {
                return check(handle.underlying, out);
            }
            return ffi::SQLITE_OK;
        }
        ffi::SQLITE_IOERR_LOCK
    }
}

unsafe extern "C" fn xfile_control(
    file: *mut ffi::sqlite3_file,
    op: c_int,
    arg: *mut c_void,
) -> c_int {
    unsafe {
        let handle = crash_file(file);
        if handle.memory.is_some() {
            // The simulator implements no file controls of its own.
            return ffi::SQLITE_NOTFOUND;
        }
        if handle.path.is_empty() || !handle.state.is_crashed() {
            if let Some(file_control) = underlying_methods(handle).xFileControl {
                return file_control(handle.underlying, op, arg);
            }
            return ffi::SQLITE_NOTFOUND;
        }
        ffi::SQLITE_NOTFOUND
    }
}

unsafe extern "C" fn xsector_size(file: *mut ffi::sqlite3_file) -> c_int {
    unsafe {
        let handle = crash_file(file);
        if handle.memory.is_some() {
            return 512;
        }
        match underlying_methods(handle).xSectorSize {
            Some(sector_size) => sector_size(handle.underlying),
            None => 512,
        }
    }
}

unsafe extern "C" fn xdevice_characteristics(file: *mut ffi::sqlite3_file) -> c_int {
    unsafe {
        let handle = crash_file(file);
        if handle.memory.is_some() {
            return 0;
        }
        match underlying_methods(handle).xDeviceCharacteristics {
            Some(characteristics) => characteristics(handle.underlying),
            None => 0,
        }
    }
}

unsafe extern "C" fn xshm_map(
    file: *mut ffi::sqlite3_file,
    region_index: c_int,
    region_size: c_int,
    extend: c_int,
    out: *mut *mut c_void,
) -> c_int {
    unsafe {
        let handle = crash_file(file);
        if handle.memory.is_some() {
            // A missing region without the extend flag maps to a null pointer
            // with success, the same way the platform VFSes behave; that is
            // what makes SQLite rebuild the wal-index from the recovered WAL.
            handle.state.shm_region(
                &handle.path,
                region_index.max(0) as usize,
                region_size.max(0) as usize,
                extend != 0,
                out,
            );
            return ffi::SQLITE_OK;
        }
        if handle.path.is_empty() || !handle.state.is_crashed() {
            if let Some(shm_map) = underlying_methods(handle).xShmMap {
                return shm_map(handle.underlying, region_index, region_size, extend, out);
            }
            return ffi::SQLITE_OK;
        }
        ffi::SQLITE_IOERR
    }
}

unsafe extern "C" fn xshm_lock(
    file: *mut ffi::sqlite3_file,
    offset: c_int,
    n: c_int,
    flags: c_int,
) -> c_int {
    unsafe {
        let handle = crash_file(file);
        if handle.memory.is_some() {
            return ffi::SQLITE_OK;
        }
        if handle.path.is_empty() || !handle.state.is_crashed() {
            if let Some(shm_lock) = underlying_methods(handle).xShmLock {
                return shm_lock(handle.underlying, offset, n, flags);
            }
            return ffi::SQLITE_OK;
        }
        ffi::SQLITE_IOERR_LOCK
    }
}

unsafe extern "C" fn xshm_barrier(file: *mut ffi::sqlite3_file) {
    unsafe {
        let handle = crash_file(file);
        if handle.memory.is_some() || handle.state.is_crashed() {
            return;
        }
        if let Some(shm_barrier) = underlying_methods(handle).xShmBarrier {
            shm_barrier(handle.underlying);
        }
    }
}

unsafe extern "C" fn xshm_unmap(file: *mut ffi::sqlite3_file, delete_flag: c_int) -> c_int {
    unsafe {
        let handle = crash_file(file);
        if handle.memory.is_some() {
            // Regions stay in simulator memory; they model files the cut kept.
            return ffi::SQLITE_OK;
        }
        if handle.path.is_empty() || !handle.state.is_crashed() {
            if let Some(shm_unmap) = underlying_methods(handle).xShmUnmap {
                return shm_unmap(handle.underlying, delete_flag);
            }
            return ffi::SQLITE_OK;
        }
        ffi::SQLITE_OK
    }
}
