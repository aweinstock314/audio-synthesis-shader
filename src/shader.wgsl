struct Params {
    samples_elapsed: u32,
    sample_rate: u32,
    samples: u32,
    slider1: f32,
    slider2: f32,
}

struct Phases {
    phase1: f32,
    phase2: f32,
    phase3: f32,
}

struct Output {
    buffer: array<f32>,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read_write> phases: Phases;
@group(0) @binding(2) var<storage, read_write> output: Output;

fn triangle(t: f32) -> f32 {
    return 2.0 * abs(t - floor(t + 0.5)) - 0.5;
}

fn square(t: f32) -> f32 {
    if t < 0.5 { return 1.0; } else { return -1.0; };
}

fn envelope1(t: f32, x: f32) -> f32 {
    let t2 = t % x;
    if t2 <= 1.0 {
        return clamp(t2, 0.0, 1.0);
    } else {
        return 0.0;
    }
}

fn envelope2(t: f32, x: f32, k: f32) -> f32 {
    let t2 = t % x;
    if t2 <= k {
        return (k - t2)/k;
    } else {
        return 0.0;
    }
}

const TAU: f32 = 6.283185307179586;

@compute @workgroup_size(1)
fn silence() {
    for(var i=0u; i < params.samples; i += 1u) {
        output.buffer[2u*i+0u] = 0.0;
        output.buffer[2u*i+1u] = 0.0;
    }
}

@compute @workgroup_size(1)
fn pan_test() {
    //let time = f32(params.samples_elapsed) / f32(params.sample_rate);
    for(var i=0u; i < params.samples; i += 1u) {
        //let t = time + (f32(i) / f32(params.sample_rate));
        let pan = abs(sin(params.slider2 * TAU * phases.phase3));
        //let pan = 0.0;
        //let a = sin((440.0 + 40.0 * t)*t);
        let freq1 = mix(440.0, 880.0, params.slider1);
        let freq2 = 220.0;
        phases.phase1 = (phases.phase1 + freq1 / f32(params.sample_rate)) % TAU;
        phases.phase2 = (phases.phase2 + freq2 / f32(params.sample_rate)) % 1.0;
        phases.phase3 = (phases.phase3 + 1.0 / f32(params.sample_rate)) % TAU;
        let a = sin(phases.phase1);
        //let b = sin((freq2* t) % TAU);
        let b = triangle(phases.phase2);
        //let b = square((freq2 * t) % 1.0);
        //let a = /*envelope2(t, 2.0) **/ sin(((params.slider2 * (32.0 * sin(t%TAU)) + freq1) * t) % TAU);
        //let a = /*envelope2(t, 2.0) **/ sin((((64.0 * triangle((mix(220.0, 330.0, params.slider2) * t) % 1.0)) + freq1) * t) % TAU);
        //let a = /*envelope2(t, 2.0) **/ envelope1(t, 1.0);
        //let a = triangle(((1.0 * sin((48.0*(t+0.1)) % TAU) + freq1) * t) % 1.0);
        //let a = sin((440.0 * t) % TAU);
        //let a = sin((1000.0 * t) % TAU);
        //let freq3 = (128.0 * triangle((mix(1.0, 330.0, params.slider2) * t) % 1.0)) + 440.0;
        if true {
            output.buffer[2u*i+0u] = pan * a + (1.0 - pan) * b;
            output.buffer[2u*i+1u] = (1.0 - pan) * a + pan * b;
        } else {
            output.buffer[2u*i+0u] = a;
            output.buffer[2u*i+1u] = a;
        }
    }
}

@compute @workgroup_size(1)
fn fm_test() {
    for(var i=0u; i < params.samples; i += 1u) {
        let freq3 = mix(220.0, 440.0, params.slider1) * pow(phases.phase2 % 1.0, 0.25) + mix(440.0, 880.0, params.slider2);
        phases.phase1 = (phases.phase1 + freq3/f32(params.sample_rate)) % TAU;
        phases.phase2 += 1.0 / f32(params.sample_rate);
        if phases.phase2 >= 0.25 {
            phases.phase1 = 0.0;
        }
        phases.phase2 %= 1.0;
        let a = envelope2(phases.phase2, 0.25, 0.2) * sin(phases.phase1);
        output.buffer[2u*i+0u] = a;
        output.buffer[2u*i+1u] = a;
    }
}

@compute @workgroup_size(1)
fn mul_test() {
    for(var i=0u; i < params.samples; i += 1u) {
        let freq1 = mix(220.0, 1760.0, params.slider1);
        let freq2 = mix(220.0, 1760.0, params.slider2);
        phases.phase1 = (phases.phase1 + freq1 / f32(params.sample_rate)) % TAU;
        phases.phase2 = (phases.phase2 + freq2 / f32(params.sample_rate)) % 1.0;
        let a = sin(phases.phase1);
        let b = 0.25 * triangle(phases.phase2);
        output.buffer[2u*i+0u] = a * b;
        output.buffer[2u*i+1u] = a * b;
    }
}
