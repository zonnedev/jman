use std::{
    io::{self, IsTerminal},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};

#[derive(Clone, Debug)]
pub struct Ui {
    quiet: bool,
    verbose: u8,
    animate: bool,
    structured: bool,
}

impl Ui {
    pub fn new(quiet: bool, verbose: u8, no_progress: bool, structured: bool) -> Self {
        Self {
            quiet,
            verbose,
            animate: !quiet
                && !structured
                && !no_progress
                && io::stderr().is_terminal()
                && std::env::var_os("CI").is_none(),
            structured,
        }
    }

    pub fn activity(&self, message: impl Into<String>) -> Activity {
        let message = message.into();
        if self.quiet || self.structured {
            return Activity::hidden();
        }
        if self.animate {
            let progress =
                ProgressBar::with_draw_target(None, ProgressDrawTarget::stderr_with_hz(12));
            progress.set_style(spinner_style());
            progress.enable_steady_tick(Duration::from_millis(80));
            progress.set_message(message);
            Activity {
                progress: Some(progress),
                started: Instant::now(),
                plain: false,
                downloading: AtomicBool::new(false),
            }
        } else {
            eprintln!("→ {message}");
            Activity {
                progress: None,
                started: Instant::now(),
                plain: true,
                downloading: AtomicBool::new(false),
            }
        }
    }

    pub fn success(&self, message: impl AsRef<str>) {
        if !self.quiet && !self.structured {
            eprintln!("✓ {}", message.as_ref());
        }
    }

    pub fn warning(&self, message: impl AsRef<str>) {
        if !self.quiet && !self.structured {
            eprintln!("! {}", message.as_ref());
        }
    }

    pub fn detail(&self, message: impl AsRef<str>) {
        if self.verbose > 0 && !self.quiet && !self.structured {
            eprintln!("  {}", message.as_ref());
        }
    }

    pub fn line(&self, message: impl AsRef<str>) {
        if !self.quiet && !self.structured {
            eprintln!("{}", message.as_ref());
        }
    }

    pub fn is_verbose(&self) -> bool {
        self.verbose > 0
    }
}

pub struct Activity {
    progress: Option<ProgressBar>,
    started: Instant,
    plain: bool,
    downloading: AtomicBool,
}

impl Activity {
    fn hidden() -> Self {
        Self {
            progress: None,
            started: Instant::now(),
            plain: false,
            downloading: AtomicBool::new(false),
        }
    }

    pub fn set_message(&self, message: impl Into<String>) {
        if let Some(progress) = &self.progress {
            progress.set_message(message.into());
        }
    }

    pub fn line(&self, message: impl AsRef<str>) {
        let message = message.as_ref();
        if let Some(progress) = &self.progress {
            progress.println(message);
        } else if self.plain {
            eprintln!("{message}");
        }
    }

    pub fn set_download_progress(&self, downloaded: u64, total: Option<u64>) {
        let Some(progress) = &self.progress else {
            return;
        };
        self.downloading.store(true, Ordering::Relaxed);
        if let Some(total) = total {
            progress.set_length(total);
            progress.set_style(download_style());
        } else {
            progress.set_style(unknown_download_style());
        }
        progress.set_message("Downloading JDK");
        progress.set_position(downloaded);
    }

    pub fn finish(mut self, message: impl Into<String>) {
        let message = message.into();
        if let Some(progress) = self.progress.take() {
            let message = format!("{} ({})", message, format_duration(self.started.elapsed()));
            if self.downloading.load(Ordering::Relaxed) {
                progress.finish_and_clear();
                eprintln!("✓ {message}");
            } else {
                progress.finish_with_message(message);
            }
        } else if self.plain {
            eprintln!(
                "✓ {} ({})",
                message,
                format_duration(self.started.elapsed())
            );
        }
    }

    pub fn finish_clean(mut self, message: impl AsRef<str>) {
        let visible = self.plain || self.progress.is_some();
        if let Some(progress) = self.progress.take() {
            progress.finish_and_clear();
        }
        if visible {
            eprintln!(
                "✓ {} ({})",
                message.as_ref(),
                format_duration(self.started.elapsed())
            );
        }
    }
}

impl Drop for Activity {
    fn drop(&mut self) {
        if let Some(progress) = &self.progress {
            progress.finish_and_clear();
        }
    }
}

fn spinner_style() -> ProgressStyle {
    ProgressStyle::with_template("{spinner:.cyan} {msg}")
        .expect("static progress template")
        .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"])
}

fn download_style() -> ProgressStyle {
    ProgressStyle::with_template(
        "{spinner:.cyan} {msg} [{wide_bar:.cyan/blue}] \
         {bytes}/{total_bytes} {bytes_per_sec} ETA {eta}",
    )
    .expect("static download template")
    .progress_chars("━━╸")
    .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"])
}

fn unknown_download_style() -> ProgressStyle {
    ProgressStyle::with_template("{spinner:.cyan} {msg} {bytes} {bytes_per_sec}")
        .expect("static download template")
        .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"])
}

pub fn format_duration(duration: Duration) -> String {
    if duration.as_secs() > 0 {
        format!("{:.1}s", duration.as_secs_f64())
    } else if duration.as_millis() > 0 {
        format!("{}ms", duration.as_millis())
    } else if duration.as_nanos() > 0 {
        format!("{}µs", duration.as_micros().max(1))
    } else {
        "0ms".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_short_and_long_durations_consistently() {
        assert_eq!(format_duration(Duration::from_millis(24)), "24ms");
        assert_eq!(format_duration(Duration::from_millis(1250)), "1.2s");
        assert_eq!(format_duration(Duration::from_micros(742)), "742µs");
        assert_eq!(format_duration(Duration::from_nanos(1)), "1µs");
    }

    #[test]
    fn structured_output_never_animates() {
        let ui = Ui::new(false, 0, false, true);
        assert!(!ui.animate);
        assert!(ui.structured);
    }

    #[test]
    fn download_style_is_valid() {
        let _ = download_style();
        let _ = unknown_download_style();
    }
}
