use crate::{Chunk, World, CHUNK_VOLUME, FORMAT_VERSION, GENERATOR_VERSION};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, BufWriter, Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

// Deliberate MVP import/export limits. These are not a streaming-world capacity claim.
const MAX_SAVE_CHUNKS: usize = 512;
const MAX_SAVE_BYTES: u64 = 12 * 1024 * 1024;
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Snapshot {
    format_version: u32,
    generator_version: u32,
    seed: u64,
    revision: u64,
    chunks: Vec<SavedChunk>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedChunk {
    position: [i32; 3],
    voxels: Vec<u8>,
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

struct Temporary(PathBuf);
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

impl World {
    /// Saves a full versioned snapshot using same-directory create-new, sync, then rename.
    /// On write/flush/rename errors the previous destination remains intact. Filesystems
    /// must provide atomic rename (Android private storage does). Directory fsync after
    /// rename is best-effort; sudden power loss is outside this MVP durability guarantee.
    pub fn save(&self, path: impl AsRef<Path>) -> io::Result<()> {
        if self.chunks.len() > MAX_SAVE_CHUNKS {
            return Err(invalid("world exceeds 512-chunk save limit"));
        }
        let snapshot = Snapshot {
            format_version: FORMAT_VERSION,
            generator_version: GENERATOR_VERSION,
            seed: self.seed,
            revision: self.revision,
            chunks: self
                .chunks
                .iter()
                .map(|(&position, chunk)| SavedChunk {
                    position,
                    voxels: chunk.voxels.to_vec(),
                })
                .collect(),
        };
        let path = path.as_ref();
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        if path.file_name().is_none() {
            return Err(invalid("save path needs a file name"));
        }
        let mut temporary = None;
        for _ in 0..16 {
            let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let candidate = parent.join(format!(
                ".matterweave-{}-{sequence}.tmp",
                std::process::id()
            ));
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&candidate)
            {
                Ok(file) => {
                    temporary = Some((Temporary(candidate), file));
                    break;
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        let (temporary, file) = temporary.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::AlreadyExists,
                "cannot reserve temporary save file",
            )
        })?;
        let mut output = BufWriter::new(file);
        serde_json::to_writer(&mut output, &snapshot).map_err(io::Error::other)?;
        output.flush()?;
        output.get_ref().sync_all()?;
        if output.get_ref().metadata()?.len() > MAX_SAVE_BYTES {
            return Err(invalid("serialized world exceeds byte limit"));
        }
        drop(output);
        fs::rename(&temporary.0, path)?;
        if let Ok(directory) = File::open(parent) {
            let _ = directory.sync_all();
        }
        Ok(())
    }

    /// Validates the complete bounded snapshot before returning a replacement world.
    /// Unknown versions, duplicate/invalid chunks, empty chunks and malformed data fail.
    pub fn load(path: impl AsRef<Path>) -> io::Result<Self> {
        let file = File::open(path)?;
        if file.metadata()?.len() > MAX_SAVE_BYTES {
            return Err(invalid("save exceeds byte limit"));
        }
        let mut bytes = Vec::new();
        file.take(MAX_SAVE_BYTES + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_SAVE_BYTES {
            return Err(invalid("save exceeds byte limit"));
        }
        let snapshot: Snapshot = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
        if snapshot.format_version != FORMAT_VERSION
            || snapshot.generator_version != GENERATOR_VERSION
        {
            return Err(invalid("unsupported world or generator version"));
        }
        if snapshot.chunks.len() > MAX_SAVE_CHUNKS {
            return Err(invalid("save exceeds 512-chunk limit"));
        }
        let mut world = Self::new(snapshot.seed);
        for chunk in snapshot.chunks {
            if chunk.voxels.len() != CHUNK_VOLUME {
                return Err(invalid("chunk must contain 4096 materials"));
            }
            if chunk
                .position
                .iter()
                .any(|&coordinate| !(i32::MIN / 16..=i32::MAX / 16).contains(&coordinate))
            {
                return Err(invalid("chunk coordinate outside voxel address space"));
            }
            let solid = chunk
                .voxels
                .iter()
                .filter(|&&material| material != 0)
                .count();
            if solid == 0 {
                return Err(invalid("empty stored chunk"));
            }
            let voxels: Box<[u8; CHUNK_VOLUME]> = chunk
                .voxels
                .into_boxed_slice()
                .try_into()
                .map_err(|_| invalid("invalid chunk size"))?;
            if world
                .chunks
                .insert(chunk.position, Chunk { voxels, solid })
                .is_some()
            {
                return Err(invalid("duplicate chunk"));
            }
        }
        world.revision = snapshot.revision;
        Ok(world)
    }
}
