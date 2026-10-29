//! Shim VFS over the default SQLite VFS.
//!
//! A failed sync restores each file to the bytes from its last successful
//! sync and then refuses further writes. That is the power-loss model: data
//! that had not reached a successful `xSync` is gone. A torn write leaves a
//! short prefix on disk and then does the same. This is not a machine
//! power-cut.

use std::ffi::{CStr, c_char, c_int, c_void};
use std::sync::Mutex;

use libsqlite3_sys::{
    SQLITE_IOERR, SQLITE_OK, sqlite3_file, sqlite3_initialize, sqlite3_int64, sqlite3_io_methods,
    sqlite3_vfs, sqlite3_vfs_find, sqlite3_vfs_register,
};

const NAME: &[u8] = b"puyu-fault\0";

struct State {
    /// Fail the next `xSync` after reverting to the last successful sync.
    fail_sync: bool,
    /// Short-write the next `xWrite`, then stop.
    tear_write: bool,
    /// Further writes and syncs fail and do not change the file.
    dead: bool,
    snapshots: Vec<Vec<u8>>,
}

impl State {
    fn new() -> Self {
        Self {
            fail_sync: false,
            tear_write: false,
            dead: false,
            snapshots: Vec::new(),
        }
    }
}

static STATE: Mutex<State> = Mutex::new(State {
    fail_sync: false,
    tear_write: false,
    dead: false,
    snapshots: Vec::new(),
});

/// Register the shim once. Later calls are no-ops.
pub fn install() {
    // SAFETY: sqlite3_initialize is the documented setup call. The leaked VFS
    // and name live for the process, which is what sqlite3_vfs_register requires.
    unsafe {
        sqlite3_initialize();
        if !sqlite3_vfs_find(NAME.as_ptr().cast()).is_null() {
            return;
        }
        let parent = sqlite3_vfs_find(std::ptr::null());
        assert!(!parent.is_null(), "sqlite default vfs");
        let mut vfs = *parent;
        vfs.zName = NAME.as_ptr().cast();
        vfs.pAppData = parent.cast();
        vfs.szOsFile += std::mem::size_of::<FaultFile>() as c_int;
        vfs.xOpen = Some(x_open);
        vfs.pNext = std::ptr::null_mut();
        let leaked = Box::leak(Box::new(vfs));
        sqlite3_vfs_register(leaked, 0);
    }
}

/// Clear a previous injection. Does not unregister the VFS.
pub fn reset() {
    *STATE.lock().expect("fault state") = State::new();
}

/// The next `xSync` is a power loss: unsynced bytes are discarded.
pub fn fail_next_sync() {
    let mut state = STATE.lock().expect("fault state");
    state.fail_sync = true;
    state.dead = false;
}

/// The next `xWrite` persists only a short prefix, then power is lost.
pub fn tear_next_write() {
    let mut state = STATE.lock().expect("fault state");
    state.tear_write = true;
    state.dead = false;
}

#[repr(C)]
struct FaultFile {
    base: sqlite3_file,
    id: usize,
}

fn real_file(file: *mut sqlite3_file) -> *mut sqlite3_file {
    // SAFETY: SQLite allocated szOsFile bytes. The parent file sits immediately
    // after FaultFile, which is the layout x_open passed to the parent VFS.
    unsafe {
        file.cast::<u8>()
            .add(std::mem::size_of::<FaultFile>())
            .cast()
    }
}

unsafe extern "C" fn x_open(
    vfs: *mut sqlite3_vfs,
    name: *const c_char,
    file: *mut sqlite3_file,
    flags: c_int,
    out_flags: *mut c_int,
) -> c_int {
    let fault = file.cast::<FaultFile>();
    // SAFETY: SQLite allocated this buffer for xOpen and it is uninitialized.
    unsafe {
        std::ptr::write_bytes(fault, 0, 1);
    }
    let id = {
        let mut state = STATE.lock().expect("fault state");
        let id = state.snapshots.len();
        state.snapshots.push(Vec::new());
        id
    };
    unsafe {
        (*fault).id = id;
    }
    let real = real_file(file);
    let parent = unsafe { (*vfs).pAppData.cast::<sqlite3_vfs>() };
    let open = unsafe { (*parent).xOpen.expect("parent xOpen") };
    let rc = unsafe { open(parent, name, real, flags, out_flags) };
    if rc != SQLITE_OK {
        return rc;
    }
    let snap = read_all(real);
    if let Some(slot) = STATE.lock().expect("fault state").snapshots.get_mut(id) {
        *slot = snap;
    }
    unsafe {
        (*file).pMethods = &IO;
    }
    SQLITE_OK
}

