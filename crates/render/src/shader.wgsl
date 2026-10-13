// Surfaces and edges for headless renders.
//
// mode 0: shaded (a soft color per part, key light plus sky and ground light)
// mode 1: matcap (gray clay lit in view space, like a studio matcap)
// mode 2: paper (flat, for hidden-line wireframes)

struct Globals {
    view_proj: mat4x4<f32>,
    view: mat4x4<f32>,
    // xyz: direction toward the key light (world); w: unused.
    light: vec4<f32>,
    // x: surface mode; y: 1 to tint lines by part.
    mode: vec4<u32>,
    line_color: vec4<f32>,
};

@group(0) @binding(0) var<uniform> g: Globals;

struct SurfaceOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) @interpolate(flat) part: u32,
    // Non-negative on two-sided cards (hair, leaves): their back faces are
    // lit like fronts instead of showing the inside-out warning tint.
    @location(2) strand: f32,
};

@vertex
fn vs_surface(@location(0) pos: vec3<f32>, @location(1) normal: vec3<f32>, @location(2) part: u32, @location(3) strand: f32) -> SurfaceOut {
    var o: SurfaceOut;
    o.clip = g.view_proj * vec4<f32>(pos, 1.0);
    o.normal = normal;
    o.part = part;
    o.strand = strand;
    return o;
}

// Soft, distinct colors so neighbouring parts read apart (linear RGB).
fn part_color(part: u32) -> vec3<f32> {
    switch part % 8u {
        case 0u: { return vec3<f32>(0.62, 0.62, 0.64); }
        case 1u: { return vec3<f32>(0.70, 0.52, 0.36); }
        case 2u: { return vec3<f32>(0.40, 0.55, 0.72); }
        case 3u: { return vec3<f32>(0.50, 0.68, 0.45); }
        case 4u: { return vec3<f32>(0.75, 0.62, 0.35); }
        case 5u: { return vec3<f32>(0.62, 0.45, 0.66); }
        case 6u: { return vec3<f32>(0.42, 0.66, 0.66); }
        default: { return vec3<f32>(0.72, 0.45, 0.45); }
    }
}

@fragment
fn fs_surface(i: SurfaceOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    var n = normalize(i.normal);
    if (!front) {
        n = -n;
    }
    if (g.mode.x == 2u) {
        return vec4<f32>(0.96, 0.96, 0.95, 1.0);
    }
    if (g.mode.x == 1u) {
        let v = normalize((g.view * vec4<f32>(n, 0.0)).xyz);
        let l = normalize(vec3<f32>(-0.45, 0.55, 0.70));
        let diffuse = max(dot(v, l), 0.0);
        let h = normalize(l + vec3<f32>(0.0, 0.0, 1.0));
        let spec = pow(max(dot(v, h), 0.0), 40.0) * 0.35;
        let rim = pow(1.0 - max(v.z, 0.0), 3.0) * 0.22;
        let clay = vec3<f32>(0.55, 0.53, 0.50);
        return vec4<f32>(clay * (0.22 + 0.78 * diffuse) + vec3<f32>(spec + rim), 1.0);
    }
    let base = part_color(i.part);
    let key = max(dot(n, normalize(g.light.xyz)), 0.0);
    let sky = mix(vec3<f32>(0.20, 0.19, 0.18), vec3<f32>(0.36, 0.38, 0.42), n.y * 0.5 + 0.5);
    // Back faces (inside-out surfaces) show in a warning tint.
    var color = base * (sky + vec3<f32>(0.85) * key);
    if (!front && i.strand < 0.0) {
        color = vec3<f32>(0.80, 0.06, 0.08) * (0.5 + 0.5 * key);
    }
    return vec4<f32>(color, 1.0);
}

struct LineOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) @interpolate(flat) part: u32,
};

@vertex
fn vs_line(@location(0) pos: vec3<f32>, @location(1) part: u32) -> LineOut {
    var o: LineOut;
    o.clip = g.view_proj * vec4<f32>(pos, 1.0);
    o.part = part;
    return o;
}

@fragment
fn fs_line(i: LineOut) -> @location(0) vec4<f32> {
    if (g.mode.y == 1u) {
        return vec4<f32>(part_color(i.part) * 0.35, 1.0);
    }
    return g.line_color;
}

// Part ids (index + 1; 0 is background) for label placement.
@fragment
fn fs_id(i: SurfaceOut) -> @location(0) u32 {
    return i.part + 1u;
}
