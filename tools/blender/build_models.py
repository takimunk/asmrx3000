"""Builds the scene models for the macOS popover in headless Blender and writes them to
apps/macos/Resources/models.bin.

    /Applications/Blender.app/Contents/MacOS/Blender -b --factory-startup --python tools/blender/build_models.py

Everything is procedural (skin-modifier skeletons, subdivision, noise displacement) and seeded,
so the output is reproducible. Units are metres, +x forward, +y left, +z up (same as the engine).

File format (little endian):
    b"ASMRMDL1", u32 model count, then per model:
    32-byte name, u32 vertex count, u32 index count,
    vertices: float32 x, y, z, nx, ny, nz, part, pad
    indices:  u32
Parts: 0 wood, 1 foliage, 2 feathers, 3 left wing, 4 right wing, 5 stone, 6 beak.
"""

import math
import os
import random
import struct

import bmesh
import bpy
from mathutils import Matrix, Vector, noise

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
OUT = os.path.join(ROOT, "apps", "macos", "Resources", "models.bin")

WOOD, FOLIAGE, FEATHERS, WING_L, WING_R, STONE, BEAK = range(7)


def reset():
    bpy.ops.wm.read_factory_settings(use_empty=True)


def evaluated_mesh(obj):
    dg = bpy.context.evaluated_depsgraph_get()
    ev = obj.evaluated_get(dg)
    me = bpy.data.meshes.new_from_object(ev, depsgraph=dg)
    me.transform(obj.matrix_world)
    return me


def link(obj):
    bpy.context.scene.collection.objects.link(obj)
    return obj


def skeleton(name, points, edges, radii, subdiv=1):
    """A skin-modifier tube along a branching skeleton: smooth, organic limbs."""
    me = bpy.data.meshes.new(name)
    me.from_pydata([tuple(p) for p in points], edges, [])
    obj = link(bpy.data.objects.new(name, me))
    obj.modifiers.new("skin", "SKIN")
    layer = me.skin_vertices[0] if len(me.skin_vertices) else me.skin_vertices.new()
    for i, r in enumerate(radii):
        layer.data[i].radius = (r, r)
    layer.data[0].use_root = True
    sub = obj.modifiers.new("sub", "SUBSURF")
    sub.levels = subdiv
    sub.render_levels = subdiv
    return obj


def blob(name, center, radius, seed, detail=2, squash=(1, 1, 1), rough=0.28, scale=1.6):
    """A displaced icosphere: a leaf cluster or a stone."""
    bm = bmesh.new()
    bmesh.ops.create_icosphere(bm, subdivisions=detail, radius=radius)
    off = Vector((seed * 7.31, seed * 3.17, seed * 5.53))
    for v in bm.verts:
        n = v.co.normalized()
        d = noise.noise(n * scale + off) + 0.5 * noise.noise(n * scale * 2.3 + off)
        v.co = n * radius * (1 + rough * d)
        v.co = Vector((v.co.x * squash[0], v.co.y * squash[1], v.co.z * squash[2])) + Vector(center)
    me = bpy.data.meshes.new(name)
    bm.to_mesh(me)
    bm.free()
    for poly in me.polygons:
        poly.use_smooth = True
    return link(bpy.data.objects.new(name, me))


def cone(name, center, radius, depth, segments, seed, jag=0.18):
    """A spruce skirt: a jagged, drooping cone of needles."""
    bm = bmesh.new()
    rnd = random.Random(seed)
    top = bm.verts.new(Vector(center) + Vector((0, 0, depth * 0.35)))
    ring = []
    for i in range(segments):
        a = 2 * math.pi * i / segments
        r = radius * (1 - jag * (i % 2) * rnd.uniform(0.6, 1.2)) * (0.9 + 0.2 * rnd.random())
        droop = -depth * (0.65 + 0.2 * (i % 2))
        ring.append(bm.verts.new(Vector(center) + Vector((r * math.cos(a), r * math.sin(a), droop))))
    inner = bm.verts.new(Vector(center) + Vector((0, 0, -depth * 0.35)))
    for i in range(segments):
        a, b = ring[i], ring[(i + 1) % segments]
        bm.faces.new((top, a, b))
        bm.faces.new((inner, b, a))
    me = bpy.data.meshes.new(name)
    bm.to_mesh(me)
    bm.free()
    for poly in me.polygons:
        poly.use_smooth = True
    obj = link(bpy.data.objects.new(name, me))
    sub = obj.modifiers.new("sub", "SUBSURF")
    sub.levels = 1
    return obj


