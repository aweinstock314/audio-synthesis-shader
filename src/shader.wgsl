struct Params {
    samples_elapsed: u32,
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

fn envelope1(t: f32, x: f32) -> f32 {
    let t2 = t % x;
    if t2 < 1.0 {
        return t2;
    } else {
        return 0.0;
    }
}

fn envelope2(t: f32, x: f32) -> f32 {
    let t2 = t % x;
    if t2 < 1.0 {
        return 1.0 - t2;
    } else {
        return 0.0;
    }
}

const TAU: f32 = 6.283185307179586;

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read_write> output: Output;

@compute @workgroup_size(1)
fn compute_main() {
    let time = f32(params.samples_elapsed) / f32(params.sample_rate);
    for(var i=0u; i < params.samples; i += 1u) {
        let t = time + (f32(i) / f32(params.sample_rate));
        let pan = abs(sin(params.slider2 * TAU * t));
        //let pan = 0.0;
        //let a = sin((440.0 + 40.0 * t)*t);
        let freq1 = mix(440.0, 880.0, params.slider1);
        let freq2 = 220.0;
        //let a = sin((freq1 * t) % TAU);
        //let b = sin((freq2* t) % TAU);
        let b = triangle((freq2 * t) % 1.0);
        //let b = square((freq2 * t) % 1.0);
        let a = envelope2(t, 2.0) * sin(((params.slider2 * (2.0 * envelope1(t, 1.0)) + freq1) * t) % TAU);
        if false {
            output.buffer[2u*i+0u] = pan * a + (1.0 - pan) * b;
            output.buffer[2u*i+1u] = (1.0 - pan) * a + pan * b;
        } else {
            output.buffer[2u*i+0u] = a;
            output.buffer[2u*i+1u] = a;
        }
    }
}
