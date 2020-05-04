use nix::{
    fcntl::{fcntl, open, FcntlArg, OFlag},
    sys::stat::Mode,
    unistd::{self, close},
};
use std::{os::unix::io::RawFd, path::Path};

// only _ever_ call this on things you're sure won't fail on
// Error::InvalidUTF8 or if you're on HP UX
trait NixToStd<T> {
    fn to_std_err(self) -> Result<T, std::io::Error>;
}

impl<T> NixToStd<T> for Result<T, nix::Error> {
    fn to_std_err(self) -> Result<T, std::io::Error> {
        use std::io;
        self.map_err(|e| match e {
            nix::Error::Sys(errno) => io::Error::from_raw_os_error(errno as i32),
            nix::Error::InvalidPath => {
                io::Error::new(std::io::ErrorKind::InvalidInput, "Path is invalid")
            }
            e => panic!("Assumption failed: {}", e),
        })
    }
}

pub fn mkdir(path: impl AsRef<Path>, mode: Mode) -> Result<(), std::io::Error> {
    let path = path.as_ref();
    unistd::mkdir(path, mode).to_std_err()
}

struct OwnedFd(RawFd);

impl Drop for OwnedFd {
    fn drop(&mut self) {
        let _ = close(self.0);
    }
}

pub struct LockFile(OwnedFd);

#[derive(snafu::Snafu, Debug)]
pub enum LockFileError {
    #[snafu(context(false), display("Can't create lockfile: {}", source))]
    Create { source: std::io::Error },

    #[snafu(display("Lockfile is already locked"))]
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
    pub fn lock(path: impl AsRef<Path>) -> Result<Self, LockFileError> {
        let path = path.as_ref();
        let fd = open(
            path,
            OFlag::O_CREAT | OFlag::O_CLOEXEC | OFlag::O_WRONLY,
            Mode::from_bits_truncate(0o600),
        )
        .map(OwnedFd)
        .to_std_err()?;
        let flock = entire_file_flock(LockTy::WRLCK);

        match fcntl(fd.0, FcntlArg::F_SETLK(&flock)).to_std_err() {
            Ok(_) => Ok(Self(fd)),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => Err(LockFileError::Locked),
            Err(e) => Err(e.into()),
        }
    }
}

impl Drop for LockFile {
    fn drop(&mut self) {
        let flock = entire_file_flock(LockTy::UNLCK);
        let _ = fcntl((self.0).0, FcntlArg::F_SETLK(&flock));
    }
}
