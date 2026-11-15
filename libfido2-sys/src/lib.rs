// The musl build links libfido2 against the static libcrypto and zlib of these crates (build.rs,
// mod musl). Naming them makes them dependencies of this crate, so that the linker sees libfido2
// before the libraries it needs.
#[cfg(target_env = "musl")]
extern crate libz_sys;
#[cfg(target_env = "musl")]
extern crate openssl_sys;

#[allow(non_camel_case_types, non_snake_case, non_upper_case_globals)]
mod ffi;

pub use ffi::*;
