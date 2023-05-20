use anyhow::Context;
use bytemuck::{Pod, Zeroable};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use egui::plot::Line;
use futures_executor::block_on;
use oddio::{Gain, Handle, Mixer, Reinhard};
use std::{
    borrow::Cow,
    collections::HashMap,
    fs::File,
    io::Read,
    mem,
    num::NonZeroU64,
    sync::mpsc::{self, TryRecvError},
    time::Instant,
};

struct EguiApp {
    scene_handle: Handle<Gain<Reinhard<Mixer<[f32; 2]>>>>,
    #[allow(unused)]
    cpal_stream: cpal::platform::Stream,
    volume: f32,
    cmd_tx: mpsc::Sender<Command>,
    ui_rx: mpsc::Receiver<UiCommand>,
    sliders: [f32; 2],
    plots: Option<(Vec<f32>, Vec<f32>)>,
    entry_points: Vec<String>,
    current_pipeline: String,
}

impl EguiApp {
    fn new(
        cc: &eframe::CreationContext<'_>,
        scene_handle: Handle<Gain<Reinhard<Mixer<[f32; 2]>>>>,
        cpal_stream: cpal::platform::Stream,
        cmd_tx: mpsc::Sender<Command>,
        ui_rx: mpsc::Receiver<UiCommand>,
        entry_points: Vec<String>,
        current_pipeline: String,
    ) -> Self {
        let ctx = cc.egui_ctx.clone();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            while let Ok(plot) = ui_rx.recv() {
                let _ = tx.send(plot);
                ctx.request_repaint();
            }
        });
        EguiApp {
            scene_handle,
            cpal_stream,
            volume: 0.0,
            cmd_tx,
            ui_rx: rx,
            sliders: [0.0; 2],
            plots: None,
            entry_points,
            current_pipeline,
        }
    }
}

