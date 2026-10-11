//! User-facing CLI text. Logging stays on `tracing`.

/// Write an error line to stderr.
pub fn error_line(message: &str) {
    #[expect(clippy::print_stderr, reason = "CLI errors are user-facing stderr")]
    {
        eprintln!("rig: {message}");
    }
}
