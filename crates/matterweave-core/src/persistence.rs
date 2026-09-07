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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    streaming: Option<SavedStreaming>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    attachment: Option<serde_json::Value>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedStreaming {
    extension_version: u32,
    center: Option<[i32; 2]>,
    empty_overrides: Vec<[i32; 3]>,
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
        self.save_with_attachment(path, self.attachment.clone())
    }

    pub fn attachment(&self) -> Option<&serde_json::Value> {
        self.attachment.as_ref()
    }

    /// Atomically stores opaque application metadata with the same world snapshot.
    /// The application owns metadata schema validation; the total byte cap applies.
    pub fn save_with_attachment(
        &self,
        path: impl AsRef<Path>,
        attachment: Option<serde_json::Value>,
    ) -> io::Result<()> {
        let mut chunks = Vec::new();
        let streaming = if let Some(stream) = &self.streaming {
            if stream.overrides.len() > MAX_SAVE_CHUNKS {
                return Err(invalid("world exceeds 512-chunk override limit"));
            }
            let mut empty_overrides = Vec::new();
            for (&position, chunk) in &stream.overrides {
                if let Some(chunk) = chunk {
                    chunks.push(SavedChunk {
                        position,
                        voxels: chunk.voxels.to_vec(),
                    });
                } else {
                    empty_overrides.push(position);
                }
            }
            Some(SavedStreaming {
                extension_version: 1,
                center: stream.center,
                empty_overrides,
            })
        } else {
            if self.chunks.len() > MAX_SAVE_CHUNKS {
                return Err(invalid("world exceeds 512-chunk save limit"));
            }
            chunks.extend(self.chunks.iter().map(|(&position, chunk)| SavedChunk {
                position,
                voxels: chunk.voxels.to_vec(),
            }));
            None
        };
        let snapshot = Snapshot {
            format_version: FORMAT_VERSION,
            generator_version: GENERATOR_VERSION,
            seed: self.seed,
            revision: self.revision,
            chunks,
            streaming,
            attachment,
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
        if !(1..=FORMAT_VERSION).contains(&snapshot.format_version)
            || snapshot.generator_version != GENERATOR_VERSION
            || (snapshot.format_version == 1
                && (snapshot.streaming.is_some() || snapshot.attachment.is_some()))
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
        world.chunk_revisions = world
            .chunks
            .keys()
            .map(|&key| (key, world.revision))
            .collect();
        world.attachment = snapshot.attachment;
        if let Some(stream) = snapshot.streaming {
            if stream.extension_version != 1
                || stream
                    .center
                    .is_some_and(|center| center.iter().any(|v| !(-16..16).contains(v)))
                || world.chunks.len() + stream.empty_overrides.len() > MAX_SAVE_CHUNKS
            {
                return Err(invalid("invalid streaming metadata"));
            }
            world.enable_streaming();
            for key in stream.empty_overrides {
                if !World::contains_stream_cell(key.map(|v| v.saturating_mul(16)))
                    || world
                        .streaming
                        .as_mut()
                        .unwrap()
                        .overrides
                        .insert(key, None)
                        .is_some()
                {
                    return Err(invalid("invalid or duplicate empty override"));
                }
            }
            if let Some(center) = stream.center {
                // Reconstruct derived residency without changing the saved revision.
                // Saturated saves remain readonly but still reconstruct correctly.
                world.revision = snapshot.revision.saturating_sub(1);
                world.stream_around([center[0] as f32 * 16.0, 0.0, center[1] as f32 * 16.0]);
                world.revision = snapshot.revision;
                world.chunk_revisions = world
                    .chunks
                    .keys()
                    .map(|&key| (key, world.revision))
                    .collect();
            }
        }
        Ok(world)
    }
}