fn read_all(real: *mut sqlite3_file) -> Vec<u8> {
    // SAFETY: real is an open parent file whose pMethods SQLite just set.
    unsafe {
        let methods = (*real).pMethods;
        let mut size: sqlite3_int64 = 0;
        if let Some(file_size) = (*methods).xFileSize
            && (file_size(real, &mut size) != SQLITE_OK || size <= 0)
        {
            return Vec::new();
        }
        let mut buf = vec![0u8; size as usize];
        if let Some(read) = (*methods).xRead
            && read(real, buf.as_mut_ptr().cast(), size as c_int, 0) != SQLITE_OK
        {
            return Vec::new();
        }
        buf
    }
}

fn restore(real: *mut sqlite3_file, bytes: &[u8]) {
    // SAFETY: caller holds an open parent file and is applying the last synced image.
    unsafe {
        let methods = (*real).pMethods;
        if let Some(truncate) = (*methods).xTruncate {
            let _ = truncate(real, bytes.len() as sqlite3_int64);
        }
        if !bytes.is_empty()
            && let Some(write) = (*methods).xWrite
        {
            let _ = write(real, bytes.as_ptr().cast(), bytes.len() as c_int, 0);
        }
    }
}

unsafe extern "C" fn x_close(file: *mut sqlite3_file) -> c_int {
    let real = real_file(file);
    let rc = call1(real, |m| m.xClose);
    unsafe {
        (*file).pMethods = std::ptr::null();
    }
    rc
}

unsafe extern "C" fn x_read(
    file: *mut sqlite3_file,
    buf: *mut c_void,
    amt: c_int,
    offset: sqlite3_int64,
) -> c_int {
    let real = real_file(file);
    // SAFETY: parent xRead matches this signature.
    unsafe {
        let methods = (*real).pMethods;
        match (*methods).xRead {
            Some(read) => read(real, buf, amt, offset),
            None => SQLITE_IOERR,
        }
    }
}

unsafe extern "C" fn x_write(
    file: *mut sqlite3_file,
    buf: *const c_void,
    amt: c_int,
    offset: sqlite3_int64,
) -> c_int {
    let tear = {
        let mut state = STATE.lock().expect("fault state");
        if state.dead {
            return SQLITE_IOERR;
        }
        if state.tear_write {
            state.tear_write = false;
            state.dead = true;
            true
        } else {
            false
        }
    };
    let real = real_file(file);
    if tear {
        let prefix = (amt - 1).max(0);
        if prefix > 0 {
            // SAFETY: buf has amt bytes. The prefix is a torn sector that reached disk.
            unsafe {
                let methods = (*real).pMethods;
                if let Some(write) = (*methods).xWrite {
                    let _ = write(real, buf, prefix, offset);
                }
            }
        }
        return SQLITE_IOERR;
    }
    // SAFETY: parent xWrite matches this signature.
    unsafe {
        let methods = (*real).pMethods;
        match (*methods).xWrite {
            Some(write) => write(real, buf, amt, offset),
            None => SQLITE_IOERR,
        }
    }
}

unsafe extern "C" fn x_sync(file: *mut sqlite3_file, flags: c_int) -> c_int {
    let fail = {
        let mut state = STATE.lock().expect("fault state");
        if state.dead {
            return SQLITE_IOERR;
        }
        if state.fail_sync {
            state.fail_sync = false;
            state.dead = true;
            true
        } else {
            false
        }
    };
    let real = real_file(file);
    let id = unsafe { (*file.cast::<FaultFile>()).id };
    if fail {
        // reset() while a file is open can shrink the vec; fail the sync
        // without touching the file rather than indexing out of bounds.
        let snap = STATE
            .lock()
            .expect("fault state")
            .snapshots
            .get(id)
            .cloned()
            .unwrap_or_default();
        restore(real, &snap);
        return SQLITE_IOERR;
    }
    let rc = unsafe {
        let methods = (*real).pMethods;
        match (*methods).xSync {
            Some(sync) => sync(real, flags),
            None => SQLITE_OK,
        }
    };
    if rc == SQLITE_OK {
        let snap = read_all(real);
        if let Some(slot) = STATE.lock().expect("fault state").snapshots.get_mut(id) {
            *slot = snap;
        }
    }
    rc
}

unsafe extern "C" fn x_truncate(file: *mut sqlite3_file, size: sqlite3_int64) -> c_int {
    if STATE.lock().expect("fault state").dead {
        return SQLITE_IOERR;
    }
    call_truncate(real_file(file), size)
}

unsafe extern "C" fn x_file_size(file: *mut sqlite3_file, size: *mut sqlite3_int64) -> c_int {
    let real = real_file(file);
    unsafe {
        let methods = (*real).pMethods;
        match (*methods).xFileSize {
            Some(file_size) => file_size(real, size),
            None => SQLITE_IOERR,
        }
    }
}

unsafe extern "C" fn x_lock(file: *mut sqlite3_file, level: c_int) -> c_int {
    call_lock(real_file(file), level, true)
}

unsafe extern "C" fn x_unlock(file: *mut sqlite3_file, level: c_int) -> c_int {
    call_lock(real_file(file), level, false)
}

