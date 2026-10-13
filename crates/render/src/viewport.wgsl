// Interactive viewport: an offscreen HDR pass (background, surfaces, wire,
// grid) with reverse-Z depth, an object-id pass for picking and outlines, and
// a composite pass that draws the result into the window.
//
// Reverse-Z: depth 1 is the near plane, 0 is infinitely far; nearer wins with
// `greater`.

struct View {
    view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    view: mat4x4<f32>,
    // xyz: camera position (world); w: 1 for orthographic cameras.
    eye: vec4<f32>,
    // xyz: direction toward the key light (world).
    light: vec4<f32>,
    bg_top: vec4<f32>,
    bg_bottom: vec4<f32>,
    grid_minor: vec4<f32>,
    grid_major: vec4<f32>,
    axis_x: vec4<f32>,
    axis_z: vec4<f32>,
    surface: vec4<f32>,
    // rgb: selection color; a: how strongly selected surfaces are tinted.
    selection: vec4<f32>,
    wire: vec4<f32>,
    // x: grid cell (m); y: grid fade distance (m); zw: target size (px).
    params: vec4<f32>,
    // x: surface mode (0 shaded, 1 matcap, 2 flat paper); y: grid on;
    // z: 1 when the output target is not sRGB (encode in the shader); w: ids valid.
    flags: vec4<u32>,
    // x: surface opacity (below 1 in x-ray); y: vertex dot diameter (px).
    extra: vec4<f32>,
    // Selected elements, and unselected edges and vertices, in element modes.
    element: vec4<f32>,
    edit_wire: vec4<f32>,
};

@group(0) @binding(0) var<uniform> v: View;
// Selection bitset over part ids (part index, not id + 1).
@group(0) @binding(1) var<storage, read> selected: array<u32>;

// A part's world transform; geometry is in object space.
struct PartTransform {
    model: mat4x4<f32>,
    // Inverse transpose of the model matrix (for normals).
    normal: mat4x4<f32>,
    // x: 1 when the transform mirrors (winding flips).
    flags: vec4<u32>,
};
@group(0) @binding(2) var<storage, read> parts: array<PartTransform>;

fn is_selected(part: u32) -> bool {
    let w = part >> 5u;
    if (w >= arrayLength(&selected)) {
        return false;
    }
    return ((selected[w] >> (part & 31u)) & 1u) == 1u;
}

// ---- fullscreen triangle ---------------------------------------------------

struct Full {
    @builtin(position) clip: vec4<f32>,
    // 0..1 across the target, y down.
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_full(@builtin(vertex_index) i: u32) -> Full {
    let x = f32((i << 1u) & 2u);
    let y = f32(i & 2u);
    var o: Full;
    o.clip = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
    o.uv = vec2<f32>(x, y);
    return o;
}

// ---- background ------------------------------------------------------------

@fragment
fn fs_background(i: Full) -> @location(0) vec4<f32> {
    let t = clamp(i.uv.y, 0.0, 1.0);
    return vec4<f32>(mix(v.bg_top.rgb, v.bg_bottom.rgb, t), 1.0);
}

// ---- surfaces ----------------------------------------------------------------

struct SurfaceOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) world: vec3<f32>,
    @location(2) @interpolate(flat) part: u32,
    @location(3) @interpolate(flat) mirrored: u32,
};

@vertex
fn vs_surface(@location(0) pos: vec3<f32>, @location(1) normal: vec3<f32>, @location(2) part: u32) -> SurfaceOut {
    let xf = parts[part];
    let world = (xf.model * vec4<f32>(pos, 1.0)).xyz;
    var o: SurfaceOut;
    o.clip = v.view_proj * vec4<f32>(world, 1.0);
    o.normal = (xf.normal * vec4<f32>(normal, 0.0)).xyz;
    o.world = world;
    o.part = part;
    o.mirrored = xf.flags.x;
    return o;
}