# ---- models -----------------------------------------------------------------------------------


def broadleaf(seed):
    """A romantic oak: short trunk, a few limbs, a billowing domed crown centred near z = 4.2."""
    rnd = random.Random(seed)
    pts = [Vector((0, 0, -0.3)), Vector((0, 0, 1.0)), Vector((0.06, 0.03, 2.1))]
    edges, radii = [(0, 1), (1, 2)], [0.34, 0.25, 0.21]
    for k in range(5):
        a = 2 * math.pi * k / 5 + rnd.uniform(-.3, .3)
        lean = rnd.uniform(0.45, 0.7)
        mid = pts[2] + Vector((math.cos(a) * lean, math.sin(a) * lean, 1.0))
        end = mid + Vector((math.cos(a) * lean * 0.9, math.sin(a) * lean * 0.9, 0.9 + rnd.uniform(0, .4)))
        i = len(pts)
        pts += [mid, end]
        radii += [0.12, 0.06]
        edges += [(2, i), (i, i + 1)]
    wood = skeleton("oak_wood", pts, edges, radii)
    leaves = []
    centre = Vector((0, 0, 4.2))
    n = 64
    for k in range(n):
        # Fibonacci points over an ellipsoid, the upper part fuller: a cumulus-like crown.
        y = 1 - (k + 0.5) / n * 1.55
        r = math.sqrt(max(0.0, 1 - y * y))
        a = k * 2.399963
        p = Vector((math.cos(a) * r * 1.95, math.sin(a) * r * 1.95, y * 1.5)) + centre
        p += Vector((rnd.uniform(-.2, .2), rnd.uniform(-.2, .2), rnd.uniform(-.15, .15)))
        leaves.append(blob(f"oak_leaf{k}", p, rnd.uniform(0.4, 0.62), seed * 31 + k, detail=2, squash=(1, 1, 0.8), rough=0.35, scale=2.4))
    for k in range(5):
        a = 2 * math.pi * k / 5
        p = centre + Vector((math.cos(a) * 0.8, math.sin(a) * 0.8, -0.2))
        leaves.append(blob(f"oak_core{k}", p, 1.0, seed * 57 + k, detail=2, squash=(1, 1, 0.8), rough=0.2))
    return [(wood, WOOD)] + [(l, FOLIAGE) for l in leaves]


def spruce(seed):
    """A slender spruce, base at the origin, crown centre near z = 3.6, tip near 6.8."""
    rnd = random.Random(seed)
    wood = skeleton("spruce_wood", [Vector((0, 0, -0.3)), Vector((0, 0, 3.0)), Vector((0, 0, 6.8))], [(0, 1), (1, 2)],
                    [0.19, 0.12, 0.03])
    skirts = []
    z, n = 0.9, 0
    while z < 6.5:
        t = (z - 0.9) / 5.9
        r = 0.12 + 1.5 * (1 - t) ** 1.1 * rnd.uniform(0.9, 1.08)
        sk = cone(f"spruce_skirt{n}", (0, 0, z + 0.5), r, 0.55 + 0.25 * (1 - t), 24, seed * 13 + n, jag=0.3)
        sk.rotation_euler = (rnd.uniform(-.05, .05), rnd.uniform(-.05, .05), rnd.uniform(0, math.pi))
        skirts.append(sk)
        z += 0.28 + 0.1 * rnd.random()
        n += 1
    bpy.context.view_layer.update()
    return [(wood, WOOD)] + [(s, FOLIAGE) for s in skirts]


