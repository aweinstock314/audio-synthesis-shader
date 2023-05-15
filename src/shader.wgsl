struct Params {
    time: f32,
    sample_rate: u32,
    samples: u32,
}

struct Output {
    buffer: array<f32>,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read_write> output: Output;

@compute @workgroup_size(1)
fn compute_main() {
    for(var i=0u; i < params.samples; i += 1u) {
        let t = params.time + (f32(i) / f32(params.sample_rate));
        let pan = abs(sin(2.0 * 6.28*t));
        let a = sin(440.0*t);
        let b = sin(880.0*t);
        output.buffer[2u*i+0u] = pan * a + (1.0 - pan) * b;
        output.buffer[2u*i+1u] = (1.0 - pan) * a + pan * b;
    }
}