@fragment
fn fs_surface(i: SurfaceOut, @builtin(front_facing) facing: bool) -> @location(0) vec4<f32> {
    // A mirroring transform flips the winding: front stays front.
    let front = facing != (i.mirrored == 1u);
    var n = normalize(i.normal);
    if (!front) {
        n = -n;
    }
    if (v.flags.x == 2u) {
        return vec4<f32>(0.92, 0.92, 0.91, v.extra.x);
    }
    var color: vec3<f32>;
    if (v.flags.x == 1u) {
        let vn = normalize((v.view * vec4<f32>(n, 0.0)).xyz);
        let l = normalize(vec3<f32>(-0.45, 0.55, 0.70));
        let diffuse = max(dot(vn, l), 0.0);
        let h = normalize(l + vec3<f32>(0.0, 0.0, 1.0));
        let spec = pow(max(dot(vn, h), 0.0), 40.0) * 0.35;
        let rim = pow(1.0 - max(vn.z, 0.0), 3.0) * 0.22;
        let clay = vec3<f32>(0.55, 0.53, 0.50);
        color = clay * (0.22 + 0.78 * diffuse) + vec3<f32>(spec + rim);
    } else {
        // Key light plus a sky and ground hemisphere, and a soft highlight.
        var to_eye = normalize(v.eye.xyz - i.world);
        if (v.eye.w > 0.5) {
            to_eye = normalize((transpose(v.view) * vec4<f32>(0.0, 0.0, 1.0, 0.0)).xyz);
        }
        let l = normalize(v.light.xyz);
        let key = max(dot(n, l), 0.0);
        let hemi = mix(vec3<f32>(0.16, 0.15, 0.14), vec3<f32>(0.42, 0.45, 0.50), n.y * 0.5 + 0.5);
        let h = normalize(l + to_eye);
        let spec = pow(max(dot(n, h), 0.0), 48.0) * 0.18;
        color = v.surface.rgb * (hemi + vec3<f32>(0.80) * key) + vec3<f32>(spec);
    }
    if (!front) {
        // The back of open surfaces (planes, cards) is legitimate: shade it a
        // little darker. Inside-out parts are a Health finding, not a color.
        color = color * 0.7;
    }
    if (is_selected(i.part)) {
        color = mix(color, v.selection.rgb, v.selection.a);
    }
    return vec4<f32>(color, v.extra.x);
}

// Part ids (index + 1; 0 is background) for picking and outlines.
@fragment
fn fs_id(i: SurfaceOut) -> @location(0) u32 {
    return i.part + 1u;
}

// ---- wire --------------------------------------------------------------------

struct LineOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) @interpolate(flat) part: u32,
};

// Clip position of an object-space point of `part`, pulled toward the
// camera by the fraction `k` of its distance so edges win over their own
// faces; along the pixel's ray, so it does not move on screen. Surfaces get
// no depth bias: slope-scaled bias let back faces show through at
// silhouettes.
fn pulled(local: vec3<f32>, part: u32, k: f32) -> vec4<f32> {
    let pos = (parts[part].model * vec4<f32>(local, 1.0)).xyz;
    var p = pos + (v.eye.xyz - pos) * k;
    if (v.eye.w > 0.5) {
        let to_eye = normalize((transpose(v.view) * vec4<f32>(0.0, 0.0, 1.0, 0.0)).xyz);
        p = pos + to_eye * (abs(dot(v.eye.xyz - pos, to_eye)) * k);
    }
    return v.view_proj * vec4<f32>(p, 1.0);
}

@vertex
fn vs_line(@location(0) local: vec3<f32>, @location(1) part: u32) -> LineOut {
    var o: LineOut;
    o.clip = pulled(local, part, 0.0008);
    o.part = part;
    return o;
}

@fragment
fn fs_line(i: LineOut) -> @location(0) vec4<f32> {
    if (is_selected(i.part)) {
        return vec4<f32>(v.selection.rgb, 1.0);
    }
    return v.wire;
}

// ---- element overlay (vertex, edge and face modes) ---------------------------

