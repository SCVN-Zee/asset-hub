# Asset Hub: imports @@MODEL@@ with its Unity materials.
# In Blender: Scripting > Open > this file > Run Script, or: blender --python open_in_blender.py
import bpy, json, os, re, shutil

HERE = os.path.dirname(os.path.abspath(__file__))
DATA = json.loads(@@DATA@@)

# An empty scene (no default cube, camera or light).
for o in list(bpy.data.objects):
    bpy.data.objects.remove(o, do_unlink=True)

before = set(bpy.data.materials)
path = os.path.join(HERE, DATA["model"])
if path.lower().endswith((".glb", ".gltf")):
    bpy.ops.import_scene.gltf(filepath=path)
else:
    bpy.ops.import_scene.fbx(filepath=path)


def base_name(name):
    return re.sub(r"\.\d{3}$", "", name)


def socket(node, *names):
    for n in names:
        s = node.inputs.get(n)
        if s is not None:
            return s
    return None


def build(mat, spec):
    if mat.node_tree is None:
        mat.use_nodes = True
    tree = mat.node_tree
    nodes, links = tree.nodes, tree.links
    bsdf = next((n for n in nodes if n.type == "BSDF_PRINCIPLED"), None)
    if bsdf is None:
        bsdf = nodes.new("ShaderNodeBsdfPrincipled")
        out = next((n for n in nodes if n.type == "OUTPUT_MATERIAL"), None) or nodes.new("ShaderNodeOutputMaterial")
        links.new(bsdf.outputs[0], out.inputs["Surface"])
    x = bsdf.location.x - 700
    y = [bsdf.location.y + 300]

    def place(node, dx=0):
        node.location = (x + dx, y[0])
        y[0] -= 280
        return node

    def image(file, data):
        img = bpy.data.images.load(os.path.join(HERE, file), check_existing=True)
        if data:
            img.colorspace_settings.name = "Non-Color"
        n = place(nodes.new("ShaderNodeTexImage"))
        n.image = img
        return n

    def math(op, a, b, c=None):
        n = nodes.new("ShaderNodeMath")
        n.operation = op
        n.location = (x + 350, y[0] + 140)
        for i, v in enumerate((a, b) if c is None else (a, b, c)):
            if isinstance(v, (int, float)):
                n.inputs[i].default_value = v
            else:
                links.new(v, n.inputs[i])
        return n.outputs[0]

    def vmul(a, b):
        n = nodes.new("ShaderNodeVectorMath")
        n.operation = "MULTIPLY"
        n.location = (x + 350, y[0] + 140)
        links.new(a, n.inputs[0])
        if isinstance(b, (tuple, list)):
            n.inputs[1].default_value = b
        else:
            links.new(b, n.inputs[1])
        return n.outputs[0]

    def separate(img):
        n = nodes.new("ShaderNodeSeparateColor") if hasattr(bpy.types, "ShaderNodeSeparateColor") else nodes.new("ShaderNodeSeparateRGB")
        n.location = (x + 300, y[0] + 280)
        links.new(img.outputs["Color"], n.inputs[0])
        return n.outputs

    r, g, b, a = spec["color"]
    color = alpha = color_img = None
    if spec.get("colorMap"):
        img = color_img = image(spec["colorMap"], False)
        color = img.outputs["Color"]
        alpha = img.outputs["Alpha"]
        if (r, g, b) != (1, 1, 1):
            color = vmul(color, (r, g, b))
        if a != 1:
            alpha = math("MULTIPLY", alpha, a)

    # Occlusion darkens the colour (Principled has no input of its own): 1 + strength * (ao - 1).
    ao = None
    if spec.get("occlusionMap"):
        ao = separate(image(spec["occlusionMap"], True))[1]
    mask_img = image(spec["maskMap"], True) if spec.get("maskMap") else None
    mask = separate(mask_img) if mask_img else None
    if mask and ao is None:
        ao = mask[1]
    if ao is not None:
        k = spec["occlusionStrength"]
        ao = math("MULTIPLY_ADD", ao, k, 1 - k)
        color = vmul(color, ao) if color is not None else None
        if color is None:
            n = nodes.new("ShaderNodeCombineXYZ")
            n.location = (x + 350, y[0])
            for i in range(3):
                links.new(math("MULTIPLY", ao, (r, g, b)[i]), n.inputs[i])
            color = n.outputs[0]
    base = socket(bsdf, "Base Color")
    if color is not None:
        links.new(color, base)
    else:
        base.default_value = (r, g, b, 1)

    # Metallic and smoothness: a map's R and A, else the numbers. Roughness = 1 - smoothness.
    metal_img = image(spec["metallicMap"], True) if spec.get("metallicMap") else mask_img
    s = spec["smoothness"]
    if metal_img is not None:
        links.new(separate(metal_img)[0], socket(bsdf, "Metallic"))
    else:
        socket(bsdf, "Metallic").default_value = spec["metallic"]
    gloss = None
    if spec["smoothnessFrom"] == "map" and metal_img is not None:
        gloss = metal_img.outputs["Alpha"]
    elif spec["smoothnessFrom"] == "albedo" and color_img is not None:
        gloss = color_img.outputs["Alpha"]
    if gloss is not None:
        links.new(math("MULTIPLY_ADD", gloss, -s, 1), socket(bsdf, "Roughness"))
    else:
        socket(bsdf, "Roughness").default_value = 1 - s

    if spec.get("normalMap"):
        nmap = nodes.new("ShaderNodeNormalMap")
        img = image(spec["normalMap"], True)
        nmap.location = (x + 350, y[0] + 280)
        nmap.inputs["Strength"].default_value = spec["normalScale"]
        links.new(img.outputs["Color"], nmap.inputs["Color"])
        links.new(nmap.outputs["Normal"], bsdf.inputs["Normal"])

    emit = socket(bsdf, "Emission Color", "Emission")
    strength = socket(bsdf, "Emission Strength")
    if spec.get("emission"):
        e = spec["emission"]
        if spec.get("emissionMap"):
            links.new(vmul(image(spec["emissionMap"], False).outputs["Color"], tuple(e)), emit)
        else:
            emit.default_value = (e[0], e[1], e[2], 1)
        if strength is not None:
            strength.default_value = 1.0

    # Unlit shaders: the colour as it is, lit by nothing.
    if spec["unlit"]:
        if color is not None:
            links.new(color, emit)
        else:
            emit.default_value = (r, g, b, 1)
        if strength is not None:
            strength.default_value = 1.0
        socket(bsdf, "Roughness").default_value = 1
        spec_in = socket(bsdf, "Specular IOR Level", "Specular")
        if spec_in is not None:
            spec_in.default_value = 0

    if spec["surface"] != "opaque":
        al = alpha if alpha is not None else a
        if spec["surface"] == "cutout" and alpha is not None:
            al = math("GREATER_THAN", alpha, spec["cutoff"])
        if isinstance(al, (int, float)):
            socket(bsdf, "Alpha").default_value = al
        else:
            links.new(al, socket(bsdf, "Alpha"))
        if hasattr(mat, "surface_render_method"):
            mat.surface_render_method = "BLENDED" if spec["surface"] == "transparent" else "DITHERED"
        elif hasattr(mat, "blend_method"):
            mat.blend_method = "BLEND" if spec["surface"] == "transparent" else "HASHED"

    if spec.get("name"):
        mat.name = spec["name"]


specs = DATA["materials"]
for mat in [m for m in bpy.data.materials if m not in before]:
    textured = mat.node_tree is not None and any(n.type == "TEX_IMAGE" for n in mat.node_tree.nodes)
    name = base_name(mat.name)
    spec = next((s for s in specs if s["target"] == name), None) or next((s for s in specs if s["target"] == "*"), None)
    if spec is None and not textured:
        spec = next((s for s in specs if s["target"] == "?"), None)
    if spec is not None:
        build(mat, spec)

# Textures go inside the .blend when it is saved, so it does not depend on this folder.
try:
    bpy.ops.file.pack_all()
except Exception:
    pass

# A temporary copy (opened by Asset Hub): everything is in Blender now.
if DATA.get("cleanup"):
    shutil.rmtree(HERE, ignore_errors=True)
