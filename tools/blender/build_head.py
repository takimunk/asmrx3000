"""Bakes the onboarding girl's 3D head (a fal.ai image-to-3D generation of
apps/macos/Resources/Onboarding/front.png) into the compact asset the app draws:

    /Applications/Blender.app/Contents/MacOS/Blender -b --factory-startup \
        --python tools/blender/build_head.py -- [model.glb]

The source GLB defaults to tools/fal/dl/head3d/hunyuan31_front.glb (Hunyuan 3D v3.1 Pro, front
view only; 500k triangles and a 4096² texture, not committed). The script

  * lines the mesh up with front.png: an orthographic view from the front of the model space
    (x right, y up, z towards the viewer) shows exactly the portrait's square, x and y in
    −0.5 … 0.5 (u = x + 0.5, v = 0.5 − y), found by matching the silhouettes;
  * decimates it to about TARGET_TRIS triangles and smooths the normals;
  * keeps only the texture's luminance, at TEX × TEX;
  * finds the markers the app needs: the earbuds, her right eye, the pivot of the turn in her
    neck and the band over which the neck blends from the shoulders (still) to the head (turning).

Writes apps/macos/Resources/Onboarding/head.bin and head.png. head.bin, little endian:
    b"ASMRHEAD", u32 version (2), u32 vertex count, u32 index count, u32 0 (reserved),
    16 float32 markers: ear left xyz, ear right xyz, eye right xyz, pivot xyz, neck lo, neck hi, 0, 0
    vertices: float16 x, y, z, nx, ny, nz, u, v (v down, matching head.png's rows), 16 bytes
    indices: u16
head.png: 8-bit grey, the texture's luminance (sRGB values, as Dither.swift reads the portraits).
"""

import math
import os
import struct
import sys

import bmesh
import bpy
import numpy as np

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
OUT = os.path.join(ROOT, "apps", "macos", "Resources", "Onboarding")
SRC = os.path.join(ROOT, "tools", "fal", "dl", "head3d", "hunyuan31_front.glb")
TARGET_TRIS = 24000
TEX = 1024

# Where things are in front.png (u, v): the earbud in her left ear (the screen's right), her
# right eye (the screen's left).
POD_FRONT = (0.67, 0.47)
EYE_RIGHT = (0.426, 0.405)


def load(path):
    bpy.ops.wm.read_factory_settings(use_empty=True)
    bpy.ops.import_scene.gltf(filepath=path)
    objs = [o for o in bpy.context.scene.objects if o.type == "MESH"]
    bpy.ops.object.select_all(action="DESELECT")
    for o in objs:
        o.select_set(True)
    bpy.context.view_layer.objects.active = objs[0]
    if len(objs) > 1:
        bpy.ops.object.join()
    obj = bpy.context.view_layer.objects.active
    bpy.ops.object.transform_apply(location=True, rotation=True, scale=True)
    return obj


def portrait_mask(n):
    img = bpy.data.images.load(os.path.join(OUT, "front.png"))
    w, h = img.size
    px = np.empty(w * h * 4, dtype=np.float32)
    img.pixels.foreach_get(px)
    a = px.reshape(h, w, 4)[::-1, :, 3]  # top row first
    ys = (np.arange(n) + 0.5) / n * h
    xs = (np.arange(n) + 0.5) / n * w
    return a[ys.astype(int)][:, xs.astype(int)] > 0.5


def splat(xy, n):
    """Coverage of the image square by the points (model x, y), n × n, top row first."""
    m = np.zeros((n, n), dtype=bool)
    u = np.floor((xy[:, 0] + 0.5) * n).astype(int)
    v = np.floor((0.5 - xy[:, 1]) * n).astype(int)
    ok = (u >= 0) & (u < n) & (v >= 0) & (v < n)
    m[v[ok], u[ok]] = True
    # Close the gaps between the points.
    g = m.copy()
    g[1:] |= m[:-1]; g[:-1] |= m[1:]; g[:, 1:] |= m[:, :-1]; g[:, :-1] |= m[:, 1:]
    return g


