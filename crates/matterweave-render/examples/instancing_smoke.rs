//! Isolated Vulkan validation exercise for static prototype instancing: two
//! prototypes, multiple quarter-turn yaws and translations (including negative
//! coordinates), whole-scene replacement, invalid-update retention, empty
//! clear, and resize. Run under Xvfb/lavapipe; this is not Android device or
//! performance evidence.
use matterweave_core::{Mesh, Vertex};
use matterweave_render::{FrameResult, Hud, LightingSettings, Renderer, StaticInstance};
use std::{
    future::Future,
    sync::Arc,
    task::{Context, Poll, Waker},
};
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, EventLoop},
    window::{Window, WindowId},
};

const IDENTITY: [[f32; 4]; 4] = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
    [0.0, 0.0, 0.0, 1.0],
];

fn box_prototype(extent: [f32; 3], color: [f32; 3], revision: u64) -> Mesh {
    // Unit-axis voxel-style box, centered on the origin, one vertex per corner.
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    for corner in 0..8 {
        let sign = [
            if corner & 1 != 0 { 1.0 } else { -1.0 },
            if corner & 2 != 0 { 1.0 } else { -1.0 },
            if corner & 4 != 0 { 1.0 } else { -1.0 },
        ];
        vertices.push(Vertex {
            position: [
                sign[0] * extent[0],
                sign[1] * extent[1],
                sign[2] * extent[2],
            ],
            normal: [0., 1., 0.],
            color,
        });
    }
    // Six quad faces from corner indices; winding is irrelevant to validation
    // but the count must be exact.
    for face in [
        [0, 1, 3, 2],
        [4, 6, 7, 5],
        [0, 4, 5, 1],
        [2, 3, 7, 6],
        [0, 2, 6, 4],
        [1, 5, 7, 3],
    ] {
        for index in [face[0], face[1], face[2], face[0], face[2], face[3]] {
            indices.push(index as u32);
        }
    }
    Mesh {
        vertices,
        indices,
        revision,
    }
}

fn instances() -> Vec<StaticInstance> {
    vec![
        StaticInstance {
            prototype: 0,
            translation: [0.0, 0.0, 0.0],
            yaw_quarters: 0,
        },
        StaticInstance {
            prototype: 0,
            translation: [-2.5, 0.5, -1.0],
            yaw_quarters: 1,
        },
        StaticInstance {
            prototype: 0,
            translation: [3.0, -0.5, 2.0],
            yaw_quarters: 2,
        },
        StaticInstance {
            prototype: 1,
            translation: [1.5, 1.0, -2.5],
            yaw_quarters: 3,
        },
        StaticInstance {
            prototype: 1,
            translation: [-1.0, -1.0, 3.5],
            yaw_quarters: 1,
        },
    ]
}

