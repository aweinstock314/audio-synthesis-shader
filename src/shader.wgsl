struct Params {
    time: f32,
    sample_rate: u32,
    samples: u32,
    slider1: f32,
    slider2: f32,
}

struct Output {
    buffer: array<f32>,
}

fn triangle(t: f32) -> f32 {
    return 2.0 * abs(t - floor(t + 0.5)) - 0.5;
}

fn square(t: f32) -> f32 {
    if t < 0.5 { return 1.0; } else { return -1.0; };
}

const TAU: f32 = 6.283185307179586;

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read_write> output: Output;

@compute @workgroup_size(1)
fn compute_main() {
    for(var i=0u; i < params.samples; i += 1u) {
        let t = params.time + (f32(i) / f32(params.sample_rate));
        let pan = abs(sin(params.slider2 * TAU * t));
        //let pan = 0.0;
        //let a = sin((440.0 + 40.0 * t)*t);
        let a = sin((mix(440.0, 880.0, params.slider1) * t) % TAU);
        //let b = sin((880.0 * t) % TAU);
        let b = triangle((220.0 * t) % 1.0);
        //let a = square((440.0 * t) % 1.0);
        output.buffer[2u*i+0u] = pan * a + (1.0 - pan) * b;
        output.buffer[2u*i+1u] = (1.0 - pan) * a + pan * b;
        //output.buffer[2u*i+0u] = a;
        //output.buffer[2u*i+1u] = a;
    }
}
