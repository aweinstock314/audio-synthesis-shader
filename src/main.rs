use anyhow::Context;
use bytemuck::{Pod, Zeroable};
use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    StreamConfig,
};
use futures_executor::block_on;
use oddio::{Frames, FramesSignal, Gain, Handle, Mixer, Reinhard, Signal, StreamControl};
use std::{
    borrow::Cow,
    collections::HashMap,
    fs::File,
    io::Cursor,
    io::Read,
    mem,
    num::NonZeroU64,
    sync::mpsc::{self, TryRecvError},
    sync::Arc,
    time::{Duration, Instant},
};

struct EguiApp {
    scene_handle: Handle<Gain<Reinhard<Mixer<[f32; 2]>>>>,
    cpal_stream: cpal::platform::Stream,
    volume: f32,
    cmd_tx: mpsc::Sender<Command>,
    sliders: [f32; 2],
}

impl EguiApp {
    fn new(
        cc: &eframe::CreationContext<'_>,
        scene_handle: Handle<Gain<Reinhard<Mixer<[f32; 2]>>>>,
        cpal_stream: cpal::platform::Stream,
        cmd_tx: mpsc::Sender<Command>,
    ) -> Self {
        EguiApp {
            scene_handle,
            cpal_stream,
            volume: 0.0,
            cmd_tx,
            sliders: [0.0; 2],
        }
    }
}

impl eframe::App for EguiApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        use egui::{CentralPanel, Slider};
        CentralPanel::default().show(ctx, |ui| {
            let volume_slider = Slider::new(&mut self.volume, -60.0..=20.0)
                .prefix("volume: ")
                .suffix("db")
                .show_value(true);
            if ui.add(volume_slider).changed() {
                self.scene_handle
                    .control::<Gain<_>, _>()
                    .set_gain(self.volume);
            }
            if ui.button("Reload shader").clicked() {
                let _ = self.cmd_tx.send(Command::ReloadPipeline);
            }
            for i in 0..2 {
                if ui
                    .add(
                        Slider::new(&mut self.sliders[i], 0.0..=1.0)
                            .prefix(format!("slider{}:", i + 1))
                            .show_value(true),
                    )
                    .changed()
                {
                    let _ = self.cmd_tx.send(Command::SetSlider(i, self.sliders[i]));
                }
            }
        });
    }
}

#[derive(Copy, Clone, Pod, Zeroable)]
#[repr(C)]
struct Params {
    samples_elapsed: u32,
    sample_rate: u32,
    samples: u32,
    sliders: [f32; 2],
}

enum Command {
    ReloadPipeline,
    SetSlider(usize, f32),
}