#[derive(Default)]
struct App {
    renderer: Option<Renderer>,
    window: Option<Arc<Window>>,
    frame: u32,
    prepared_frame: Option<u32>,
}
impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let window = Arc::new(
            event_loop
                .create_window(
                    Window::default_attributes().with_title("Matterweave instancing validation"),
                )
                .unwrap(),
        );
        let mut initialization = Box::pin(Renderer::new(window.clone()));
        let Poll::Ready(result) = initialization
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
        else {
            panic!("Renderer initialization unexpectedly suspended")
        };
        let mut renderer = result.unwrap();
        println!("{}", renderer.capabilities);
        // Two prototypes, five instances across all four quarter yaws and
        // negative translations, plus a legacy chunk to exercise the identity
        // instance fallback in the same draw loop.
        let stats = renderer
            .replace_static_scene(
                &[
                    box_prototype([0.4, 0.4, 0.4], [0.8, 0.4, 0.3], 1),
                    box_prototype([0.25, 0.9, 0.25], [0.3, 0.7, 0.5], 1),
                ],
                &instances(),
            )
            .unwrap();
        assert_eq!(stats.prototypes, 2);
        assert_eq!(stats.instances, 5);
        assert_eq!(stats.batches, 2);
        assert_eq!(stats.vertices, 16);
        assert_eq!(stats.indices, 72);
        assert!(stats.allocated_bytes > stats.source_bytes.min(1));
        renderer
            .upload_chunk(
                [0, 0, 0],
                &box_prototype([0.2, 0.2, 0.2], [0.4, 0.4, 0.6], 2),
            )
            .unwrap();
        self.renderer = Some(renderer);
        self.window = Some(window);
    }
    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        if !matches!(event, WindowEvent::RedrawRequested) {
            return;
        }
        let renderer = self.renderer.as_mut().unwrap();
        if self.prepared_frame != Some(self.frame) {
            match self.frame {
                1 => {
                    // Whole-scene replacement with different source geometry
                    // and instance set, while a previous frame may be in flight.
                    let stats = renderer
                        .replace_static_scene(
                            &[box_prototype([0.3, 0.3, 0.3], [0.9, 0.2, 0.2], 3)],
                            &[
                                StaticInstance {
                                    prototype: 0,
                                    translation: [0.0, 0.5, 0.0],
                                    yaw_quarters: 0,
                                },
                                StaticInstance {
                                    prototype: 0,
                                    translation: [-3.0, 0.0, -3.0],
                                    yaw_quarters: 2,
                                },
                            ],
                        )
                        .unwrap();
                    assert_eq!(
                        (stats.prototypes, stats.instances, stats.batches),
                        (1, 2, 1)
                    );
                }
                2 => {
                    // Invalid updates must retain the previous scene wholesale.
                    let before = renderer.static_scene_stats().unwrap();
                    let mut broken = box_prototype([0.3, 0.3, 0.3], [0.9, 0.2, 0.2], 4);
                    broken.indices.push(500);
                    assert!(renderer
                        .replace_static_scene(&[broken], &instances())
                        .is_err());
                    assert_eq!(renderer.static_scene_stats(), Some(before));
                    assert!(renderer
                        .replace_static_scene(
                            &[box_prototype([0.3, 0.3, 0.3], [0.9, 0.2, 0.2], 4)],
                            &[StaticInstance {
                                prototype: 7,
                                translation: [0.0; 3],
                                yaw_quarters: 0
                            }],
                        )
                        .is_err());
                    assert_eq!(renderer.static_scene_stats(), Some(before));
                    assert!(renderer
                        .replace_static_scene(
                            &[box_prototype([0.3, 0.3, 0.3], [0.9, 0.2, 0.2], 4)],
                            &[StaticInstance {
                                prototype: 0,
                                translation: [f32::NAN; 3],
                                yaw_quarters: 0
                            }],
                        )
                        .is_err());
                    assert_eq!(renderer.static_scene_stats(), Some(before));
                    assert!(renderer
                        .replace_static_scene(
                            &[box_prototype([0.3, 0.3, 0.3], [0.9, 0.2, 0.2], 4)],
                            &[StaticInstance {
                                prototype: 0,
                                translation: [0.0; 3],
                                yaw_quarters: 5
                            }],
                        )
                        .is_err());
                    assert_eq!(renderer.static_scene_stats(), Some(before));
                }
                3 => {
                    // Empty instance list clears the scene safely.
                    let stats = renderer.replace_static_scene(&[], &[]).unwrap();
                    assert_eq!(stats, Default::default());
                    assert_eq!(renderer.static_scene_stats(), None);
                }
                4 => {
                    let _ = self
                        .window
                        .as_ref()
                        .unwrap()
                        .request_inner_size(winit::dpi::PhysicalSize::new(800, 600));
                    renderer.resize(800, 600);
                }
                5 => {
                    renderer.resize(0, 0);
                    assert_eq!(
                        renderer.render(IDENTITY, [0.; 3], &Hud::new(1., 1.)),
                        FrameResult::Retry
                    );
                    renderer.resize(800, 600);
                }
                6 => {
                    // Upload both LOD-like prototypes, initially using only one.
                    renderer
                        .replace_static_scene(
                            &[
                                box_prototype([0.4; 3], [0.8, 0.4, 0.3], 8),
                                box_prototype([0.2; 3], [0.3, 0.7, 0.5], 8),
                            ],
                            &[instances()[0]],
                        )
                        .unwrap();
                }
                7 => {
                    let before = renderer.static_scene_stats().unwrap();
                    let after = renderer
                        .update_static_instances(&[StaticInstance {
                            prototype: 1,
                            translation: [-0.4, 0., 0.],
                            yaw_quarters: 3,
                        }])
                        .unwrap();
                    assert_eq!(
                        (after.vertices, after.indices, after.source_bytes),
                        (before.vertices, before.indices, before.source_bytes)
                    );
                    assert_eq!(after.allocated_bytes, before.allocated_bytes);
                    assert!(renderer
                        .update_static_instances(&[StaticInstance {
                            prototype: 2,
                            translation: [0.; 3],
                            yaw_quarters: 0,
                        }])
                        .is_err());
                    assert_eq!(renderer.static_scene_stats(), Some(after));
                }
                8 => {
                    let before = renderer.static_scene_stats().unwrap();
                    let after = renderer.update_static_instances(&instances()).unwrap();
                    assert_eq!(after.instances, 5);
                    assert_eq!(after.source_bytes, before.source_bytes);
                    assert_eq!(after.allocated_bytes, before.allocated_bytes + 4 * 16);
                }
                9 => {
                    let before = renderer.static_scene_stats().unwrap();
                    let after = renderer.update_static_instances(&[]).unwrap();
                    assert_eq!(after.instances, 0);
                    assert_eq!(after.allocated_bytes, before.allocated_bytes);
                }
                10 => {
                    renderer.update_static_instances(&[instances()[0]]).unwrap();
                }
                _ => {}
            }
            self.prepared_frame = Some(self.frame);
        }
        let size = self.window.as_ref().unwrap().inner_size();
        let lighting = LightingSettings {
            shadows: matches!(self.frame, 0 | 2 | 4 | 6..=10),
            ..Default::default()
        };
        match renderer.render_with_lighting(
            IDENTITY,
            [0.0; 3],
            &Hud::new(size.width as f32, size.height as f32),
            &lighting,
        ) {
            FrameResult::Presented => {
                if self.frame == 0 {
                    // Both static batches plus the chunk caster are submitted;
                    // instanced scene bounds participate in shadow fitting.
                    assert_eq!(renderer.shadow_caster_meshes(), 3);
                }
                if !lighting.shadows {
                    assert_eq!(renderer.shadow_caster_meshes(), 0);
                }
                self.frame += 1;
            }
            FrameResult::Retry => return,
            result => panic!("Unexpected render result: {result:?}"),
        }
        if self.frame == 11 {
            self.renderer = None;
            self.window = None;
            println!(
                "instancing_smoke: eleven frames passed; instance-only reselect/grow/hide/restore; two prototypes/five instances/four yaws, \
                 whole-scene replacement, invalid-update retention (index/prototype/NaN/yaw), \
                 empty clear, identity chunk fallback, shadow batches, resize/zero extent/recreation"
            );
            event_loop.exit();
        }
    }
    fn about_to_wait(&mut self, _: &ActiveEventLoop) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
}

fn main() {
    EventLoop::new()
        .unwrap()
        .run_app(&mut App::default())
        .unwrap();
}