def bird(flying):
    """A songbird facing +x, ~0.2 m long, body centred at the origin. Perched: wings folded
    against the body. Flying: wings spread along y, parts 3/4 so the app can beat them about x."""
    body = skeleton("bird_body",
                    [Vector((-0.13, 0, 0.02)), Vector((-0.07, 0, 0.005)), Vector((-0.01, 0, 0.0)), Vector((0.04, 0, 0.015)),
                     Vector((0.075, 0, 0.045)), Vector((0.095, 0, 0.06))],
                    [(0, 1), (1, 2), (2, 3), (3, 4), (4, 5)], [0.006, 0.035, 0.052, 0.048, 0.032, 0.036], subdiv=2)
    tail = skeleton("bird_tail", [Vector((-0.1, 0, 0.02)), Vector((-0.19, 0, 0.035))], [(0, 1)], [0.012, 0.02], subdiv=1)
    tail.scale = (1, 1.6, 0.3)
    beak = skeleton("bird_beak", [Vector((0.122, 0, 0.058)), Vector((0.155, 0, 0.052))], [(0, 1)], [0.012, 0.002], subdiv=1)
    parts = [(body, FEATHERS), (tail, FEATHERS), (beak, BEAK)]
    for side, part in ((1, WING_L), (-1, WING_R)):
        if flying:
            w = skeleton(f"bird_wing{side}",
                         [Vector((0.01, side * 0.03, 0.03)), Vector((-0.01, side * 0.12, 0.035)), Vector((-0.05, side * 0.21, 0.03))],
                         [(0, 1), (1, 2)], [0.035, 0.03, 0.01], subdiv=1)
            w.scale = (1, 1, 0.22)
            parts.append((w, part))
        else:
            w = blob(f"bird_wing{side}", (-0.03, side * 0.038, 0.028), 0.05, 4 + side, detail=2,
                     squash=(1.7, 0.42, 0.62), rough=0.05)
            parts.append((w, FEATHERS))
    bpy.context.view_layer.update()
    return parts


def herm():
    """The listener: a small classical herm (a head on a tapering pillar), head centre at z = 0,
    facing +x. Its base reaches z = -1.75."""
    bm = bmesh.new()
    # Tapering square pillar with a plinth and a moulded cap.
    def box(cx, cy, cz, sx, sy, sz):
        res = bmesh.ops.create_cube(bm, size=1.0)
        for v in res["verts"]:
            v.co = Vector((v.co.x * sx + cx, v.co.y * sy + cy, v.co.z * sz + cz))
        return res["verts"]
    shaft = box(0, 0, -0.95, 0.24, 0.24, 1.3)
    for v in shaft:
        if v.co.z > -0.5:
            v.co.x *= 0.82
            v.co.y *= 0.82
    box(0, 0, -1.68, 0.36, 0.36, 0.14)
    box(0, 0, -0.27, 0.3, 0.3, 0.06)
    me = bpy.data.meshes.new("herm_pillar")
    bm.to_mesh(me)
    bm.free()
    pillar = link(bpy.data.objects.new("herm_pillar", me))
    bev = pillar.modifiers.new("bevel", "BEVEL")
    bev.width = 0.015
    bev.segments = 2

    # Bust: shoulders, neck, head; nose, brow and a knot of hair.
    bust = skeleton("herm_bust",
                    [Vector((0, 0, -0.24)), Vector((0, 0, -0.12)), Vector((0, 0, 0.0)), Vector((0, 0, 0.09)),
                     Vector((0, 0.17, -0.2)), Vector((0, -0.17, -0.2))],
                    [(0, 1), (1, 2), (2, 3), (0, 4), (0, 5)], [0.13, 0.06, 0.1, 0.085, 0.07, 0.07], subdiv=2)
    bust.scale = (0.85, 1, 1)
    bpy.context.view_layer.update()
    nose = skeleton("herm_nose", [Vector((0.075, 0, 0.02)), Vector((0.11, 0, -0.015))], [(0, 1)], [0.018, 0.012], subdiv=1)
    hair = blob("herm_hair", (-0.02, 0, 0.07), 0.1, 3, detail=2, squash=(1.0, 0.95, 0.85), rough=0.12, scale=4.0)
    return [(pillar, STONE), (bust, STONE), (nose, STONE), (hair, STONE)]


