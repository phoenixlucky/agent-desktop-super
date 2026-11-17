use std::{
    io, mem,
    os::windows::{io::AsRawHandle, process::CommandExt},
    process::{Child, Command},
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE},
    System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
    },
    System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
        SetInformationJobObject,
    },
    System::Threading::{CREATE_SUSPENDED, OpenThread, ResumeThread, THREAD_SUSPEND_RESUME},
};

pub(crate) struct ChildJob(HANDLE);

unsafe impl Send for ChildJob {}

impl ChildJob {
    pub(crate) fn new() -> io::Result<Self> {
        let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { mem::zeroed() };
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let configured = unsafe {
            SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if configured == 0 {
            let error = io::Error::last_os_error();
            unsafe { CloseHandle(handle) };
            return Err(error);
        }
        Ok(Self(handle))
    }

    pub(crate) fn spawn(&self, command: &mut Command) -> io::Result<Child> {
        command.creation_flags(CREATE_SUSPENDED);
        let child = command.spawn()?;
        let assigned = unsafe { AssignProcessToJobObject(self.0, child.as_raw_handle() as HANDLE) };
        if assigned == 0 {
            return stop_child(child, io::Error::last_os_error());
        }
        if let Err(error) = resume_initial_thread(child.id()) {
            return stop_child(child, error);
        }
        Ok(child)
    }
}

fn resume_initial_thread(pid: u32) -> io::Result<()> {
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    let mut entry: THREADENTRY32 = unsafe { mem::zeroed() };
    entry.dwSize = mem::size_of::<THREADENTRY32>() as u32;
    let mut found = false;
    let mut result = Ok(());
    let mut has_entry = unsafe { Thread32First(snapshot, &mut entry) } != 0;
    while has_entry {
        if entry.th32OwnerProcessID == pid {
            let thread = unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID) };
            if thread.is_null() {
                result = Err(io::Error::last_os_error());
            } else {
                let resumed = unsafe { ResumeThread(thread) };
                unsafe { CloseHandle(thread) };
                if resumed == u32::MAX {
                    result = Err(io::Error::last_os_error());
                } else {
                    found = true;
                }
            }
            break;
        }
        has_entry = unsafe { Thread32Next(snapshot, &mut entry) } != 0;
    }
    unsafe { CloseHandle(snapshot) };
    result?;
    if found {
        Ok(())
    } else {
        Err(io::Error::other("suspended process thread was not found"))
    }
}

fn stop_child(mut child: Child, error: io::Error) -> io::Result<Child> {
    let _ = child.kill();
    let _ = child.wait();
    Err(error)
}

impl Drop for ChildJob {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}