unsafe extern "C" fn x_check_reserved(file: *mut sqlite3_file, out: *mut c_int) -> c_int {
    let real = real_file(file);
    unsafe {
        let methods = (*real).pMethods;
        match (*methods).xCheckReservedLock {
            Some(check) => check(real, out),
            None => SQLITE_OK,
        }
    }
}

unsafe extern "C" fn x_file_control(file: *mut sqlite3_file, op: c_int, arg: *mut c_void) -> c_int {
    let real = real_file(file);
    unsafe {
        let methods = (*real).pMethods;
        match (*methods).xFileControl {
            Some(control) => control(real, op, arg),
            None => SQLITE_OK,
        }
    }
}

unsafe extern "C" fn x_sector_size(file: *mut sqlite3_file) -> c_int {
    let real = real_file(file);
    unsafe {
        let methods = (*real).pMethods;
        match (*methods).xSectorSize {
            Some(sector) => sector(real),
            None => 512,
        }
    }
}

unsafe extern "C" fn x_device_characteristics(file: *mut sqlite3_file) -> c_int {
    let real = real_file(file);
    unsafe {
        let methods = (*real).pMethods;
        match (*methods).xDeviceCharacteristics {
            Some(chars) => chars(real),
            None => 0,
        }
    }
}

unsafe extern "C" fn x_shm_map(
    file: *mut sqlite3_file,
    page: c_int,
    page_size: c_int,
    flags: c_int,
    out: *mut *mut c_void,
) -> c_int {
    let real = real_file(file);
    unsafe {
        let methods = (*real).pMethods;
        match (*methods).xShmMap {
            Some(map) => map(real, page, page_size, flags, out),
            None => SQLITE_IOERR,
        }
    }
}

unsafe extern "C" fn x_shm_lock(
    file: *mut sqlite3_file,
    offset: c_int,
    n: c_int,
    flags: c_int,
) -> c_int {
    let real = real_file(file);
    unsafe {
        let methods = (*real).pMethods;
        match (*methods).xShmLock {
            Some(lock) => lock(real, offset, n, flags),
            None => SQLITE_OK,
        }
    }
}

unsafe extern "C" fn x_shm_barrier(file: *mut sqlite3_file) {
    let real = real_file(file);
    unsafe {
        let methods = (*real).pMethods;
        if let Some(barrier) = (*methods).xShmBarrier {
            barrier(real);
        }
    }
}

unsafe extern "C" fn x_shm_unmap(file: *mut sqlite3_file, delete_flag: c_int) -> c_int {
    let real = real_file(file);
    unsafe {
        let methods = (*real).pMethods;
        match (*methods).xShmUnmap {
            Some(unmap) => unmap(real, delete_flag),
            None => SQLITE_OK,
        }
    }
}

fn call1(
    real: *mut sqlite3_file,
    pick: fn(&sqlite3_io_methods) -> Option<unsafe extern "C" fn(*mut sqlite3_file) -> c_int>,
) -> c_int {
    // SAFETY: real is an open parent file.
    unsafe {
        let methods = (*real).pMethods;
        match pick(&*methods) {
            Some(fun) => fun(real),
            None => SQLITE_OK,
        }
    }
}

fn call_truncate(real: *mut sqlite3_file, size: sqlite3_int64) -> c_int {
    unsafe {
        let methods = (*real).pMethods;
        match (*methods).xTruncate {
            Some(truncate) => truncate(real, size),
            None => SQLITE_IOERR,
        }
    }
}

fn call_lock(real: *mut sqlite3_file, level: c_int, lock: bool) -> c_int {
    unsafe {
        let methods = (*real).pMethods;
        let fun = if lock {
            (*methods).xLock
        } else {
            (*methods).xUnlock
        };
        match fun {
            Some(fun) => fun(real, level),
            None => SQLITE_OK,
        }
    }
}

static IO: sqlite3_io_methods = sqlite3_io_methods {
    iVersion: 2,
    xClose: Some(x_close),
    xRead: Some(x_read),
    xWrite: Some(x_write),
    xTruncate: Some(x_truncate),
    xSync: Some(x_sync),
    xFileSize: Some(x_file_size),
    xLock: Some(x_lock),
    xUnlock: Some(x_unlock),
    xCheckReservedLock: Some(x_check_reserved),
    xFileControl: Some(x_file_control),
    xSectorSize: Some(x_sector_size),
    xDeviceCharacteristics: Some(x_device_characteristics),
    xShmMap: Some(x_shm_map),
    xShmLock: Some(x_shm_lock),
    xShmBarrier: Some(x_shm_barrier),
    xShmUnmap: Some(x_shm_unmap),
    xFetch: None,
    xUnfetch: None,
};

#[allow(dead_code)]
fn _name_is_c_str() {
    let _ = CStr::from_bytes_with_nul(NAME);
}