def fit(obj):
    """Scale and offset taking the model's front view (Blender x, z) onto the portrait."""
    me = obj.data
    co = np.empty(len(me.vertices) * 3, dtype=np.float32)
    me.vertices.foreach_get("co", co)
    co = co.reshape(-1, 3)
    pts = co[np.random.default_rng(1).choice(len(co), min(len(co), 200000), replace=False)][:, [0, 2]]
    n = 128
    target = portrait_mask(n)
    lo, hi = pts.min(0), pts.max(0)
    # Start from the bounding boxes: her height from the top of the hair to the image's bottom.
    ys = np.nonzero(target.any(1))[0]
    top = 0.5 - ys.min() / n
    s0 = (top + 0.5) / (hi[1] - lo[1])
    best = (-1, None)
    for s in s0 * np.linspace(0.94, 1.06, 13):
        for tx in np.linspace(-0.06, 0.06, 13):
            for ty in np.linspace(-0.04, 0.04, 9):
                ox = -(lo[0] + hi[0]) / 2 * s + tx
                oy = top - hi[1] * s + ty
                m = splat(pts * s + np.array([ox, oy]), n)
                iou = (m & target).sum() / max(1, (m | target).sum())
                if iou > best[0]:
                    best = (iou, (s, ox, oy))
    print("fit: IoU %.3f scale %.4f offset %.4f %.4f" % (best[0], *best[1]))
    return best[1]


def to_model(obj, s, ox, oy):
    """Blender (x right, -y towards the viewer, z up) to the model space."""
    me = obj.data
    for v in me.vertices:
        x, y, z = v.co
        v.co = (x * s + ox, z * s + oy, -y * s)
    me.update()


def decimate(obj):
    # glTF splits the vertices along the texture's seams: weld them first (the UVs live on the
    # face corners), or the decimation treats every chart as its own piece.
    bm = bmesh.new()
    bm.from_mesh(obj.data)
    bmesh.ops.remove_doubles(bm, verts=bm.verts, dist=1e-5)
    bm.to_mesh(obj.data)
    bm.free()
    tris = sum(len(p.vertices) - 2 for p in obj.data.polygons)
    mod = obj.modifiers.new("dec", "DECIMATE")
    mod.ratio = min(1.0, TARGET_TRIS / tris)
    bpy.ops.object.modifier_apply(modifier=mod.name)
    bm = bmesh.new()
    bm.from_mesh(obj.data)
    # Drop specks: islands of under 0.5 % of the faces.
    seen, islands = set(), []
    for f in bm.faces:
        if f.index in seen:
            continue
        stack, isl = [f], []
        seen.add(f.index)
        while stack:
            g = stack.pop()
            isl.append(g)
            for e in g.edges:
                for h in e.link_faces:
                    if h.index not in seen:
                        seen.add(h.index)
                        stack.append(h)
        islands.append(isl)
    small = [f for isl in islands if len(isl) < 0.005 * len(bm.faces) for f in isl]
    bmesh.ops.delete(bm, geom=small, context="FACES")
    bmesh.ops.triangulate(bm, faces=bm.faces[:])
    bm.to_mesh(obj.data)
    bm.free()
    for p in obj.data.polygons:
        p.use_smooth = True
    obj.data.update()
    print("decimated:", len(obj.data.polygons), "triangles,", len(islands), "islands,", len(small), "faces dropped")


def texture_luminance(obj):
    img = None
    for m in obj.data.materials:
        for node in (m.node_tree.nodes if m and m.node_tree else []):
            if node.type == "TEX_IMAGE" and node.image:
                img = node.image
    w, h = img.size
    px = np.empty(w * h * 4, dtype=np.float32)
    img.pixels.foreach_get(px)
    px = px.reshape(h, w, 4)[::-1]  # top row first
    lum = 0.2126 * px[..., 0] + 0.7152 * px[..., 1] + 0.0722 * px[..., 2]
    k = w // TEX
    lum = lum[: TEX * k, : TEX * k].reshape(TEX, k, TEX, k).mean(axis=(1, 3))
    return np.clip(lum, 0, 1)


