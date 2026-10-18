/// Metal source, compiled at runtime (`MTLDevice.makeLibrary(source:)`) because the build only
/// needs the Command Line Tools, which ship no offline `metal` compiler.
///
/// The scene is a white-line engraving, after Bewick: light is cut into a black block as fine
/// lines. Three passes:
///   1. `shadowVert`: depth from the sun, for cast shadows.
///   2. `gbufVert` / `gbufFrag`: the meshes (Blender models, terrain) → tone, material, the
///      screen direction the engraver's lines should run, distance, world position, id.
///   3. `engrave`: every pixel cuts lines whose width follows the tone, plus contours,
///      sound made visible and the selection.
/// The spectrogram and the track scopes are engraved the same way.
enum Shaders {
    static let source = """
    #include <metal_stdlib>
    using namespace metal;

    struct VOut { float4 pos [[position]]; float2 uv; };

    vertex VOut fullscreen(uint vid [[vertex_id]]) {
        float2 p = float2((vid << 1) & 2, vid & 2);
        VOut o;
        o.pos = float4(p * 2.0 - 1.0, 0.0, 1.0);
        o.uv = float2(p.x, 1.0 - p.y);
        return o;
    }

    float hash11(float x) { return fract(sin(x * 127.1 + 311.7) * 43758.5453); }
    float hash21(float2 p) {
        p = fract(p * float2(123.34, 456.21));
        p += dot(p, p + 45.32);
        return fract(p.x * p.y);
    }

    // ---- scene ---------------------------------------------------------------------------------

    struct Vtx { packed_float3 p; packed_float3 n; float part; float pad; };

    struct FrameU {
        float4x4 viewProj;
        float4x4 sunViewProj;
        float4 cam;          // eye xyz, time
        float4 view;         // width px, height px, px per point, radians per px
        float4 head;         // yaw, -, -, playing
        float4 counts;       // river points, birds, water flow 0..1, -
        float4 bracket;      // selected box in px (x0, y0, x1, y1); x1 < x0 = none
        float4 hoverBracket;
        float4 plumb;        // bird and ground point in px, for a selected bird
        float4 sun;          // direction to the sun xyz, plumb on
    };

    struct Inst {
        float4x4 model;
        float4 info;         // id (-1 none, 100 river), flags (1 selected, 2 hovered), lean m, height m
        float4 anim;         // wing angle, phase, local wind, kind (0 thing, 1 tree)
    };

    // Data buffer layout (mirrors SceneRenderer).
    constant int D_RIVER = 0;   // 8: x, y, z, level
    constant int D_BIRD = 8;    // 4: screen x px, y px, px per metre, level
    constant int D_BIRDZ = 12;  // 4: distance from the eye, -, -, -
    constant int D_WIND = 16;   // 16 x 4 samples, +y (left) .. -y (right)

    constant int M_VOID = 0, M_GROUND = 1, M_WATER = 2, M_FOLIAGE = 3, M_WOOD = 4, M_BIRD = 5,
                 M_STONE = 6, M_SECTION = 7;

    float windAt(float y, constant float4* D) {
        float x = clamp((20.0 - y) / 40.0, 0.0, 1.0) * 63.0;
        int i0 = int(x);
        int i1 = min(i0 + 1, 63);
        return mix(D[D_WIND + i0 / 4][i0 % 4], D[D_WIND + i1 / 4][i1 % 4], x - float(i0));
    }

    struct GOut {
        float4 pos [[position]];
        float3 world;
        float3 normal;
        float part;
    };

    float3 deform(float3 p, float3 n, float part, constant Inst& I, float time, thread float3& nOut) {
        float3 q = p;
        nOut = n;
        // Wings beat about the body's long axis.
        if (part > 2.5 && part < 4.5) {
            float a = (part < 3.5 ? 1.0 : -1.0) * I.anim.x;
            float3 r = q - float3(0, 0, 0.03);
            float c = cos(a), s = sin(a);
            q = float3(r.x, c * r.y - s * r.z, s * r.y + c * r.z) + float3(0, 0, 0.03);
            nOut = float3(n.x, c * n.y - s * n.z, s * n.y + c * n.z);
        }
        return q;
    }

    float3 sway(float3 w, float3 local, float part, constant Inst& I, float time) {
        if (I.anim.w < 0.5) return w;
        // Trees bend with the gust front (towards -y), more at the top; leaves shiver.
        float hf = clamp(local.z / max(I.info.w, 1.0), 0.0, 1.3);
        float lean = I.info.z;
        w.y -= lean * hf * hf;
        w.x += lean * 0.18 * hf * sin(time * 2.1 + I.anim.y);
        if (part > 0.5 && part < 1.5) {
            float k = dot(local, float3(3.1, 2.3, 1.7));
            w += float3(0.02, 0.03, 0.015) * I.anim.z * sin(time * (7.0 + 5.0 * I.anim.z) + k);
        }
        return w;
    }

    vertex float4 shadowVert(uint vid [[vertex_id]], const device Vtx* V [[buffer(0)]],
                             constant Inst& I [[buffer(1)]], constant FrameU& F [[buffer(2)]]) {
        Vtx v = V[vid];
        float3 n;
        float3 p = deform(float3(v.p), float3(v.n), v.part, I, F.cam.w, n);
        float3 w = sway((I.model * float4(p, 1)).xyz, p, v.part, I, F.cam.w);
        return F.sunViewProj * float4(w, 1);
    }

    vertex GOut gbufVert(uint vid [[vertex_id]], const device Vtx* V [[buffer(0)]],
                         constant Inst& I [[buffer(1)]], constant FrameU& F [[buffer(2)]]) {
        Vtx v = V[vid];
        float3 n;
        float3 p = deform(float3(v.p), float3(v.n), v.part, I, F.cam.w, n);
        float3 w = sway((I.model * float4(p, 1)).xyz, p, v.part, I, F.cam.w);
        GOut o;
        o.pos = F.viewProj * float4(w, 1);
        o.world = w;
        o.normal = normalize((I.model * float4(n, 0)).xyz);
        o.part = v.part;
        return o;
    }

    struct GBuf { float4 a [[color(0)]]; float4 b [[color(1)]]; };

    fragment GBuf gbufFrag(GOut in [[stage_in]], bool front [[front_facing]],
                           constant Inst& I [[buffer(1)]], constant FrameU& F [[buffer(2)]],
                           depth2d<float> shadowMap [[texture(0)]]) {
        constexpr sampler cmp(coord::normalized, filter::linear, address::clamp_to_edge, compare_func::less_equal);
        float3 n = normalize(in.normal) * (front ? 1.0 : -1.0);
        float3 eye = F.cam.xyz;
        float3 V = normalize(eye - in.world);
        if (dot(n, V) < 0.0) n = -n;
        float part = in.part;
        int mat = part < 0.5 ? M_WOOD : part < 1.5 ? M_FOLIAGE : part < 4.5 ? M_BIRD : part < 5.5 ? M_STONE
                : part < 6.5 ? M_BIRD : part < 7.5 ? M_GROUND : part < 8.5 ? M_WATER : M_SECTION;
        float time = F.cam.w;
        float3 L = F.sun.xyz;

        if (mat == M_WATER) {
            // Ripples run downstream (-y), quicker with more flow.
            float ph = in.world.y * 3.1 + time * (1.2 + 3.0 * F.counts.z);
            n = normalize(float3(0.07 * sin(in.world.x * 5.0 + ph * 0.6), 0.12 * sin(ph) + 0.05 * sin(in.world.x * 11.0 - ph * 1.7), 1.0));
        }

        // Shadow map, 4 taps.
        float4 sp = F.sunViewProj * float4(in.world + n * 0.04, 1);
        float2 suv = float2(sp.x * 0.5 + 0.5, 0.5 - sp.y * 0.5);
        float lit = 0.0;
        float texel = 1.0 / 2048.0;
        for (int k = 0; k < 4; k++) {
            float2 o = float2(k & 1 ? 1.0 : -1.0, k & 2 ? 1.0 : -1.0) * texel * 1.5;
            lit += shadowMap.sample_compare(cmp, suv + o, sp.z - 0.0015);
        }
        lit *= 0.25;

        float diff = max(dot(n, L), 0.0) * lit;
        float sky = 0.5 + 0.5 * n.z;
        float tone;
        if (mat == M_WATER) {
            float spec = pow(max(dot(reflect(-V, n), L), 0.0), 40.0) * lit;
            tone = 0.42 + 0.2 * lit + 1.4 * spec;
        } else if (mat == M_STONE) {
            tone = 0.20 + 0.85 * diff + 0.1 * sky;        // marble keeps its light
        } else if (mat == M_BIRD) {
            tone = 0.45 + 0.6 * diff;
        } else if (mat == M_GROUND) {
            tone = 0.12 + 0.75 * diff;
        } else if (mat == M_SECTION) {
            tone = 0.18 + 0.35 * max(dot(n, L), 0.0);
        } else if (mat == M_FOLIAGE) {
            tone = 0.08 + 0.95 * diff + 0.08 * sky;
        } else {
            tone = 0.06 + 0.7 * diff;
        }
        float dist = length(eye - in.world);
        tone *= mix(1.0, 0.7, clamp((dist - 24.0) / 24.0, 0.0, 1.0));

        // The engraver's line runs along the surface: around trunks and crowns, along the
        // river, across the ground.
        float3 axis;
        if (mat == M_WATER) axis = float3(0, 1, 0);          // along the flow
        else if (mat == M_GROUND) axis = float3(1, 0.25, 0);  // across it, so the river stands out
        else if (mat == M_SECTION) axis = normalize(cross(n, float3(0, 0, 1)) + float3(0, 0, 1e-4));
        else {
            float3 t = cross(n, float3(0, 0, 1));
            axis = length(t) > 0.15 ? normalize(t) : float3(1, 0, 0);
        }
        float4 c0 = F.viewProj * float4(in.world, 1);
        float4 c1 = F.viewProj * float4(in.world + axis * 0.05, 1);
        float2 d = (c1.xy / c1.w - c0.xy / c0.w) * float2(1, -1);
        float angle = atan2(d.y * F.view.y, d.x * F.view.x);

        float flags = I.info.y;
        float id = I.info.x;
        if (mat == M_WATER) id = 100.0;
        if (mat == M_GROUND || mat == M_SECTION) { id = -1.0; flags = 0.0; }

        GBuf o;
        o.a = float4(clamp(tone, 0.0, 1.5), float(mat) + 32.0 * flags, angle, dist / 60.0);
        o.b = float4(in.world, id);
        return o;
    }

    // ---- engraving -----------------------------------------------------------------------------

    // Width of an engraved line at distance f from its centre: anti-aliased, `w` pixels wide.
    inline float cut(float f, float w) { return clamp(w * 0.5 - f + 0.5, 0.0, 1.0); }

    // Watercolour washes per material: void, ground, water, foliage, wood, bird, stone, cut face.
    constant float3 WASH[8] = {
        float3(0.0), float3(0.62, 0.48, 0.30), float3(0.36, 0.52, 0.66), float3(0.42, 0.58, 0.34),
        float3(0.56, 0.40, 0.26), float3(0.80, 0.46, 0.30), float3(0.86, 0.80, 0.70), float3(0.50, 0.38, 0.26)
    };

    // ASMR_DEBUG=1 shows tone tinted by material instead of the engraving.
    constant bool DEBUG_VIEW = false;

    inline int contourClass(int mat) {
        if (mat == M_VOID) return 0;
        if (mat == M_GROUND || mat == M_SECTION) return 1;
        if (mat == M_WATER) return 2;
        return 3;
    }

    fragment float4 engrave(VOut in [[stage_in]], constant FrameU& F [[buffer(0)]], constant float4* D [[buffer(1)]],
                            texture2d<float, access::read> ga [[texture(0)]],
                            texture2d<float, access::read> gb [[texture(1)]]) {
        float2 px = in.pos.xy;
        uint2 ip = uint2(px);
        uint2 size = uint2(ga.get_width(), ga.get_height());
        float4 a = ga.read(ip);
        float4 b = gb.read(ip);
        float s = F.view.z;
        float time = F.cam.w;
        float tone = a.x;
        int code = int(a.y + 0.5);
        int mat = code & 31;
        bool sel = (code & 32) != 0;
        bool hov = (code & 64) != 0;
        float dist = a.w * 60.0;
        float pxWorld = dist * F.view.w;      // metres per pixel at this depth
        float g = 0.0;
        if (DEBUG_VIEW) {
            float3 pal[8] = { float3(0), float3(0.6, 0.45, 0.25), float3(0.25, 0.45, 1), float3(0.2, 0.85, 0.3),
                              float3(0.7, 0.35, 0.15), float3(1, 0.25, 0.25), float3(1, 1, 0.6), float3(0.5) };
            return float4(pal[mat] * (0.25 + 0.75 * min(tone, 1.0)) + (sel ? 0.25 : 0.0), 1);
        }

        if (mat != M_VOID) {
            float ang = a.z;
            float2 dir = float2(cos(ang), sin(ang));
            float2 nrm = float2(-dir.y, dir.x);
            float along = dot(px, dir);
            float across = dot(px, nrm);
            float SP = (mat == M_GROUND ? 2.4 : mat == M_WOOD ? 1.5 : mat == M_WATER ? 1.6 : 1.8) * s;   // line pitch
            // A hand-cut line wanders a little.
            across += 0.32 * s * sin(along / (9.0 * s) + floor(across / SP) * 1.7);
            float k = across / SP;
            float line = floor(k);
            float f = abs(fract(k) - 0.5) * SP;
            float t = tone;
            if (sel) t = min(t * 1.25 + 0.15, 1.2);
            float w = clamp(t, 0.0, 1.0) * SP * 0.9;
            float cov = cut(f, w);
            float bright = 0.9;

            if (mat == M_FOLIAGE) {
                // Short flicks rather than long lines, moving with the gust.
                float wind = windAt(b.y, D);
                float P = (7.0 + 5.0 * hash11(line)) * s;
                float ph = fract(along / P + hash11(line * 3.7) + time * 0.15 * wind);
                cov *= step(ph, 0.3 + 0.6 * clamp(t, 0.0, 1.0));
                bright = 0.9;
            } else if (mat == M_GROUND) {
                // Dot-and-lick: lines break into points in the shade.
                float P = 3.0 * s;
                float ph = fract(along / P + hash11(line));
                cov = cut(f, w * 0.55) * step(ph, clamp(t * 1.7, 0.12, 1.0));
                // The ground fades into the plate towards the cut edge.
                bright = 0.62 * (1.0 - 0.6 * smoothstep(8.0, 13.0, length(b.xy)));
            } else if (mat == M_WATER) {
                float P = (26.0 + 22.0 * hash11(line)) * s;
                float ph = fract(along / P + hash11(line * 1.3) - time * (0.05 + 0.2 * F.counts.z));
                cov = cut(f, max(w * 0.6, 0.9)) * step(ph, 0.55 + 0.4 * clamp(t, 0.0, 1.0));
                if (t > 0.95) cov = max(cov, cut(f, SP * 0.7));   // sun glints
                bright = 0.95;
            } else if (mat == M_BIRD) {
                cov = cut(f, max(w, 1.0));
                bright = 1.0;
            } else if (mat == M_STONE) {
                bright = 1.0;
            } else if (mat == M_SECTION) {
                bright = 0.6;
            }
            g = cov * bright;

            // Cross-hatching where the light is strongest.
            if (t > 0.72 && mat != M_GROUND && mat != M_WATER) {
                float2 d2 = float2(cos(ang + 1.1), sin(ang + 1.1));
                float k2 = dot(px, float2(-d2.y, d2.x)) / SP;
                float f2 = abs(fract(k2) - 0.5) * SP;
                g = max(g, cut(f2, (t - 0.72) / 0.5 * SP * 0.7) * bright);
            }
        } else {
            // Air: rare long strokes drifting with the gust front.
            float row = floor(px.y / (13.0 * s));
            float ry = fmod(px.y, 13.0 * s);
            float yWorld = mix(16.0, -16.0, in.uv.x);
            float wind = windAt(yWorld, D);
            float P = (90.0 + 60.0 * hash11(row)) * s;
            float ph = fract(px.x / P + hash11(row * 7.0) - time * (0.04 + 0.05 * hash11(row * 3.0)));
            float strokeOn = step(0.55, hash11(row * 11.0)) * step(ph, 0.28) * smoothstep(0.0, 0.08, ph) * smoothstep(0.28, 0.2, ph);
            float wob = 1.2 * s * sin(px.x / (23.0 * s) + row);
            g = cut(abs(ry - 6.5 * s - wob), 0.9) * strokeOn * clamp(wind * 0.5, 0.0, 0.45);
        }

        // Contours: a hairline where the next pixel is farther away or another kind of thing.
        int cls = contourClass(mat);
        float edge = 0.0;
        float haloSel = 0.0, haloHov = 0.0;
        if (cls > 0) {
            int2 dirs[4] = { int2(1, 0), int2(-1, 0), int2(0, 1), int2(0, -1) };
            for (int k = 0; k < 4; k++) {
                int2 q = clamp(int2(ip) + dirs[k], int2(0), int2(size) - 1);
                float4 n = ga.read(uint2(q));
                int nc = contourClass(int(n.y + 0.5) & 31);
                int nm = int(n.y + 0.5) & 31;
                // Inside a crown only real gaps count, or every needle tier would be outlined.
                float jump = (mat == M_FOLIAGE && nm == M_FOLIAGE) ? 1.08 : 1.025;
                bool farther = n.w > a.w * jump + 0.0005;
                if ((cls == 3 && (farther || nc < 3)) || (cls == 2 && nc == 1)) edge = 1.0;
            }
            g = max(g, edge * (cls == 2 ? 0.8 : hov ? 1.0 : 0.82));
        }
        // A soft aura around the selection, two and four points out.
        if (!sel) {
            for (int k = 0; k < 8; k++) {
                float an = float(k) * 0.785398;
                for (int r = 1; r <= 2; r++) {
                    int2 q = clamp(int2(px + float2(cos(an), sin(an)) * float(r) * 2.0 * s), int2(0), int2(size) - 1);
                    int nc = int(ga.read(uint2(q)).y + 0.5);
                    if ((nc & 32) != 0) haloSel = max(haloSel, r == 1 ? 1.0 : 0.5);
                    if ((nc & 64) != 0) haloHov = max(haloHov, r == 1 ? 0.6 : 0.0);
                }
            }
            float grain = hash21(floor(px / s) + floor(time * 8.0));
            g = max(g, haloSel * (0.22 + 0.25 * step(0.6, grain)));
            g = max(g, haloHov * 0.12);
        }

        // Sound, made visible. The river: ripples spreading on the water.
        if (mat == M_WATER) {
            for (int i = 0; i < int(F.counts.x); i++) {
                float4 R = D[D_RIVER + i];
                float lvl = clamp(R.w * 3.0, 0.0, 1.0);
                if (lvl < 0.03) continue;
                float d = length(b.xy - R.xy);
                for (int k = 0; k < 3; k++) {
                    float ph = fract(time * 0.3 + float(k) / 3.0 + float(i) * 0.37);
                    float rad = 0.2 + ph * 2.0;
                    float on = cut(abs(d - rad) / pxWorld, 1.1);
                    g = max(g, on * lvl * (1.0 - ph) * 0.9);
                }
            }
            // Listening circle never reaches the water; nothing else to add here.
        }
        // Birds: spherical wavefronts, seen as rings around the singer.
        for (int i = 0; i < int(F.counts.y); i++) {
            float4 B = D[D_BIRD + i];
            float lvl = clamp(B.w * 9.0, 0.0, 1.0);
            if (lvl < 0.03) continue;
            float bd = D[D_BIRDZ + i].x;
            if (mat != M_VOID && dist < bd - 1.0) continue;
            float d = length(px - B.xy);
            for (int k = 0; k < 3; k++) {
                float ph = fract(time * 0.85 + float(k) / 3.0 + float(i) * 0.5);
                float rad = (0.7 + ph * 2.6) * B.z;
                float on = cut(abs(d - rad), 1.1);
                g = max(g, on * lvl * (1.0 - ph * ph) * 0.85);
            }
        }
        // The listening circle around the herm, with a tick where the head is turned.
        if (mat == M_GROUND) {
            float r = length(b.xy);
            float an = atan2(b.y, b.x) - F.head.x;
            an = atan2(sin(an), cos(an));
            float ring = cut(abs(r - 2.3) / pxWorld, 1.0) * step(fract(an * 12.0 / 3.14159), 0.5);
            float tick = cut(abs(an) * r / pxWorld, 1.0) * step(2.3, r) * step(r, 3.3);
            g = max(g, max(ring, tick) * 0.7);
        }
        // A dotted plumb line from a selected bird down to the ground.
        if (F.sun.w > 0.5) {
            float2 A = F.plumb.xy, Bp = F.plumb.zw;
            float2 ab = Bp - A;
            float h = clamp(dot(px - A, ab) / max(dot(ab, ab), 1.0), 0.0, 1.0);
            float d = length(px - (A + ab * h));
            float dots = step(fract(h * length(ab) / (4.0 * s)), 0.5);
            g = max(g, cut(d, 1.0) * dots * 0.9 * step(0.04, h));
        }
        // Fine corner marks around the selection and the hovered thing.
        float4 bx = F.bracket;
        for (int k = 0; k < 2; k++) {
            if (bx.z > bx.x) {
                float L = 7.0 * s;
                float2 d0 = px - bx.xy, d1 = bx.zw - px;
                bool inside = d0.x >= 0 && d0.y >= 0 && d1.x >= 0 && d1.y >= 0;
                bool nearX = min(d0.x, d1.x) < L, nearY = min(d0.y, d1.y) < L;
                bool onX = min(d0.x, d1.x) < 1.0, onY = min(d0.y, d1.y) < 1.0;
                if (inside && ((onX && nearY) || (onY && nearX))) g = max(g, k == 0 ? 0.95 : 0.4);
            }
            bx = F.hoverBracket;
        }

        // The plate itself is not quite black.
        g = max(g, 0.025 * hash21(px));

        // Hand colouring: thin watercolour washes laid under the lines, a little blotchy and
        // a little off register, as on an old tinted print.
        float2 off = float2(1.6, -1.1) * s;
        uint2 wp = uint2(clamp(px + off, float2(0), float2(size) - 1.0));
        float4 wa = ga.read(wp);
        int wm = int(wa.y + 0.5) & 31;
        float3 wash = float3(0);
        if (wm != M_VOID) {
            float pigment = 0.75 + 0.5 * hash21(floor((px + off) / (23.0 * s)) + float2(wm, 3.0))
                          * (0.6 + 0.4 * sin(px.x / (37.0 * s) + px.y / (29.0 * s)));
            float strength = wm == M_GROUND ? 0.75 : wm == M_FOLIAGE || wm == M_WATER ? 1.2 : 1.0;
            wash = WASH[wm] * (0.07 + 0.2 * min(wa.x, 1.0)) * pigment * strength;
        }
        // Lines take a faint warmth of their wash; the selection stays pure white.
        float3 ink = sel ? float3(1.0) : mix(float3(1.0, 0.985, 0.95), WASH[mat] * 1.6, 0.12);
        float3 c = wash * (1.0 - g) + ink * g;
        return float4(c, 1.0);
    }

    // ---- spectrogram ---------------------------------------------------------------------------

    struct SpecU {
        float4 size;   // w px, h px, px per point, time
        float4 ring;   // newest column, columns, bands, visible columns
        float4 zoom;   // lo, hi (fractions of the band range), f lo Hz, f hi Hz
        float4 hover;  // x px, y px, active, frozen
        float4 birds;  // up to 4 bird pitches, Hz
        float4 sel;    // selected bird pitch Hz (0 = none)
    };

    float yForFreq(float f, constant SpecU& u) {
        float frac = log(f / u.zoom.z) / log(u.zoom.w / u.zoom.z);
        return (1.0 - (frac - u.zoom.x) / (u.zoom.y - u.zoom.x)) * u.size.y;
    }

    // Engraved like the scene: one fine line per row, swelling where there is energy.
    fragment float4 spectrogram(VOut in [[stage_in]], constant SpecU& u [[buffer(0)]],
                                texture2d<float> hist [[texture(0)]]) {
        constexpr sampler smp(filter::linear, address::repeat);
        float2 px = in.pos.xy;
        float s = u.size.z;
        float W = u.ring.y, B = u.ring.z, vis = u.ring.w;
        float SP = 2.0 * s;
        float row = floor(px.y / SP);
        float yc = (row + 0.5) * SP;
        float f = abs(px.y - yc);
        float colF = (u.size.x - px.x) / u.size.x * vis;
        float col = u.ring.x - colF;
        float frac = mix(u.zoom.x, u.zoom.y, 1.0 - yc / u.size.y);
        float v = hist.sample(smp, float2((col + 0.5) / W, (frac * (B - 1.0) + 0.5) / B)).r;
        float tone = pow(clamp((v - 0.2) / 0.62, 0.0, 1.0), 1.6);
        float age = px.x / u.size.x;
        float g = cut(f, tone * SP * 1.0) * mix(0.55, 1.0, age);

        // Decade rules, dotted: 100 Hz, 1 kHz, 10 kHz.
        for (int k = 0; k < 3; k++) {
            float y = yForFreq(100.0 * pow(10.0, float(k)), u);
            g = max(g, cut(abs(px.y - y), 1.0) * step(fmod(px.x, 5.0 * s), s) * 0.4);
        }
        // Bird pitches as ticks at the newest edge; the selected bird's pitch across.
        for (int k = 0; k < 4; k++) {
            float fr = u.birds[k];
            if (fr <= 0.0) continue;
            float y = yForFreq(fr, u);
            g = max(g, cut(abs(px.y - y), 1.4) * step(u.size.x - 8.0 * s, px.x));
        }
        if (u.sel.x > 0.0) {
            float y = yForFreq(u.sel.x, u);
            g = max(g, cut(abs(px.y - y), 1.0) * step(fmod(px.x, 3.0 * s), 1.5 * s) * 0.7);
        }
        if (u.hover.z > 0.5) {
            float vx = cut(abs(px.x - u.hover.x), 1.0) * step(fmod(px.y, 3.0 * s), 1.5 * s);
            float hy = cut(abs(px.y - u.hover.y), 1.0) * step(fmod(px.x, 3.0 * s), 1.5 * s);
            g = max(g, max(vx, hy) * 0.6);
        }
        return float4(g, g, g, 1.0);
    }

    // ---- track scopes --------------------------------------------------------------------------

    struct TrackU {
        float4 size;      // w px, h px, px per point, row count
        float4 rows[8];   // per macro row: top px, height px, scope row (-1 none), slider fraction
    };

    // Each macro row shows its sound's live waveform as engraved vertical strokes. Up to the
    // slider value the image is developed; beyond it only a faint latent trace remains.
    fragment float4 tracks(VOut in [[stage_in]], constant TrackU& u [[buffer(0)]],
                           texture2d<float> scope [[texture(0)]]) {
        constexpr sampler smp(filter::linear, address::clamp_to_edge);
        float2 px = in.pos.xy;
        float s = u.size.z;
        float4 r = float4(0, 0, -2, 0);
        for (int i = 0; i < int(u.size.w); i++) {
            if (px.y >= u.rows[i].x && px.y < u.rows[i].x + u.rows[i].y) r = u.rows[i];
        }
        if (r.z < -1.5) return float4(0, 0, 0, 1);
        float mid = r.x + r.y * 0.5;
        float cutX = r.w * u.size.x;

        float P = 2.0 * s;
        float colX = (floor(px.x / P) + 0.5) * P;
        float amp = r.z >= 0.0 ? scope.sample(smp, float2(colX / u.size.x, (r.z + 0.5) / 5.0)).r : 0.0;
        float half_ = max(amp * r.y * 0.4, 0.5 * s);
        float stroke = cut(abs(px.x - colX), 0.95 * s) * step(abs(px.y - mid), half_);

        float developed = smoothstep(cutX + 10.0 * s, cutX - 10.0 * s, px.x);
        float latent = 0.13 + 0.05 * hash21(floor(px / s));
        float g = stroke * mix(latent, 0.92, developed);
        g = max(g, cut(abs(px.y - mid), 1.0) * (developed > 0.5 ? 0.0 : 0.16));
        return float4(g, g, g, 1.0);
    }
    """
}
