pub type AppResult<T = ()> = Result<T, anyhow::Error>;

macro_rules! vrm_error {
    ($err:expr) => {
        let _e = $err;
        #[cfg(feature = "log")]
        bevy::log::error!("{_e}")
    };
    ($message:literal, $err: expr) => {
        let _e = $err;
        #[cfg(feature = "log")]
        bevy::log::error!("{}: {_e}", $message)
    };
    ($($arg:tt)*) => {{
        #[cfg(feature = "log")]
        bevy::log::error!($($arg)*)
    }};
}

pub(crate) use vrm_error;

/// Reports a recoverable problem in an asset without failing the load.
///
/// VRM files in the wild routinely omit fields the specification marks as
/// required. Every such case is skipped with one of these warnings rather than
/// aborting, so a single malformed extension never costs the whole avatar.
macro_rules! vrm_warn {
    ($err:expr) => {
        let _e = $err;
        #[cfg(feature = "log")]
        bevy::log::warn!("{_e}")
    };
    ($message:literal, $err: expr) => {
        let _e = $err;
        #[cfg(feature = "log")]
        bevy::log::warn!("{}: {_e}", $message)
    };
    ($($arg:tt)*) => {{
        #[cfg(feature = "log")]
        bevy::log::warn!($($arg)*)
    }};
}

pub(crate) use vrm_warn;
