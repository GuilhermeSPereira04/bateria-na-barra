//! What is playing. Stub: Task 8 reads it from the system media controls.
#![allow(dead_code)] // filled by Task 8

#[derive(Clone, Default)]
pub struct NowPlaying {
    pub title: String,
    pub artist: String,
    pub playing: bool,
    /// BGRA 48x48.
    pub cover: Option<Vec<u8>>,
}
