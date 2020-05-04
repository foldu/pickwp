use nix::{
    fcntl::{fcntl, open, FcntlArg, OFlag},
    sys::stat::Mode,
    unistd::close,
    NixPath,
};
use std::os::unix::io::RawFd;

struct OwnedFd(RawFd);

impl Drop for OwnedFd {
    fn drop(&mut self) {
        let _ = close(self.0);
    }
}

pub struct LockFile(OwnedFd);

#[derive(snafu::Snafu, Debug)]
pub enum Error {
    #[snafu(context(false))]
    Create {
        source: nix::Error,
    },

    Locked,
}

#[repr(i32)]
enum LockTy {
    //RDLCK = libc::F_RDLCK,
    WRLCK = libc::F_WRLCK,
    UNLCK = libc::F_UNLCK,
}

fn entire_file_flock(ty: LockTy) -> libc::flock {
    libc::flock {
        l_type: ty as libc::c_short,
        l_whence: libc::SEEK_SET as libc::c_short,
        l_start: 0,
        l_len: 0,
        // doesn't matter in this context
        l_pid: 0,
    }
}

impl LockFile {
    pub fn lock(path: &impl NixPath) -> Result<Self, Error> {
        let fd = open(
            path,
            OFlag::O_CREAT | OFlag::O_CLOEXEC | OFlag::O_WRONLY,
            Mode::from_bits_truncate(0o600),
        )
        .map(OwnedFd).map_err(|e| match e {
            nix::Error::Sys(nix
        })?;
        let flock = entire_file_flock(LockTy::WRLCK);
        match fcntl(fd.0, FcntlArg::F_SETLK(&flock)) {
            Ok(_) => Ok(Self(fd)),
            Err(nix::Error::Sys(nix::errno::Errno::EAGAIN)) => Err(Error::Locked),
            Err(e) => Err(Error::Create { source: e }),
        }
    }
}

impl Drop for LockFile {
    fn drop(&mut self) {
        let flock = entire_file_flock(LockTy::UNLCK);
        let _ = fcntl((self.0).0, FcntlArg::F_SETLK(&flock));
    }
}