fn main() -> anyhow::Result<()> {
    let host = cpal::default_host();
    let audio_device = host
        .default_output_device()
        .context("HostTrait::default_output_device")?;
    let config = audio_device
        .default_output_config()
        .context("DeviceTrait::default_output_config")?;
    let sample_rate = config.sample_rate();
    let config = cpal::StreamConfig {
        channels: 2,
        sample_rate,
        buffer_size: cpal::BufferSize::Default,
    };
    println!("{:?}", config);
    let (mut scene_handle, scene) =
        oddio::split(Gain::new(Reinhard::new(Mixer::<[f32; 2]>::new())));
    let cpal_stream = audio_device.build_output_stream(
        &config,
        move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
            let frames = oddio::frame_stereo(data);
            oddio::run(&scene, sample_rate.0, frames);
        },
        move |err| {
            eprintln!("{}", err);
        },
        None,
    )?;
    cpal_stream.play()?;

    let (mut oddio_stream_handle, oddio_stream) = oddio::split(oddio::Stream::new(
        sample_rate.0,
        sample_rate.0 as usize / 4,
    ));

    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::default());
    let adapter = instance
        .enumerate_adapters(wgpu::Backends::PRIMARY)
        .next()
        .expect("wgpu::Instance::enumerate_adapters");
    let (device, queue) = block_on(adapter.request_device(
        &wgpu::DeviceDescriptor {
            label: None,
            features: adapter.features(),
            limits: adapter.limits(),
        },
        None,
    ))
    .expect("wgpu::Adapter::request_device");

    let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: None,
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: NonZeroU64::new(mem::size_of::<Params>() as u64),
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: false },
                    has_dynamic_offset: false,
                    min_binding_size: NonZeroU64::new(4),
                },
                count: None,
            },
        ],
    });
    let params_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: mem::size_of::<Params>() as u64,
        usage: wgpu::BufferUsages::UNIFORM
            | wgpu::BufferUsages::COPY_DST
            | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let output_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 4 * 2 * sample_rate.0 as u64,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_DST
            | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &bind_group_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: params_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: output_buffer.as_entire_binding(),
            },
        ],
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: None,
        bind_group_layouts: &[&bind_group_layout],
        push_constant_ranges: &[],
    });
    let mut pipeline = {
        let shader_source = {
            let mut source = String::new();
            let mut f = File::open("src/shader.wgsl")?;
            f.read_to_string(&mut source)?;
            source
        };
        let shader_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Wgsl(Cow::Owned(shader_source)),
        });
        device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: None,
            layout: Some(&pipeline_layout),
            module: &shader_module,
            entry_point: "compute_main",
        })
    };
    let (cmd_tx, cmd_rx) = std::sync::mpsc::channel();

    let mut belt = wgpu::util::StagingBelt::new(1024);
    std::thread::spawn({
        move || {
            let (tx, rx) = std::sync::mpsc::channel();
            let mut samples_elapsed = 0;
            let samples_per_iter = 1 * sample_rate.0 / 8;
            let mut sliders = [0.0; 2];
            'outer: loop {
                'inner: loop {
                    match cmd_rx.try_recv() {
                        Ok(Command::ReloadPipeline) => {
                            pipeline = {
                                let shader_source = {
                                    let mut source = String::new();
                                    if let Ok(mut f) = File::open("src/shader.wgsl") {
                                        let _ = f.read_to_string(&mut source);
                                        source
                                    } else {
                                        continue 'inner;
                                    }
                                };
                                let shader_module =
                                    device.create_shader_module(wgpu::ShaderModuleDescriptor {
                                        label: None,
                                        source: wgpu::ShaderSource::Wgsl(Cow::Owned(shader_source)),
                                    });
                                device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                                    label: None,
                                    layout: Some(&pipeline_layout),
                                    module: &shader_module,
                                    entry_point: "compute_main",
                                })
                            };
                        }
                        Ok(Command::SetSlider(i, x)) => {
                            sliders[i] = x;
                        }
                        Err(TryRecvError::Empty) => break 'inner,
                        Err(TryRecvError::Disconnected) => break 'outer,
                    }
                }
                let start = Instant::now();
                let mut encoder =
                    device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
                belt.write_buffer(
                    &mut encoder,
                    &params_buffer,
                    0,
                    NonZeroU64::new(mem::size_of::<Params>() as u64).unwrap(),
                    &device,
                )
                .copy_from_slice(bytemuck::bytes_of(&Params {
                    samples_elapsed,
                    sample_rate: sample_rate.0,
                    samples: samples_per_iter,
                    sliders,
                }));
                belt.finish();
                {
                    let mut pass =
                        encoder.begin_compute_pass(&wgpu::ComputePassDescriptor::default());
                    pass.set_pipeline(&pipeline);
                    pass.set_bind_group(0, &bind_group, &[]);
                    pass.dispatch_workgroups(1, 1, 1);
                    drop(pass);
                }
                let _ = queue.submit(vec![encoder.finish()]);
                wgpu::util::DownloadBuffer::read_buffer(
                    &device,
                    &queue,
                    &output_buffer.slice(..),
                    {
                        let tx = tx.clone();
                        move |buf| {
                            if let Ok(buf) = buf {
                                let cast_buf = bytemuck::cast_slice::<u8, [f32; 2]>(&buf);
                                //println!("read_buffer: {:?} {:?}", cast_buf.len(), &cast_buf[0..10]);
                                let _ = tx.send(cast_buf.to_vec());
                            }
                        }
                    },
                );
                device.poll(wgpu::Maintain::Wait);
                let _gpu_time = Instant::now() - start;
                //println!("{} samples in {:?}", samples_per_iter, gpu_time);
                while let Ok(buf) = rx.try_recv() {
                    let mut i = 0;
                    while i < buf.len().min(samples_per_iter as usize) {
                        let ret = oddio_stream_handle
                            .control::<oddio::Stream<_>, _>()
                            .write(&buf[i..samples_per_iter as usize]);
                        if ret == 0 {
                            //std::thread::sleep(Duration::from_millis(50));
                        }
                        i += ret;
                    }
                }
                samples_elapsed += samples_per_iter;
            }
        }
    });

    scene_handle.control::<Mixer<_>, _>().play(oddio_stream);

    let native_options = eframe::NativeOptions::default();
    eframe::run_native(
        "audio-synthesis-shader",
        native_options,
        Box::new(|cc| Box::new(EguiApp::new(cc, scene_handle, cpal_stream, cmd_tx))),
    )
    .unwrap();

    Ok(())
}
