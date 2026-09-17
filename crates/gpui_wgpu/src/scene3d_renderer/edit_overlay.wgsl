struct View { size: vec2<f32>, density: vec2<f32> }
@group(0) @binding(0) var<uniform> view: View;
@group(0) @binding(1) var depths: texture_2d<f32>;
@group(0) @binding(2) var identities: texture_2d<u32>;
@group(0) @binding(3) var<storage,read> auxiliary_ids: array<u32>;

fn is_auxiliary(id: u32) -> bool {
    if id == 0u { return false; }
    var lo=0u;var hi=arrayLength(&auxiliary_ids);
    loop {
        if lo>=hi {break;}
        let mid=lo+(hi-lo)/2u;
        let candidate=auxiliary_ids[mid];
        if candidate==id {return true;}
        if candidate<id {lo=mid+1u;} else {hi=mid;}
    }
    return false;
}

fn self_tolerance(pixel: vec2<i32>, id: u32, depth: f32) -> f32 {
    let offsets=array<vec2<i32>,4>(vec2(1,0),vec2(-1,0),vec2(0,1),vec2(0,-1));
    let upper=vec2<i32>(textureDimensions(depths))-vec2(1);
    var slope=vec2(0.);
    for(var i=0u;i<4u;i+=1u) {
        let neighbor=clamp(pixel+offsets[i],vec2(0),upper);
        if textureLoad(identities,neighbor,0).r==id {
            slope[i/2u]=max(slope[i/2u],abs(textureLoad(depths,neighbor,0).r-depth));
        }
    }
    return 0.75*(slope.x+slope.y);
}

struct Input {
    @location(0) a: vec4<f32>,
    @location(1) b: vec4<f32>,
    @location(2) color: vec4<f32>,
    @location(3) hidden_color: vec4<f32>,
    @location(4) shape: vec4<f32>,
    @location(5) params: vec4<f32>,
    @location(6) ids: vec4<u32>,
}
struct Vertex {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) a: vec4<f32>,
    @location(1) @interpolate(flat) b: vec4<f32>,
    @location(2) @interpolate(flat) color: vec4<f32>,
    @location(3) @interpolate(flat) hidden_color: vec4<f32>,
    @location(4) @interpolate(flat) shape: vec4<f32>,
    @location(5) @interpolate(flat) params: vec4<f32>,
    @location(6) @interpolate(flat) ids: vec4<u32>,
}
@vertex fn vertex_main(input: Input, @builtin(vertex_index) index: u32) -> Vertex {
    let corners = array<vec2<f32>, 6>(vec2(0.,0.),vec2(1.,0.),vec2(0.,1.),vec2(0.,1.),vec2(1.,0.),vec2(1.,1.));
    let aa = 1. / min(view.density.x, view.density.y);
    let delta = input.b.xy-input.a.xy;
    let segment_length = length(delta);
    var axis = vec2(1.,0.);
    if segment_length > 0. { axis = delta/segment_length; }
    let normal = vec2(-axis.y,axis.x);
    let padding = input.shape.x + aa;
    let corner = corners[index];
    let pixel = (input.a.xy + axis*mix(-padding,segment_length+padding,corner.x)
        + normal*mix(-padding,padding,corner.y))*view.density;
    var out: Vertex;
    out.position = vec4(pixel / view.size * vec2(2.,-2.) + vec2(-1.,1.),0.,1.);
    out.a = input.a; out.b = input.b; out.color = input.color;
    out.hidden_color = input.hidden_color; out.shape = input.shape;
    out.params = input.params; out.ids = input.ids;
    return out;
}
struct Surface { color: vec4<f32>, depth: f32, coverage: f32 }
fn surface(input: Vertex) -> Surface {
    let p = input.position.xy / view.density;
    let ab = input.b.xy - input.a.xy;
    let length_squared = dot(ab,ab);
    let t = clamp(dot(p-input.a.xy,ab) / max(length_squared, 1e-12),0.,1.);
    let distance = length(p - mix(input.a.xy,input.b.xy,t));
    let aa = max(fwidth(distance), 0.5 / max(view.density.x,view.density.y));
    var coverage = 1. - smoothstep(input.shape.x-aa*0.5, input.shape.x+aa*0.5, distance);
    let inverse_w = mix(input.a.w,input.b.w,t);
    let depth = mix(input.a.z*input.a.w,input.b.z*input.b.w,t) / inverse_w;
    var pixel = vec2<i32>(input.position.xy);
    var occluder = textureLoad(identities,pixel,0).r;
    // Self visibility follows the projected primitive, not the width-expanded quad.
    // Foreign surfaces continue to mask the actual fragment footprint.
    if occluder==0u || is_auxiliary(occluder) {
        let center=clamp(vec2<i32>(mix(input.a.xy,input.b.xy,t)*view.density),vec2(0),vec2<i32>(textureDimensions(depths))-vec2(1));
        let center_id=textureLoad(identities,center,0).r;
        if center_id==0u || is_auxiliary(center_id) {
            pixel=center;
            occluder=center_id;
        }
    }
    let surface_depth=textureLoad(depths,pixel,0).r;
    var tolerance=input.params.x;
    if is_auxiliary(occluder) {tolerance+=self_tolerance(pixel,occluder,surface_depth);}
    let hidden = occluder != 0u && depth > surface_depth + tolerance;
    var color = input.color;
    if hidden {
        color = input.hidden_color;
        if input.shape.y == 0. { coverage = 0.; }
        if input.shape.y == 2. && length_squared > 1e-12 {
            let along = t*sqrt(length_squared) + input.params.y;
            let period = input.shape.z+input.shape.w;
            if period < aa {
                coverage *= input.shape.z / period;
            } else {
                let phase = along - floor(along/period)*period;
                let dash_distance = abs(phase-input.shape.z*0.5)-input.shape.z*0.5;
                coverage *= 1.-smoothstep(-aa*0.5,aa*0.5,dash_distance);
            }
        }
    }
    let alpha = color.a * coverage;
    return Surface(vec4(color.rgb*alpha,alpha),depth,coverage);
}
@fragment fn color_main(input: Vertex) -> @location(0) vec4<f32> {
    return surface(input).color;
}
struct Data { @location(0) id: u32, @location(1) depth: f32 }
@fragment fn data_main(input: Vertex) -> Data {
    let value = surface(input);
    if value.coverage < 0.5 || value.color.a <= 0. { discard; }
    return Data(input.ids.x,value.depth);
}