// Bit 31 of the part marks a selected element.
const EDIT_PART: u32 = 0x7fffffffu;

struct EditOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) @interpolate(flat) selected: u32,
    // Position inside a vertex dot, -1 to 1.
    @location(1) uv: vec2<f32>,
};

fn edit_color(selected: u32) -> vec4<f32> {
    if (selected == 1u) {
        return vec4<f32>(v.element.rgb, 1.0);
    }
    return v.edit_wire;
}

@vertex
fn vs_edit_line(@location(0) local: vec3<f32>, @location(1) part: u32) -> EditOut {
    var o: EditOut;
    o.clip = pulled(local, part & EDIT_PART, 0.0012);
    o.selected = part >> 31u;
    o.uv = vec2<f32>(0.0);
    return o;
}

@fragment
fn fs_edit(i: EditOut) -> @location(0) vec4<f32> {
    return edit_color(i.selected);
}

// One instance per vertex: a screen-aligned quad of six vertices.
@vertex
fn vs_edit_point(@builtin(vertex_index) corner: u32, @location(0) local: vec3<f32>, @location(1) part: u32) -> EditOut {
    var quad = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0),
    );
    let c = quad[corner];
    let selected = part >> 31u;
    let size = select(v.extra.y, v.extra.y * 1.35, selected == 1u);
    var clip = pulled(local, part & EDIT_PART, 0.0016);
    // Half the diameter in pixels is size / target in clip units (times w).
    clip = vec4<f32>(clip.xy + c * size / v.params.zw * clip.w, clip.zw);
    var o: EditOut;
    o.clip = clip;
    o.selected = selected;
    o.uv = c;
    return o;
}

@fragment
fn fs_edit_point(i: EditOut) -> @location(0) vec4<f32> {
    let r = length(i.uv);
    if (r > 1.0) {
        discard;
    }
    let c = edit_color(i.selected);
    // A soft rim keeps small dots round.
    return vec4<f32>(c.rgb, c.a * clamp((1.0 - r) * 4.0, 0.0, 1.0));
}

@vertex
fn vs_edit_face(@location(0) local: vec3<f32>, @location(1) part: u32) -> EditOut {
    var o: EditOut;
    o.clip = pulled(local, part & EDIT_PART, 0.0010);
    o.selected = 1u;
    o.uv = vec2<f32>(0.0);
    return o;
}

@fragment
fn fs_edit_face(i: EditOut) -> @location(0) vec4<f32> {
    return vec4<f32>(v.element.rgb, 0.28);
}

// ---- infinite ground grid ------------------------------------------------------

struct GridOut {
    @location(0) color: vec4<f32>,
    @builtin(frag_depth) depth: f32,
};

fn unproject(ndc: vec2<f32>, z: f32) -> vec3<f32> {
    let p = v.inv_view_proj * vec4<f32>(ndc, z, 1.0);
    return p.xyz / p.w;
}

// Coverage of grid lines one pixel wide at `coord` (in cells).
fn lines(coord: vec2<f32>) -> vec2<f32> {
    let d = max(fwidth(coord), vec2<f32>(1e-6));
    let g = abs(fract(coord - 0.5) - 0.5) / d;
    return vec2<f32>(1.0) - min(g, vec2<f32>(1.0));
}

