use std::{env, fs, path::PathBuf};
fn main() {
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo OUT_DIR"));
    for name in ["world", "hud", "shadow"] {
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
        for (entry, stage) in [
            ("vs_main", naga::ShaderStage::Vertex),
            ("fs_main", naga::ShaderStage::Fragment),
        ] {
            if name == "shadow" && stage == naga::ShaderStage::Fragment {
                continue;
            }
            // Naga's default ADJUST_COORDINATE_SPACE flips Y for Vulkan, preserving
            // the core's right-handed, 0..1 depth camera and top-left HUD convention.
            let words = naga::back::spv::write_vec(
                &module,
                &info,
                &Default::default(),
                Some(&naga::back::spv::PipelineOptions {
                    shader_stage: stage,
                    entry_point: entry.into(),
                }),
            )
            .expect("compile SPIR-V");
            let bytes: Vec<u8> = words.into_iter().flat_map(u32::to_le_bytes).collect();
            fs::write(out.join(format!("{name}.{entry}.spv")), bytes).expect("write SPIR-V");
        }
    }
}