def column():
    """A fallen fluted column drum and a broken base: the ruin by the river."""
    parts = []
    bm = bmesh.new()
    flutes, rings = 20, 12
    length, radius = 1.6, 0.26
    verts = []
    for j in range(rings + 1):
        x = -length / 2 + length * j / rings
        row = []
        for i in range(flutes * 2):
            a = 2 * math.pi * i / (flutes * 2)
            r = radius * (0.94 if i % 2 else 1.0)
            # The broken end is jagged.
            if j == rings:
                x2 = x - 0.18 * (0.5 + 0.5 * noise.noise(Vector((a * 2, 0.3, 0.7))))
            else:
                x2 = x
            row.append(bm.verts.new((x2, r * math.cos(a), r * math.sin(a))))
        verts.append(row)
    n = flutes * 2
    for j in range(rings):
        for i in range(n):
            bm.faces.new((verts[j][i], verts[j][(i + 1) % n], verts[j + 1][(i + 1) % n], verts[j + 1][i]))
    bm.faces.new(list(reversed(verts[0])))
    bm.faces.new(verts[rings])
    me = bpy.data.meshes.new("column_drum")
    bm.to_mesh(me)
    bm.free()
    drum = link(bpy.data.objects.new("column_drum", me))
    drum.rotation_euler = (0.08, 0, 0.5)
    drum.location = (0, 0, radius - 0.04)
    parts.append((drum, STONE))

    base = blob("column_base", (0.9, -0.55, 0.12), 0.32, 9, detail=2, squash=(1.1, 1.1, 0.5), rough=0.18, scale=2.0)
    stump = skeleton("column_stump", [Vector((0.9, -0.55, 0.1)), Vector((0.9, -0.55, 0.55))], [(0, 1)], [0.25, 0.24], subdiv=1)
    parts.append((base, STONE))
    parts.append((stump, STONE))
    for k in range(3):
        parts.append((blob(f"stone{k}", (-0.6 + 0.5 * k, 0.5 - 0.3 * k, 0.05), 0.1 + 0.04 * k, 20 + k, detail=1,
                           squash=(1.2, 1, 0.6), rough=0.3), STONE))
    return parts


def campfire():
    """A ring of stones with three logs leaned together, centred on the origin at ground level."""
    parts = []
    rnd = random.Random(31)
    for k in range(10):
        a = 2 * math.pi * k / 10 + rnd.uniform(-0.1, 0.1)
        c = (math.cos(a) * 0.48, math.sin(a) * 0.48, 0.05)
        parts.append((blob(f"fire_stone{k}", c, rnd.uniform(0.09, 0.13), 40 + k, detail=1, squash=(1.2, 1.0, 0.7), rough=0.3), STONE))
    for k in range(3):
        a = 2 * math.pi * k / 3 + 0.4
        foot = Vector((math.cos(a) * 0.36, math.sin(a) * 0.36, 0.03))
        top = Vector((math.cos(a) * 0.04, math.sin(a) * 0.04, 0.42))
        parts.append((skeleton(f"fire_log{k}", [foot, top], [(0, 1)], [0.045, 0.035], subdiv=1), WOOD))
    return parts


# ---- export -----------------------------------------------------------------------------------


