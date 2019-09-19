#[macro_export]
macro_rules! try_or_err {
    ($e:expr) => {
        if let Err(e) = $e {
            log::error!("{}", e);
        }
    };
}
