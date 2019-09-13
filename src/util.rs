pub trait PathBufExt {
    fn into_string(self) -> Result<String, std::ffi::OsString>;
}

impl PathBufExt for std::path::PathBuf {
    fn into_string(self) -> Result<String, std::ffi::OsString> {
        self.into_os_string().into_string()
    }
}