def collect(parts):
    """Triangulated, welded vertices and indices for a list of (object, part)."""
    verts, index, lookup = [], [], {}
    for obj, part in parts:
        me = evaluated_mesh(obj)
        me.calc_loop_triangles()
        normals = me.corner_normals
        for tri in me.loop_triangles:
            for loop in tri.loops:
                co = me.vertices[me.loops[loop].vertex_index].co
                n = normals[loop].vector
                key = (round(co.x, 5), round(co.y, 5), round(co.z, 5), round(n.x, 3), round(n.y, 3), round(n.z, 3), part)
                i = lookup.get(key)
                if i is None:
                    i = len(verts)
                    lookup[key] = i
                    verts.append((co.x, co.y, co.z, n.x, n.y, n.z, float(part), 0.0))
                index.append(i)
        bpy.data.meshes.remove(me)
    return verts, index


def preview(path):
    """Lays every model out in a row and renders a workbench image, for checking shapes."""
    reset()
    scene = bpy.context.scene
    x = -12.0
    with open(OUT, "rb") as f:
        assert f.read(8) == b"ASMRMDL1"
        (count,) = struct.unpack("<I", f.read(4))
        for _ in range(count):
            name = f.read(32).rstrip(b"\0").decode()
            nv, ni = struct.unpack("<II", f.read(8))
            data = struct.unpack(f"<{nv * 8}f", f.read(nv * 32))
            idx = struct.unpack(f"<{ni}I", f.read(ni * 4))
            scale = 8.0 if name.startswith("bird") else 1.0
            verts = [(data[i * 8] * scale + x, data[i * 8 + 1] * scale, data[i * 8 + 2] * scale + (1.75 if name == "herm" else 0)) for i in range(nv)]
            faces = [idx[i:i + 3] for i in range(0, ni, 3)]
            me = bpy.data.meshes.new(name)
            me.from_pydata(verts, [], faces)
            for poly in me.polygons:
                poly.use_smooth = True
            link(bpy.data.objects.new(name, me))
            x += 4.2 if name != "broadleaf_b" else 5.0
    cam = bpy.data.objects.new("cam", bpy.data.cameras.new("cam"))
    link(cam)
    cam.location = (0.4, -34, 4)
    cam.rotation_euler = (math.radians(82), 0, 0)
    cam.data.lens = 40
    scene.camera = cam
    scene.render.engine = "BLENDER_WORKBENCH"
    scene.render.resolution_x, scene.render.resolution_y = 1800, 600
    scene.render.filepath = path
    bpy.ops.render.render(write_still=True)


def main():
    builders = [
        ("broadleaf", lambda: broadleaf(7)),
        ("broadleaf_b", lambda: broadleaf(23)),
        ("spruce", lambda: spruce(5)),
        ("spruce_b", lambda: spruce(11)),
        ("bird", lambda: bird(False)),
        ("bird_flying", lambda: bird(True)),
        ("herm", herm),
        ("ruin", column),
        ("campfire", campfire),
    ]
    models = []
    for name, build in builders:
        reset()
        for o in bpy.context.scene.objects:
            o.select_set(False)
        parts = build()
        bpy.context.view_layer.update()
        verts, index = collect(parts)
        print(f"{name}: {len(verts)} vertices, {len(index) // 3} triangles")
        models.append((name, verts, index))

    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    with open(OUT, "wb") as f:
        f.write(b"ASMRMDL1")
        f.write(struct.pack("<I", len(models)))
        for name, verts, index in models:
            f.write(name.encode().ljust(32, b"\0"))
            f.write(struct.pack("<II", len(verts), len(index)))
            for v in verts:
                f.write(struct.pack("<8f", *v))
            f.write(struct.pack(f"<{len(index)}I", *index))
    print("wrote", OUT, os.path.getsize(OUT), "bytes")
    if os.environ.get("PREVIEW"):
        preview(os.environ["PREVIEW"])


main()