def export(obj, lum):
    me = obj.data
    me.calc_loop_triangles()
    uvl = me.uv_layers.active.data
    co = [v.co for v in me.vertices]
    nrm = [v.normal for v in me.vertices]
    index, verts, idx = {}, [], []
    for tri in me.loop_triangles:
        for li in tri.loops:
            vi = me.loops[li].vertex_index
            uv = uvl[li].uv
            key = (vi, round(uv[0], 5), round(uv[1], 5))
            if key not in index:
                index[key] = len(verts)
                c, n = co[vi], nrm[vi]
                verts.append((c.x, c.y, c.z, n.x, n.y, n.z, uv[0], 1 - uv[1]))
            idx.append(index[key])
    V = np.array([v[:3] for v in verts])
    markers = find_markers(V)
    with open(os.path.join(OUT, "head.bin"), "wb") as f:
        f.write(b"ASMRHEAD")
        assert len(verts) < 65536
        f.write(struct.pack("<4I", 2, len(verts), len(idx), 0))
        f.write(struct.pack("<16f", *markers))
        f.write(np.array(verts, dtype="<f2").tobytes())
        f.write(np.array(idx, dtype="<u2").tobytes())
    out = bpy.data.images.new("head_lum", TEX, TEX, alpha=False)
    rgba = np.repeat(lum[::-1, :, None], 4, axis=2)
    rgba[..., 3] = 1
    out.pixels.foreach_set(rgba.astype(np.float32).ravel())
    out.filepath_raw = os.path.join(OUT, "head.png")
    out.file_format = "PNG"
    sc = bpy.context.scene
    sc.render.image_settings.color_mode = "BW"
    sc.render.image_settings.color_depth = "8"
    sc.render.image_settings.compression = 100
    out.save_render(os.path.join(OUT, "head.png"), scene=sc)
    print("wrote", len(verts), "vertices,", len(idx) // 3, "triangles; markers", ["%.3f" % m for m in markers])


def find_markers(V):
    def front_point(u, v, r=0.012):
        x, y = u - 0.5, 0.5 - v
        near = (np.abs(V[:, 0] - x) < r) & (np.abs(V[:, 1] - y) < r)
        z = V[near, 2].max()
        return np.array([x, y, z])

    ear_l = front_point(*POD_FRONT)
    eye_r = front_point(*EYE_RIGHT)
    # Her head's middle at eye level, and the neck: the slice under the chin.
    band = V[np.abs(V[:, 1] - eye_r[1]) < 0.02]
    mid = (band[:, 0].min() + band[:, 0].max()) / 2
    # The other earbud: the left one mirrored, pushed out to the surface (it sits under her hair).
    ry, rz = ear_l[1], ear_l[2]
    side = V[(np.abs(V[:, 1] - ry) < 0.015) & (np.abs(V[:, 2] - rz) < 0.03) & (V[:, 0] < mid)]
    ear_r = np.array([side[:, 0].min(), ry, rz]) if len(side) else np.array([2 * mid - ear_l[0], ry, rz])
    neck_y = -0.11
    neck = V[(np.abs(V[:, 1] - neck_y) < 0.01) & (np.abs(V[:, 0] - mid) < 0.09)]
    pivot = np.array([mid, -0.07, (neck[:, 2].min() + neck[:, 2].max()) / 2 if len(neck) else 0.0])
    return [*ear_l, *ear_r, *eye_r, *pivot, -0.235, -0.125, 0.0, 0.0]


def main():
    argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
    obj = load(argv[0] if argv else SRC)
    s, ox, oy = fit(obj)
    to_model(obj, s, ox, oy)
    lum = texture_luminance(obj)
    decimate(obj)
    export(obj, lum)


main()
