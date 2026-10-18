/// Metal source, compiled at runtime (`MTLDevice.makeLibrary(source:)`) because the build only
/// needs the Command Line Tools, which ship no offline `metal` compiler.
///
/// The scene is a quiet tonal picture in grey with depth fog. Three passes:
///   1. `shadowVert`: depth from the sun, for cast shadows.
///   2. `gbufVert` / `gbufFrag`: the meshes (Blender models, terrain) → tone, material, the
///      screen direction the engraver's lines should run, distance, world position, id.
///   3. `engrave` (the name is historical): tone → grey, fog, a soft contour, sound made
///      visible as faint rings, the selection, and the violet listener.
/// The spectrogram and the track scopes are smooth grey on the panel.
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
        float4 counts;       // river points, birds, water flow 0..1, generic sources
        float4 bracket;      // selected box in px (x0, y0, x1, y1); x1 < x0 = none
        float4 hoverBracket;
        float4 plumb;        // bird and ground point in px, for a selected bird
        float4 sun;          // direction to the sun xyz, plumb on
        float4 env;          // daylight 0..1, wetness 0..1, lightning flash 0..1, -
        float4 gaze;         // listener point screen x, y px, px per metre, live 1 / resting 0
        float4 gazeHead;     // head forward direction xyz (world), visible
        float4 haze;         // horizon y px (where the level ground meets the sky), -, -, -
        float4 listener;     // head position xyz, its distance from the eye
        float4x4 invViewProj;
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
    constant int D_GEN = 32;    // 8: generic sources x, y, z, level
    constant int D_GENINFO = 40;// 8: kind (ASMR_SOURCE_*), screen x px, y px, px per metre

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
        // Beyond the shadow map (the far land) everything is in the open.
        if (any(suv < 0.0) || any(suv > 1.0)) lit = 1.0;

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

    // ---- shading -------------------------------------------------------------------------------

    // Coverage of a line `w` pixels wide at distance f from its centre, anti-aliased.
    inline float cut(float f, float w) { return clamp(w * 0.5 - f + 0.5, 0.0, 1.0); }

    // The panel's background grey (Ink.panel), so the strips sit flush in it.
    constant float PANEL = 0.078;

    // ASMR_DEBUG=1 shows tone tinted by material instead of the shading.
    constant bool DEBUG_VIEW = false;

    inline int contourClass(int mat) {
        if (mat == M_VOID) return 0;
        if (mat == M_GROUND || mat == M_SECTION) return 1;
        if (mat == M_WATER) return 2;
        return 3;
    }

    // Daylight values per material: the darkest (in shade) and the lightest (in full sun).
    // Near-black trees on a dark field, the river a little lighter than the land, the herm grey stone.
    constant float2 RANGE[8] = {
        float2(0.0), float2(0.06, 0.19), float2(0.17, 0.32), float2(0.022, 0.15),
        float2(0.018, 0.11), float2(0.04, 0.16), float2(0.20, 0.56), float2(0.08, 0.16)
    };

    // The scene as a quiet tonal picture in grey: smooth light and shade, soft depth fog into
    // a dusk-grey haze by day and near black by night, a sparse contour where things overlap, and the
    // sounds drawn as faint rings. The violet listener is the only colour.
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
        if (DEBUG_VIEW) {
            float3 pal[8] = { float3(0), float3(0.6, 0.45, 0.25), float3(0.25, 0.45, 1), float3(0.2, 0.85, 0.3),
                              float3(0.7, 0.35, 0.15), float3(1, 0.25, 0.25), float3(1, 1, 0.6), float3(0.5) };
            return float4(pal[mat] * (0.25 + 0.75 * min(tone, 1.0)) + (sel ? 0.25 : 0.0), 1);
        }

        float day = F.env.x;
        // A dark, misty picture: by day a dusk-grey haze, by night nearly black.
        float dim = mix(0.55, 1.0, day);
        float fogC = mix(0.05, 0.23, day);
        float skyTop = mix(0.025, 0.10, day);
        // Ink for drawn marks (rings, outlines): pale on the dark picture.
        float inkC = 0.86;

        float v;
        float fog;
        if (mat == M_VOID) {
            float above = clamp((F.haze.x - px.y) / (F.view.y * 0.75), 0.0, 1.0);
            v = mix(fogC, skyTop, pow(above, 0.8));
            fog = 1.0;
        } else {
            float2 r = RANGE[mat];
            float t = clamp(tone, 0.0, 1.0);
            if (sel) t = min(t + 0.18, 1.0);
            v = mix(r.x, r.y, smoothstep(0.0, 1.0, t)) * dim;
            // Sun glints on the water.
            if (mat == M_WATER) v += max(tone - 1.0, 0.0) * 0.35 * dim;
            if (mat == M_GROUND) v *= 1.0 - 0.25 * F.env.y;      // wet ground darkens
            // Distance fog: clear around the herm, the far land dissolves into the sky.
            fog = 1.0 - exp(-max(dist - 6.0, 0.0) / mix(24.0, 28.0, day));
            fog = max(fog, smoothstep(60.0, 150.0, dist));
            v = mix(v, fogC, fog);
        }

        // Contours: a soft line where the next pixel is farther away or another kind of thing.
        int cls = contourClass(mat);
        float edge = 0.0;
        if (cls > 1) {
            int2 dirs[4] = { int2(1, 0), int2(-1, 0), int2(0, 1), int2(0, -1) };
            for (int k = 0; k < 4; k++) {
                int2 q = clamp(int2(ip) + dirs[k] * int(max(s * 0.5, 1.0)), int2(0), int2(size) - 1);
                float4 n = ga.read(uint2(q));
                int nm = int(n.y + 0.5) & 31;
                int nc = contourClass(nm);
                float jump = (mat == M_FOLIAGE && nm == M_FOLIAGE) ? 1.12 : 1.03;
                bool farther = n.w > a.w * jump + 0.0005;
                if ((cls == 3 && (farther || nc < 3)) || (cls == 2 && nc == 1)) edge = 1.0;
            }
        }
        // Contours are dark and fade with the fog.
        float lineC = 0.0;
        v = mix(v, lineC, edge * 0.35 * (1.0 - fog));

        // Marks drawn over the picture, in ink: g is their coverage.
        float g = 0.0;
        // An outline around the selection (and, fainter, the hovered thing), two points out.
        if (!sel) {
            float haloSel = 0.0, haloHov = 0.0;
            for (int k = 0; k < 8; k++) {
                float an = float(k) * 0.785398;
                int2 q = clamp(int2(px + float2(cos(an), sin(an)) * 1.6 * s), int2(0), int2(size) - 1);
                int nc = int(ga.read(uint2(q)).y + 0.5);
                if ((nc & 32) != 0) haloSel = 1.0;
                if ((nc & 64) != 0) haloHov = 1.0;
            }
            g = max(g, haloSel * 0.85);
            g = max(g, haloHov * 0.35);
        }

        // The river: ripples spreading on the water where it sounds.
        if (mat == M_WATER) {
            for (int i = 0; i < int(F.counts.x); i++) {
                float4 R = D[D_RIVER + i];
                float lvl = clamp(R.w * 3.0, 0.0, 1.0);
                if (lvl < 0.03) continue;
                float d = length(b.xy - R.xy);
                for (int k = 0; k < 2; k++) {
                    float ph = fract(time * 0.25 + float(k) / 2.0 + float(i) * 0.37);
                    float rad = 0.2 + ph * 2.0;
                    float on = cut(abs(d - rad) / pxWorld, 1.0);
                    g = max(g, on * lvl * (1.0 - ph) * 0.35);
                }
            }
        }
        // Birds: thin rings around the singer, while it sings.
        for (int i = 0; i < int(F.counts.y); i++) {
            float4 B = D[D_BIRD + i];
            float lvl = clamp(B.w * 9.0, 0.0, 1.0);
            if (lvl < 0.03) continue;
            float bd = D[D_BIRDZ + i].x;
            if (mat != M_VOID && dist < bd - 1.0) continue;
            float d = length(px - B.xy);
            for (int k = 0; k < 2; k++) {
                float ph = fract(time * 0.7 + float(k) / 2.0 + float(i) * 0.5);
                float rad = (0.7 + ph * 2.4) * B.z;
                float on = cut(abs(d - rad), 0.9 * s);
                g = max(g, on * lvl * (1.0 - ph) * 0.5);
            }
        }
        // The listening circle on the ground around the herm, with a tick where the head is turned.
        if (mat == M_GROUND) {
            float r = length(b.xy);
            float an = atan2(b.y, b.x) - F.head.x;
            an = atan2(sin(an), cos(an));
            float ring = cut(abs(r - 2.3) / pxWorld, 0.9) * step(fract(an * 12.0 / 3.14159), 0.5);
            float tick = cut(abs(an) * r / pxWorld, 0.9) * step(2.3, r) * step(r, 3.3);
            g = max(g, max(ring, tick) * 0.35);
        }
        // A dotted plumb line from a selected bird down to the ground.
        if (F.sun.w > 0.5) {
            float2 A = F.plumb.xy, Bp = F.plumb.zw;
            float2 ab = Bp - A;
            float h = clamp(dot(px - A, ab) / max(dot(ab, ab), 1.0), 0.0, 1.0);
            float d = length(px - (A + ab * h));
            float dots = step(fract(h * length(ab) / (4.0 * s)), 0.5);
            g = max(g, cut(d, 1.0) * dots * 0.7 * step(0.04, h));
        }
        // Corner marks around the selection and the hovered thing.
        float4 bx = F.bracket;
        for (int k = 0; k < 2; k++) {
            if (bx.z > bx.x) {
                float L = 7.0 * s;
                float2 d0 = px - bx.xy, d1 = bx.zw - px;
                bool inside = d0.x >= 0 && d0.y >= 0 && d1.x >= 0 && d1.y >= 0;
                bool nearX = min(d0.x, d1.x) < L, nearY = min(d0.y, d1.y) < L;
                bool onX = min(d0.x, d1.x) < s, onY = min(d0.y, d1.y) < s;
                if (inside && ((onX && nearY) || (onY && nearX))) g = max(g, k == 0 ? 0.9 : 0.35);
            }
            bx = F.hoverBracket;
        }

        // Generic sources: rain curtains with splashes, a campfire's flames, crickets. All grey.
        float light = 0.0;
        for (int i = 0; i < int(F.counts.w); i++) {
            float4 G = D[D_GEN + i];
            float4 I = D[D_GENINFO + i];
            int kind = int(I.x + 0.5);
            float2 sp = I.yz;
            float ppm = I.w;
            if (kind == 3) {
                if (G.w < 0.002) continue;
                float lvl = clamp(0.35 + G.w * 25.0, 0.0, 1.0);
                float R = 2.6 * ppm;
                float dx = abs(px.x - sp.x);
                float ex = smoothstep(R, R * 0.55, dx);
                float ey = smoothstep(sp.y - 6.0 * ppm, sp.y - 4.5 * ppm, px.y) * smoothstep(sp.y + 0.8 * ppm, sp.y, px.y);
                float k = (px.x + 0.22 * px.y) / (6.0 * s);
                float ln = floor(k);
                float fo = abs(fract(k) - 0.5) * 6.0 * s;
                float ph = fract(px.y / ((22.0 + 14.0 * hash11(ln)) * s) - time * (1.4 + 0.5 * hash11(ln * 3.1)) + hash11(ln * 7.3));
                g = max(g, cut(fo, 0.8 * s) * step(ph, 0.14) * lvl * ex * ey * 0.35);
                if ((mat == M_GROUND || mat == M_WATER) && length(b.xy - G.xy) < 2.6) {
                    float2 cell = floor(b.xy * 2.5);
                    float sph = fract(time * 1.3 + hash21(cell));
                    float2 c = (cell + 0.2 + 0.6 * float2(hash21(cell + 1.7), hash21(cell + 4.1))) / 2.5;
                    float rr = 0.03 + sph * 0.16;
                    g = max(g, cut(abs(length(b.xy - c) - rr) / pxWorld, 1.0) * (1.0 - sph) * lvl * 0.4);
                }
            } else if (kind == 5) {
                float lvl = clamp(0.35 + G.w * 8.0, 0.0, 1.0);
                float flick = 0.8 + 0.2 * sin(time * 9.0) * sin(time * 6.3 + 1.0);
                // A soft column of light where the flames are.
                float2 q = (px - sp) / ppm;
                float flame = exp(-q.x * q.x / 0.08) * smoothstep(0.2, -0.2, q.y) * smoothstep(-1.6 * flick, -0.2, q.y);
                light = max(light, flame * lvl);
                if (mat != M_VOID) {
                    float d = length(b.xy - G.xy);
                    light = max(light, exp(-d * d / 6.0) * flick * lvl * mix(0.5, 0.2, day));
                }
            } else if (kind == 6) {
                float lvl = clamp(G.w * 25.0, 0.0, 1.0);
                if (lvl < 0.03 || mat != M_GROUND) continue;
                float d = length(b.xy - G.xy);
                float ph = fract(time * 2.0 + float(i) * 0.31);
                float rr = 0.08 + ph * 0.6;
                g = max(g, cut(abs(d - rr) / pxWorld, 0.9) * (1.0 - ph) * lvl * 0.3);
            }
        }

        v = mix(v, inkC, g * (1.0 - 0.6 * fog));
        v = v + (1.0 - v) * light;
        // A lightning strike lights the whole picture for a moment.
        v += F.env.z * 0.35 * (1.0 - v);
        // A gentle vignette keeps the eye in the middle.
        float2 cv = in.uv * 2.0 - 1.0;
        v *= 1.0 - 0.22 * dot(cv * float2(0.8, 1.0), cv * float2(0.8, 1.0)) * 0.5;
        // An invisible dither so the fog does not band.
        v += (hash21(px) - 0.5) / 255.0;
        float3 c = float3(clamp(v, 0.0, 1.0));

        // The listener: the one coloured thing. A violet point at the head, and a soft beam of
        // light from it towards where the head looks, like a faint lamp in the mist.
        if (F.gazeHead.w > 0.5) {
            float live = F.gaze.w;
            float strength = mix(0.5, 1.0, live);
            float3 violet = float3(0.53, 0.33, 1.0);
            float3 head = F.listener.xyz;
            float3 dirB = normalize(F.gazeHead.xyz);
            float2 ndc = float2(in.uv.x * 2.0 - 1.0, 1.0 - in.uv.y * 2.0);
            float4 r0 = F.invViewProj * float4(ndc, 0.0, 1.0);
            float4 r1 = F.invViewProj * float4(ndc, 1.0, 1.0);
            float3 ro = F.cam.xyz;
            float3 rd = normalize(r1.xyz / r1.w - r0.xyz / r0.w);
            float tc = max(dot(head - ro, rd), 0.0);
            float tEnd = mat == M_VOID ? tc + 30.0 : dist;
            float t0 = max(tc - 14.0, 0.0), t1 = min(tc + 30.0, tEnd);
            float beam = 0.0;
            float spread = 0.30;
            const int N = 24;
            float dt = max(t1 - t0, 0.0) / float(N);
            for (int k = 0; k < N; k++) {
                float3 q = ro + rd * (t0 + (float(k) + 0.5) * dt);
                float3 w = q - head;
                float along = dot(w, dirB);
                if (along <= 0.0) continue;
                float radial = length(w - along * dirB);
                float wd = along * spread + 0.12;
                float fall = exp(-along / 10.0) * smoothstep(0.0, 0.6, along);
                beam += exp(-(radial * radial) / (wd * wd)) * fall / (wd * 1.6) * dt;
                beam += exp(-(radial * radial) / (0.0225 + 0.0025 * along * along)) * fall * 0.8 * dt;
            }
            beam = 1.0 - exp(-beam * 0.9);

            float2 hp = F.gaze.xy;
            float ppm = F.gaze.z;
            float d = length(px - hp);
            float core = max(0.12 * ppm, 2.8 * s);
            float hidden = (mat != M_VOID && dist < F.listener.w - 0.3) ? 0.3 : 1.0;
            float halo = (exp(-pow(d / (core * 4.0), 2.0)) * 0.7 + exp(-d / (core * 9.0)) * 0.25) * hidden;
            float disc = smoothstep(core + s, core - s, d) * hidden;
            // Mixed in rather than added, so the violet stays violet over the fog.
            float aBeam = clamp(beam * 0.8 * strength, 0.0, 0.75);
            c = mix(c, mix(violet, float3(0.78, 0.68, 1.0), 0.35), aBeam);
            c = mix(c, violet, clamp(halo * strength, 0.0, 0.9));
            c = mix(c, float3(0.93, 0.88, 1.0), disc * strength);
        }
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

    // A smooth grey field on the panel: brighter where there is energy, older columns fainter.
    fragment float4 spectrogram(VOut in [[stage_in]], constant SpecU& u [[buffer(0)]],
                                texture2d<float> hist [[texture(0)]]) {
        constexpr sampler smp(filter::linear, address::repeat);
        float2 px = in.pos.xy;
        float s = u.size.z;
        float W = u.ring.y, B = u.ring.z, vis = u.ring.w;
        float colF = (u.size.x - px.x) / u.size.x * vis;
        float col = u.ring.x - colF;
        float frac = mix(u.zoom.x, u.zoom.y, 1.0 - px.y / u.size.y);
        float v = hist.sample(smp, float2((col + 0.5) / W, (frac * (B - 1.0) + 0.5) / B)).r;
        float tone = pow(clamp((v - 0.22) / 0.6, 0.0, 1.0), 1.5);
        float age = px.x / u.size.x;
        float g = mix(PANEL, 0.86, tone * mix(0.6, 1.0, age));

        // Decade rules, faint and dotted: 100 Hz, 1 kHz, 10 kHz.
        for (int k = 0; k < 3; k++) {
            float y = yForFreq(100.0 * pow(10.0, float(k)), u);
            g = max(g, cut(abs(px.y - y), 1.0) * step(fmod(px.x, 6.0 * s), s) * 0.28);
        }
        for (int k = 0; k < 4; k++) {
            float fr = u.birds[k];
            if (fr <= 0.0) continue;
            float y = yForFreq(fr, u);
            g = max(g, cut(abs(px.y - y), 1.2 * s) * step(u.size.x - 6.0 * s, px.x) * 0.8);
        }
        if (u.sel.x > 0.0) {
            float y = yForFreq(u.sel.x, u);
            g = max(g, cut(abs(px.y - y), s) * step(fmod(px.x, 3.0 * s), 1.5 * s) * 0.6);
        }
        if (u.hover.z > 0.5) {
            float vx = cut(abs(px.x - u.hover.x), s) * step(fmod(px.y, 3.0 * s), 1.5 * s);
            g = max(g, vx * 0.5);
        }
        return float4(g, g, g, 1.0);
    }

    // ---- track scopes --------------------------------------------------------------------------

    struct TrackU {
        float4 size;      // w px, h px, px per point, row count
        float4 rows[16];  // per row: top px, height px, scope row (-1 none), slider fraction
    };

    // Each lever's slider. The rail is a thin line; up to the value the sound's live waveform
    // stands on it as a soft band (a plain bar for groups without a sound of their own).
    fragment float4 tracks(VOut in [[stage_in]], constant TrackU& u [[buffer(0)]],
                           texture2d<float> scope [[texture(0)]]) {
        constexpr sampler smp(filter::linear, address::clamp_to_edge);
        float2 px = in.pos.xy;
        float s = u.size.z;
        float4 r = float4(0, 0, -2, 0);
        for (int i = 0; i < int(u.size.w); i++) {
            if (px.y >= u.rows[i].x && px.y < u.rows[i].x + u.rows[i].y) r = u.rows[i];
        }
        if (r.z < -1.5) return float4(PANEL, PANEL, PANEL, 1);
        float mid = r.x + r.y * 0.5;
        float cutX = r.w * u.size.x;
        bool before = px.x <= cutX;

        float rail = cut(abs(px.y - mid), s) * (before ? 0.0 : 0.22);
        float g = PANEL;
        g = mix(g, 0.9, rail);
        float body;
        if (r.z >= 0.0) {
            float amp = scope.sample(smp, float2(px.x / u.size.x, (r.z + 0.5) / float(scope.get_height()))).r;
            float half_ = max(amp * r.y * 0.22, 1.0 * s);
            body = smoothstep(half_ + 0.6 * s, half_ - 0.6 * s, abs(px.y - mid));
        } else {
            body = cut(abs(px.y - mid), 2.0 * s);
        }
        g = mix(g, 0.62, body * (before ? 1.0 : 0.0));
        return float4(g, g, g, 1.0);
    }
    """
}
