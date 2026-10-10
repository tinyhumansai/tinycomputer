//! Owned native child and reader workers: successful cleanup is a joined barrier.
use command_group::GroupChild;
use std::process::ExitStatus;
use std::thread::JoinHandle;

#[derive(Debug)]
pub(super) struct Owned {
    pub(super) child: GroupChild,
    readers: Vec<JoinHandle<()>>,
    kill_sent: bool,
    finished: bool,
    #[cfg(test)]
    fail_reap: bool,
}
impl Owned {
    pub(super) fn new(child: GroupChild) -> Self {
        Self {
            child,
            readers: Vec::new(),
            kill_sent: false,
            finished: false,
            #[cfg(test)]
            fail_reap: false,
        }
    }
    pub(super) fn reader(&mut self, worker: JoinHandle<()>) {
        self.readers.push(worker);
    }
    pub(super) fn try_wait(&mut self) -> std::io::Result<Option<ExitStatus>> {
        self.child.try_wait()
    }
    pub(super) fn cleanup(&mut self) -> Result<(), &'static str> {
        if self.finished {
            return Ok(());
        }
        // The direct child may already have exited while descendants still hold
        // our pipes. Signal the owned group before reaping or joining readers.
        if !self.kill_sent {
            if let Err(error) = self.child.kill() {
                // POSIX ESRCH means the entire group is already gone. Other errors
                // retain the child and readers so explicit stop can retry cleanup.
                #[cfg(unix)]
                const NO_SUCH_PROCESS: i32 = 3;
                #[cfg(unix)]
                let gone = error.raw_os_error() == Some(NO_SUCH_PROCESS);
                #[cfg(not(unix))]
                let gone = false;
                if !gone {
                    return Err("native_kill_failed");
                }
            }
            // A successful signal (or ESRCH) is terminal for this owned group;
            // retries must not signal a recycled process-group identity.
            self.kill_sent = true;
        }
        #[cfg(test)]
        if self.fail_reap {
            self.fail_reap = false;
            return Err("native_reap_failed");
        }
        self.child.wait().map_err(|_| "native_reap_failed")?;
        let mut failed = false;
        while let Some(reader) = self.readers.pop() {
            failed |= reader.join().is_err();
        }
        self.finished = true;
        if failed {
            Err("native_reader_failed")
        } else {
            Ok(())
        }
    }
}
impl Drop for Owned {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}
#[cfg(test)]
#[path = "owned_tests.rs"]
mod tests;
