use std::{env, fs, path::PathBuf};

/// `OpEntryPoint` and `OpExecutionMode`, and `ExecutionMode::EarlyFragmentTests`.
const OP_ENTRY_POINT: u32 = 15;
const OP_EXECUTION_MODE: u32 = 16;
const EARLY_FRAGMENT_TESTS: u32 = 9;
const EXECUTION_MODEL_FRAGMENT: u32 = 4;

/// Force the early fragment test on `entry` in this module.
///
/// Naga 24 parses WGSL's `@early_depth_test` but its SPIR-V backend does not
/// emit the execution mode, so insert it here. The two depth-tested full-screen
/// passes (the sky dome and the cloud composite) need the depth test to run
/// before the shader: that is what stops covered pixels from being shaded at
/// all rather than being shaded and then discarded. Neither writes depth nor
/// discards, which is what makes the early test legal and useful.
fn add_early_fragment_tests(words: &mut Vec<u32>, entry: &str) {
    let mut entry_id = None;
    let mut found = Vec::new();
    // Every OpEntryPoint precedes every OpExecutionMode, so the new mode goes
    // right after the last entry point of the module.
    let mut insert_at = 0;
    let mut index = 5; // SPIR-V header.
    while index < words.len() {
        let word_count = (words[index] >> 16) as usize;
        let opcode = words[index] & 0xffff;
        assert!(word_count > 0, "truncated SPIR-V instruction");
        if opcode == OP_ENTRY_POINT {
            // Operands: execution model, entry point id, name, interfaces.
            let bytes: Vec<u8> = words[index + 3..index + word_count]
                .iter()
                .flat_map(|word| word.to_le_bytes())
                .take_while(|b| *b != 0)
                .collect();
            let name = String::from_utf8_lossy(&bytes).into_owned();
            found.push(format!("{}:{}", words[index + 1], name));
            if words[index + 1] == EXECUTION_MODEL_FRAGMENT && name == entry {
                entry_id = Some(words[index + 2]);
            }
            insert_at = index + word_count;
        }
        index += word_count;
    }
    let id = entry_id.unwrap_or_else(|| panic!("no fragment entry {entry} in {found:?}"));
    let mode = (3 << 16) | OP_EXECUTION_MODE;
    words.splice(insert_at..insert_at, [mode, id, EARLY_FRAGMENT_TESTS]);
}

fn main() {
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo OUT_DIR"));
    // (name, fragment entry points). A shader with two fragment entries shares
    // one vertex stage between two pipelines: water with the world pass, the
    // cloud march with the sky dome.
    let shaders: [(&str, &[&str]); 9] = [
        ("world", &["fs_main", "fs_water"]),
        // The sky dome and the volumetric cloud march share a full-screen
        // vertex stage and the group-0 lighting uniform. Both are drawn with a
        // depth EQUAL test after opaque geometry, so both need the early test.
        ("sky", &["fs_main", "fs_clouds"]),
        ("cloud_composite", &["fs_main"]),
        ("upscale", &["fs_main"]),
        ("hud", &["fs_main"]),
        ("shadow", &[]),
        ("ray_reference", &["fs_main"]),
        ("ray_hierarchy_gpu", &["fs_main"]),
        ("comparison_raster", &["fs_main"]),
    ];
    for (name, fragments) in shaders {
        let source = format!("src/{name}.wgsl");
        println!("cargo:rerun-if-changed={source}");
        let text = fs::read_to_string(&source).expect("read WGSL");
        let module = naga::front::wgsl::parse_str(&text)
            .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&text)));
        let info = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::PUSH_CONSTANT,
        )
        .validate(&module)
        .expect("validate shader");
        let entries = std::iter::once(("vs_main", naga::ShaderStage::Vertex)).chain(
            fragments
                .iter()
                .map(|entry| (*entry, naga::ShaderStage::Fragment)),
        );
        for (entry, stage) in entries {
            // Naga's default ADJUST_COORDINATE_SPACE flips Y for Vulkan, preserving
            // the core's right-handed, 0..1 depth camera and top-left HUD convention.
            let mut words = naga::back::spv::write_vec(
                &module,
                &info,
                &Default::default(),
                Some(&naga::back::spv::PipelineOptions {
                    shader_stage: stage,
                    entry_point: entry.into(),
                }),
            )
            .expect("compile SPIR-V");
            // Naga strips each output to the requested entry point, so only the
            // fragment outputs of these two modules carry `fs_main`.
            if matches!(name, "sky" | "cloud_composite")
                && matches!(stage, naga::ShaderStage::Fragment)
                && entry == "fs_main"
            {
                add_early_fragment_tests(&mut words, "fs_main");
            }
            let bytes: Vec<u8> = words.into_iter().flat_map(u32::to_le_bytes).collect();
            fs::write(out.join(format!("{name}.{entry}.spv")), bytes).expect("write SPIR-V");
        }
    }
}
