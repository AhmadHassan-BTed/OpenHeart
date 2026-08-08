/// Base trait implemented by pipeline stage output artifacts.
pub trait Artifact {
    fn format_version(&self) -> u32;
    fn token_count(&self) -> u32;
    fn file_count(&self) -> u16;
}
