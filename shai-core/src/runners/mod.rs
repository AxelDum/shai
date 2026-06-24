pub mod clifixer;
pub mod coder;
pub mod compacter;
pub mod gerund;
pub mod searcher;

#[cfg(test)]
pub(crate) mod test_helpers {
    /// Mutex to serialize tests that call `std::env::set_current_dir`.
    /// Since the working directory is process-wide, parallel tests would
    /// otherwise interfere with each other.
    pub static DIR_TEST_MUTEX: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
}
