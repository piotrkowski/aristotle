// SPDX-License-Identifier: AGPL-3.0-or-later
//! Exit requests handled by the launcher after restoring the stock interface.

use std::path::Path;

pub const KUAL_EXIT_CODE: u8 = 20;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum Target {
    #[default]
    Kindle,
    Kual,
}

impl Target {
    pub fn exit_code(self) -> u8 {
        match self {
            Self::Kindle => 0,
            Self::Kual => KUAL_EXIT_CODE,
        }
    }
}

pub fn kual_available() -> bool {
    std::env::var_os("ARISTOTLE_EXIT_ACTIONS").as_deref() == Some(std::ffi::OsStr::new("1"))
        && Path::new("/opt/amazon/ebook/booklet/KUALBooklet.jar").is_file()
}