@fragment
fn fs_grid(i: Full) -> GridOut {
    let ndc = vec2<f32>(i.uv.x * 2.0 - 1.0, 1.0 - i.uv.y * 2.0);
    // Two finite points on the pixel's ray (reverse-Z: 1 is the near plane).
    let a = unproject(ndc, 1.0);
    let b = unproject(ndc, 0.5);
    let dir = b - a;
    // Derivatives first, in uniform control flow; misses are dropped at the end.
    let t = select(-1.0, -a.y / dir.y, abs(dir.y) > 1e-9);
    let hit = a + dir * max(t, 0.0);
    let cell = v.params.x;
    let minor = lines(hit.xz / cell);
    let major = lines(hit.xz / (cell * 10.0));
    // Axis lines, 1.5 pixels wide: X runs along z = 0, Z along x = 0.
    let ad = max(fwidth(hit.zx), vec2<f32>(1e-6)) * 1.5;
    let ax = vec2<f32>(1.0) - min(abs(hit.zx) / ad, vec2<f32>(1.0));
    let fade = 1.0 - smoothstep(v.params.y * 0.35, v.params.y, distance(hit.xz, v.eye.xz));
    var color = vec4<f32>(v.grid_minor.rgb, max(minor.x, minor.y) * v.grid_minor.a);
    let maj = max(major.x, major.y) * v.grid_major.a;
    color = mix(color, vec4<f32>(v.grid_major.rgb, maj), step(color.a, maj));
    color = mix(color, vec4<f32>(v.axis_x.rgb, 1.0), ax.x);
    color = mix(color, vec4<f32>(v.axis_z.rgb, 1.0), ax.y);
    color.a = color.a * fade;
    if (t <= 0.0 || color.a < 0.004) {
        discard;
    }
    // Sit a little behind surfaces on the ground plane (a floor hides the
    // grid instead of flickering with it): back along the ray by a small
    // fraction of the distance.
    var to_eye = v.eye.xyz - hit;
    var dist = length(to_eye);
    if (v.eye.w > 0.5) {
        to_eye = normalize((transpose(v.view) * vec4<f32>(0.0, 0.0, 1.0, 0.0)).xyz);
        dist = abs(dot(v.eye.xyz - hit, to_eye));
    } else {
        to_eye = to_eye / max(dist, 1e-6);
    }
    let behind = hit - to_eye * (dist * 0.002);
    let clip = v.view_proj * vec4<f32>(behind, 1.0);
    var o: GridOut;
    o.color = color;
    o.depth = clamp(clip.z / clip.w, 0.0, 1.0);
    return o;
}

// ---- composite into the window -------------------------------------------------

@group(1) @binding(0) var hdr: texture_2d<f32>;
@group(1) @binding(1) var ids: texture_2d<u32>;

fn id_at(p: vec2<i32>) -> u32 {
    let size = vec2<i32>(textureDimensions(ids));
    let q = clamp(p, vec2<i32>(0), size - vec2<i32>(1));
    return textureLoad(ids, q, 0).x;
}

fn encode_srgb(c: vec3<f32>) -> vec3<f32> {
    let lo = c * 12.92;
    let hi = 1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - vec3<f32>(0.055);
    return select(hi, lo, c <= vec3<f32>(0.0031308));
}

@fragment
fn fs_composite(i: Full) -> @location(0) vec4<f32> {
    let size = vec2<f32>(textureDimensions(hdr));
    let p = vec2<i32>(clamp(floor(i.uv * size), vec2<f32>(0.0), size - vec2<f32>(1.0)));
    var color = clamp(textureLoad(hdr, p, 0).rgb, vec3<f32>(0.0), vec3<f32>(1.0));
    if (v.flags.w == 1u) {
        // Selection outline: where the part id changes next to a selected part.
        let c = id_at(p);
        var edge = 0.0;
        var offsets = array<vec2<i32>, 8>(
            vec2<i32>(1, 0), vec2<i32>(-1, 0), vec2<i32>(0, 1), vec2<i32>(0, -1),
            vec2<i32>(2, 0), vec2<i32>(-2, 0), vec2<i32>(0, 2), vec2<i32>(0, -2),
        );
        for (var k = 0u; k < 8u; k = k + 1u) {
            let n = id_at(p + offsets[k]);
            if (n != c && ((n > 0u && is_selected(n - 1u)) || (c > 0u && is_selected(c - 1u)))) {
                edge = max(edge, select(0.55, 1.0, k < 4u));
            }
        }
        color = mix(color, v.selection.rgb, edge);
    }
    if (v.flags.z == 1u) {
        color = encode_srgb(color);
    }
    return vec4<f32>(color, 1.0);
}
