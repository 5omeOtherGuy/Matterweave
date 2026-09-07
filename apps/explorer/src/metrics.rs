//! Bounded, opt-in developer frame capture. CPU fields are wall times, not CPU busy time.
use std::{
    fs::{self, File, OpenOptions},
    io::{self, BufWriter, Read, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

const MAX_FRAMES: u32 = 240_000;

pub struct FrameLog {
    writer: BufWriter<File>,
    remaining: u32,
    pub path: PathBuf,
}

impl FrameLog {
    /// Place an integer frame count in `profile-frames.txt` beside the world save.
    /// Consume the request only after a fresh output file is successfully opened.
    /// Normal gameplay does not create a capture or write per-frame data.
    pub fn requested(directory: &Path) -> io::Result<Option<Self>> {
        let request = directory.join("profile-frames.txt");
        let file = match File::open(&request) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        let mut text = String::new();
        file.take(33).read_to_string(&mut text)?;
        if text.len() > 32 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "profile request exceeds 32 bytes",
            ));
        }
        let count = text
            .trim()
            .parse::<u32>()
            .ok()
            .filter(|n| (1..=MAX_FRAMES).contains(n))
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "profile count must be 1..240000",
                )
            })?;
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let path = directory.join(format!("frame-profile-{stamp}.csv"));
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        let mut writer = BufWriter::with_capacity(64 * 1024, file);
        writeln!(writer, "presented_count,draw_interval_wall_ms,main_wall_ms,stream_wall_ms,mesh_upload_wall_ms,save_wall_ms,previous_completed_gpu_ms,previous_completed_shadow_gpu_ms")?;
        writer.flush()?;
        fs::remove_file(request)?;
        Ok(Some(Self {
            writer,
            remaining: count,
            path,
        }))
    }

    /// GPU values describe the prior completed submission, not this CPU sample.
    /// Missing or invalid measurements remain empty CSV cells, never fabricated zeroes.
    pub fn record(&mut self, frame: u64, values: [Option<f64>; 7]) -> io::Result<bool> {
        if self.remaining == 0 {
            return Ok(false);
        }
        write!(self.writer, "{frame}")?;
        for value in values {
            write!(self.writer, ",")?;
            if let Some(value) = value.filter(|value| value.is_finite() && *value >= 0.) {
                write!(self.writer, "{value:.4}")?;
            }
        }
        writeln!(self.writer)?;
        self.remaining -= 1;
        if self.remaining == 0 {
            self.writer.flush()?;
        }
        Ok(self.remaining != 0)
    }

    pub fn flush(&mut self) -> io::Result<()> {
        self.writer.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn directory(name: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("matterweave-metrics-{}-{name}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        path
    }
    #[test]
    fn capture_is_opt_in_bounded_and_keeps_missing_measurements_distinct() {
        let dir = directory("capture");
        assert!(FrameLog::requested(&dir).unwrap().is_none());
        fs::write(dir.join("profile-frames.txt"), "2").unwrap();
        let mut log = FrameLog::requested(&dir).unwrap().unwrap();
        assert!(!dir.join("profile-frames.txt").exists());
        assert!(log
            .record(
                1,
                [
                    Some(16.),
                    Some(2.),
                    Some(0.),
                    None,
                    None,
                    Some(f64::NAN),
                    Some(-1.)
                ]
            )
            .unwrap());
        assert!(!log.record(2, [Some(8.); 7]).unwrap());
        assert!(!log.record(3, [Some(9.); 7]).unwrap());
        let text = fs::read_to_string(&log.path).unwrap();
        assert_eq!(text.lines().count(), 3);
        assert!(text.lines().nth(1).unwrap().ends_with("0.0000,,,,"));
        drop(log);
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn malformed_or_excessive_requests_are_retained_without_opening_a_capture() {
        let dir = directory("invalid");
        for request in [
            "0",
            "240001",
            "not-a-number",
            "999999999999999999999999999999999999999",
        ] {
            fs::write(dir.join("profile-frames.txt"), request).unwrap();
            assert!(FrameLog::requested(&dir).is_err());
            assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
            assert_eq!(
                fs::read_to_string(dir.join("profile-frames.txt")).unwrap(),
                request
            );
        }
        fs::remove_dir_all(dir).unwrap();
    }
}
