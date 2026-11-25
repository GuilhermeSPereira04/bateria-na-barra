//! Start menu apps. Stub: Task 5 enumerates them and adds the icon.
#![allow(dead_code)] // filled by Task 5

#[derive(Clone)]
pub struct AppEntry {
    pub name: String,
    /// AppUserModelID or parsing name.
    pub id: String,
}