impl eframe::App for EguiApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        use egui::{CentralPanel, Slider};
        while let Ok(cmd) = self.ui_rx.try_recv() {
            match cmd {
                UiCommand::PlotData(mut plot) => {
                    let (mut left, mut right) = (Vec::new(), Vec::new());
                    for x in plot.drain(..) {
                        left.push(x[0]);
                        right.push(x[1]);
                    }
                    self.plots = Some((left, right));
                }
                UiCommand::NewPipelines(entry_points, current_pipeline) => {
                    self.entry_points = entry_points;
                    self.current_pipeline = current_pipeline;
                }
            }
        }
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
                let slider = Slider::new(&mut self.sliders[i], 0.0..=1.0)
                    .prefix(format!("slider{}:", i + 1))
                    .show_value(true);
                if ui.add(slider).changed() {
                    let _ = self.cmd_tx.send(Command::SetSlider(i, self.sliders[i]));
                }
            }
            ui.horizontal(|ui| {
                for entry_point in self.entry_points.iter() {
                    if ui
                        .radio_value(&mut self.current_pipeline, entry_point.clone(), entry_point)
                        .changed()
                    {
                        let _ = self
                            .cmd_tx
                            .send(Command::SetCurrentPipeline(self.current_pipeline.clone()));
                    }
                }
            });
            if let Some((left, right)) = &self.plots {
                use egui::plot::{Plot, PlotPoints};
                let plot_aspect = ui.available_width() / 200.0;
                let left_points = PlotPoints::from_ys_f32(&left);
                let right_points = PlotPoints::from_ys_f32(&right);
                let left_line = Line::new(left_points);
                let right_line = Line::new(right_points);
                Plot::new("left_channel")
                    .view_aspect(plot_aspect)
                    .show(ui, |plot_ui| plot_ui.line(left_line));
                Plot::new("right_channel")
                    .view_aspect(plot_aspect)
                    .show(ui, |plot_ui| plot_ui.line(right_line));
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

#[derive(Copy, Clone, Pod, Zeroable)]
#[repr(C)]
struct Phases {
    phases: [f32; 3],
}

enum Command {
    ReloadPipeline,
    SetSlider(usize, f32),
    SetCurrentPipeline(String),
}

enum UiCommand {
    PlotData(Vec<[f32; 2]>),
    NewPipelines(Vec<String>, String),
}

struct Pipelines {
    pipelines: HashMap<String, wgpu::ComputePipeline>,
    current_pipeline: String,
    entry_points: Vec<String>,
}

impl Pipelines {
    fn from_str(
        device: &wgpu::Device,
        pipeline_layout: &wgpu::PipelineLayout,
        shader_source: &str,
    ) -> anyhow::Result<Self> {
        match naga::front::wgsl::Parser::new().parse(&shader_source) {
            Ok(module) => {
                let entry_points = module
                    .entry_points
                    .iter()
                    .map(|ep| ep.name.clone())
                    .collect::<Vec<String>>();
                println!("entry_points: {:?}", entry_points);
                anyhow::ensure!(!entry_points.is_empty(), "No entry points found");
                let shader_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: None,
                    source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(shader_source)),
                });
                let mut ret = Pipelines {
                    pipelines: HashMap::new(),
                    current_pipeline: entry_points[0].clone(),
                    entry_points: entry_points.clone(),
                };
                for entry_point in entry_points.into_iter() {
                    let pipeline =
                        device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                            label: None,
                            layout: Some(&pipeline_layout),
                            module: &shader_module,
                            entry_point: &entry_point,
                        });
                    ret.pipelines.insert(entry_point, pipeline);
                }
                Ok(ret)
            }
            Err(e) => {
                e.emit_to_stderr(&shader_source);
                Err(e)?
            }
        }
    }
    fn keys(&self) -> Vec<String> {
        self.entry_points.clone()
    }
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
                    min_binding_size: NonZeroU64::new(mem::size_of::<Phases>() as u64),
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
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
    let phases_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: mem::size_of::<Phases>() as u64,
        usage: wgpu::BufferUsages::STORAGE
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
                resource: phases_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: output_buffer.as_entire_binding(),
            },
        ],
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: None,
        bind_group_layouts: &[&bind_group_layout],
        push_constant_ranges: &[],
    });
    let mut pipelines = {
        let shader_source = {
            let mut source = String::new();
            let mut f = File::open("src/shader.wgsl")?;
            f.read_to_string(&mut source)?;
            source
        };
        Pipelines::from_str(&device, &pipeline_layout, &shader_source)?
    };
    let (cmd_tx, cmd_rx) = std::sync::mpsc::channel();
    let (ui_tx, ui_rx) = std::sync::mpsc::channel();
    let initial_entry_points = pipelines.keys();
    let initial_pipeline = pipelines.current_pipeline.clone();

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
                            let shader_source = {
                                let mut source = String::new();
                                if let Ok(mut f) = File::open("src/shader.wgsl") {
                                    let _ = f.read_to_string(&mut source);
                                    source
                                } else {
                                    continue 'inner;
                                }
                            };
                            match Pipelines::from_str(&device, &pipeline_layout, &shader_source) {
                                Ok(new_pipelines) => {
                                    pipelines = new_pipelines;
                                    let _ = ui_tx.send(UiCommand::NewPipelines(
                                        pipelines.keys(),
                                        pipelines.current_pipeline.clone(),
                                    ));
                                }
                                Err(_) => continue 'inner,
                            }
                        }
                        Ok(Command::SetSlider(i, x)) => {
                            sliders[i] = x;
                        }
                        Ok(Command::SetCurrentPipeline(current_pipeline)) => {
                            pipelines.current_pipeline = current_pipeline;
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
                    pass.set_pipeline(
                        &pipelines
                            .pipelines
                            .get(&pipelines.current_pipeline)
                            .unwrap(),
                    );
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
                        let ui_tx = ui_tx.clone();
                        move |buf| {
                            if let Ok(buf) = buf {
                                let cast_buf = &bytemuck::cast_slice::<u8, [f32; 2]>(&buf)
                                    [..samples_per_iter as usize];
                                //println!("read_buffer: {:?} {:?}", cast_buf.len(), &cast_buf[0..10]);
                                let _ = tx.send(cast_buf.to_vec());
                                let _ = ui_tx.send(UiCommand::PlotData(cast_buf.to_vec()));
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
        Box::new(|cc| {
            Box::new(EguiApp::new(
                cc,
                scene_handle,
                cpal_stream,
                cmd_tx,
                ui_rx,
                initial_entry_points,
                initial_pipeline,
            ))
        }),
    )
    .unwrap();

    Ok(())
}
