using System.Text.Json;
using CUE4Parse.FileProvider;
using CUE4Parse.UE4.Assets.Exports;
using CUE4Parse.UE4.Assets.Exports.Material;
using CUE4Parse.UE4.Assets.Exports.StaticMesh;
using CUE4Parse.UE4.Assets.Exports.Component.StaticMesh;
using CUE4Parse_Conversion.Options;
using CUE4Parse_Conversion.Meshes;
using CUE4Parse.UE4.Assets.Exports.Texture;
using CUE4Parse.UE4.Objects.Core.Math;
using CUE4Parse.UE4.Objects.UObject;
using CUE4Parse_Conversion.Textures;
using SkiaSharp;

// scene <umap>: static-mesh placements of a level and its always-loaded sublevels -> <out>/scene.glb (+ scene.json: player start).
// UE (cm, X fwd / Y right / Z up, left-handed) -> glTF/Bevy (m, Y up, -Z fwd): p' = (y, z, -x) * 0.01.
static class SceneExport
{
    const int MaxTex = 512;
    static double[] Basis(FVector loc, FRotator r, FVector s)
    {
        double rad = Math.PI / 180, p = r.Pitch * rad, y = r.Yaw * rad, ro = r.Roll * rad;
        double sp = Math.Sin(p), cp = Math.Cos(p), sy = Math.Sin(y), cy = Math.Cos(y), sr = Math.Sin(ro), cr = Math.Cos(ro);
        // columns = scaled local axes in parent space, then translation (UE FRotationMatrix)
        double[] X = [cp * cy, cp * sy, sp], Y = [sr * sp * cy - cr * sy, sr * sp * sy + cr * cy, -sr * cp], Z = [-(cr * sp * cy + sr * sy), cy * sr - cr * sp * sy, cr * cp];
        var m = new double[12];
        for (int i = 0; i < 3; i++) { m[i * 4] = X[i] * s.X; m[i * 4 + 1] = Y[i] * s.Y; m[i * 4 + 2] = Z[i] * s.Z; m[i * 4 + 3] = i == 0 ? loc.X : i == 1 ? loc.Y : loc.Z; }
        return m;
    }
    static double[] Mul(double[] a, double[] b) // 3x4 affine a*b
    {
        var m = new double[12];
        for (int i = 0; i < 3; i++) for (int j = 0; j < 4; j++) m[i * 4 + j] = a[i * 4] * b[j] + a[i * 4 + 1] * b[4 + j] + a[i * 4 + 2] * b[8 + j] + (j == 3 ? a[i * 4 + 3] : 0);
        return m;
    }
    static double[] Local(UObject c) => Basis(c.GetOrDefault("RelativeLocation", FVector.ZeroVector), c.GetOrDefault("RelativeRotation", FRotator.ZeroRotator), c.GetOrDefault("RelativeScale3D", FVector.OneVector));
    static double[] World(UObject c, int depth = 0)
    {
        var m = Local(c);
        var parent = c.GetOrDefault<FPackageIndex?>("AttachParent", null);
        if (depth < 16 && parent != null && !parent.IsNull && parent.Load() is UObject pc) m = Mul(World(pc, depth + 1), m);
        return m;
    }
    // UE world matrix -> glTF column-major 4x4 node matrix: C * M * C^-1, C maps (x,y,z)->(y,z,-x)*0.01 (translation scaled by 0.01 only)
    static float[] Node(double[] m)
    {
        double[,] M = new double[3, 4];
        for (int i = 0; i < 3; i++) for (int j = 0; j < 4; j++) M[i, j] = m[i * 4 + j];
        int[] perm = [1, 2, 0]; double[] sg = [1, 1, -1]; // row r of C picks UE axis perm[r] with sign sg[r]
        var o = new float[16];
        for (int r = 0; r < 3; r++)
            for (int c = 0; c < 4; c++)
            {
                double v = sg[r] * (c < 3 ? M[perm[r], perm[c]] * sg[c] : M[perm[r], 3] * 0.01);
                o[c * 4 + r] = (float)v;
            }
        o[15] = 1; return o;
    }

    record Tex(string Name, byte[] Data, string Mime);
    class Mat { public string Name = ""; public int Tex = -1; public float[] Color = [0.6f, 0.6f, 0.6f, 1]; }
    class Prim { public byte[] Pos = [], Nrm = [], Uv = [], Idx = []; public int Verts, Indices, Mat; public float[] Min = [], Max = []; }

    public static void Run(DefaultFileProvider provider, string outDir, string umap, double radiusCm)
    {
        var texList = new List<Tex>(); var texIdx = new Dictionary<string, int>();
        var matList = new List<Mat>(); var matIdx = new Dictionary<string, int>();
        var meshList = new List<List<Prim>>(); var meshIdx = new Dictionary<string, int>();
        var nodes = new List<float[]>(); var nodeMesh = new List<int>(); var nodeName = new List<string>();
        object? playerStart = null;
        var seen = new HashSet<string>(StringComparer.OrdinalIgnoreCase); var todo = new Queue<string>(); todo.Enqueue(umap);
        int skipped = 0; var pending = new List<(UObject e, UStaticMesh sm, double[] w)>();
        while (todo.Count > 0)
        {
            var lv = todo.Dequeue(); if (!seen.Add(lv)) continue;
            Console.WriteLine("level " + lv);
            IEnumerable<UObject> ex;
            try { ex = provider.LoadPackage(lv).GetExports().ToList(); } catch (Exception e) { Console.WriteLine("  load fail " + e.Message); continue; }
            foreach (var e in ex)
            {
                try
                {
                    if (e.ExportType == "LevelStreamingAlwaysLoaded")
                    {
                        var wa = e.GetOrDefault<FSoftObjectPath>("WorldAsset").AssetPathName.Text;
                        if (wa.Contains("GlobalPostProcess")) continue;
                        todo.Enqueue("Bodycam/Content" + wa["/Game".Length..wa.LastIndexOf('.')]);
                    }
                    else if (e.ExportType == "PlayerStart" && playerStart == null)
                    {
                        var rc = e.GetOrDefault<FPackageIndex>("RootComponent").Load()!; var w = World(rc);
                        var rot = rc.GetOrDefault("RelativeRotation", FRotator.ZeroRotator);
                        playerStart = new { map = lv, loc_cm = new[] { w[3], w[7], w[11] }, yaw_deg = rot.Yaw };
                        Console.WriteLine("PlayerStart " + JsonSerializer.Serialize(playerStart));
                    }
                    else if (e.ExportType is "StaticMeshComponent" or "InstancedStaticMeshComponent" or "HierarchicalInstancedStaticMeshComponent")
                    {
                        if (e.GetOrDefault("bHiddenInGame", false) || !e.GetOrDefault("bVisible", true) || !e.GetOrDefault("bRenderInMainPass", true)) { skipped++; continue; }
                        var smRef = e.GetOrDefault<FPackageIndex?>("StaticMesh", null);
                        if (smRef == null || smRef.IsNull || smRef.Load() is not UStaticMesh sm) { skipped++; continue; }
                        var wm = World(e);
                        if (e is UInstancedStaticMeshComponent ism)
                            foreach (var inst in ism.PerInstanceSMData ?? [])
                            { var t = inst.TransformData; pending.Add((e, sm, Mul(wm, Basis(t.Translation, t.Rotator(), t.Scale3D)))); }
                        else pending.Add((e, sm, wm));
                    }
                }
                catch (Exception ex2) { Console.WriteLine($"  skip {e.Name}: {ex2.Message}"); skipped++; }
            }
        }
        var ps = (dynamic?)playerStart; double[]? c0 = ps == null ? null : (double[])ps.loc_cm;
        foreach (var (e, sm, w) in pending)
        {
            if (c0 != null && Math.Sqrt(Math.Pow(w[3] - c0[0], 2) + Math.Pow(w[7] - c0[1], 2)) > radiusCm) { skipped++; continue; }
            var path = sm.GetPathName();
            if (!meshIdx.TryGetValue(path, out var mi)) { mi = meshIdx[path] = meshList.Count; meshList.Add(BuildMesh(sm, e, provider, texList, texIdx, matList, matIdx)); }
            if (meshList[mi].Count == 0) { skipped++; continue; }
            nodes.Add(Node(w)); nodeMesh.Add(mi); nodeName.Add(sm.Name);
        }
        Console.WriteLine($"meshes {meshList.Count} nodes {nodes.Count} materials {matList.Count} textures {texList.Count} skipped {skipped}");
        WriteGlb(Path.Combine(outDir, "scene.glb"), meshList, nodes, nodeMesh, nodeName, matList, texList);
        File.WriteAllText(Path.Combine(outDir, "scene.json"), JsonSerializer.Serialize(new { umap, player_start = playerStart }, new JsonSerializerOptions { WriteIndented = true }));
    }

    static List<Prim> BuildMesh(UStaticMesh sm, UObject comp, DefaultFileProvider provider, List<Tex> texList, Dictionary<string, int> texIdx, List<Mat> matList, Dictionary<string, int> matIdx)
    {
        var res = new List<Prim>();
        if (!MeshConverter.TryConvert(sm, out var cm, EMeshQuality.Highest, ENaniteMeshFormat.NoNanite, null) || cm.LODs.Count == 0) return res;
        var lod = cm.LODs[0];
        var over = comp.GetOrDefault<FPackageIndex[]?>("OverrideMaterials", null);
        foreach (var sec in lod.Sections)
        {
            var mref = over != null && sec.MaterialIndex < over.Length && !over[sec.MaterialIndex].IsNull ? over[sec.MaterialIndex].Load<UMaterialInterface>() : (sec.MaterialIndex < cm.Materials.Length ? cm.Materials[sec.MaterialIndex].Material?.Load<UMaterialInterface>() : null);
            int mat = GetMat(mref, texList, texIdx, matList, matIdx);
            // re-index vertices of this section
            var map = new Dictionary<uint, uint>(); var pos = new List<float>(); var nrm = new List<float>(); var uv = new List<float>(); var idx = new List<uint>();
            for (int f = 0; f < sec.NumFaces; f++)
            {
                uint[] tri = [lod.Indices[sec.FirstIndex + f * 3], lod.Indices[sec.FirstIndex + f * 3 + 1], lod.Indices[sec.FirstIndex + f * 3 + 2]];
                // No winding swap: UE index order already gives CCW (glTF front) after the (y, z, -x) conversion. The swap
                // made 99% of triangles disagree with their normals, so culling showed object insides (tools/fix_winding.py).
                foreach (var vi in tri)
                {
                    if (!map.TryGetValue(vi, out var ni))
                    {
                        ni = map[vi] = (uint)(pos.Count / 3); var v = lod.Vertices[(int)vi];
                        pos.AddRange([v.Position.Y * .01f, v.Position.Z * .01f, -v.Position.X * .01f]);
                        nrm.AddRange([v.Normal.Y, v.Normal.Z, -v.Normal.X]);
                        uv.AddRange([v.Uv.U, v.Uv.V]);
                    }
                    idx.Add(ni);
                }
            }
            if (idx.Count == 0) continue;
            var p = new Prim { Verts = pos.Count / 3, Indices = idx.Count, Mat = mat, Pos = Bytes(pos), Nrm = Bytes(nrm), Uv = Bytes(uv), Idx = idx.SelectMany(BitConverter.GetBytes).ToArray() };
            p.Min = [pos.Where((_, i) => i % 3 == 0).Min(), pos.Where((_, i) => i % 3 == 1).Min(), pos.Where((_, i) => i % 3 == 2).Min()];
            p.Max = [pos.Where((_, i) => i % 3 == 0).Max(), pos.Where((_, i) => i % 3 == 1).Max(), pos.Where((_, i) => i % 3 == 2).Max()];
            res.Add(p);
        }
        return res;
    }
    static byte[] Bytes(List<float> f) => f.SelectMany(BitConverter.GetBytes).ToArray();

    static readonly string[] Prefer = ["PM_Diffuse", "Diffuse", "BaseColor", "Base Color", "Albedo", "BaseColorMap", "Color", "Texture"];
    static int GetMat(UMaterialInterface? mi, List<Tex> texList, Dictionary<string, int> texIdx, List<Mat> matList, Dictionary<string, int> matIdx)
    {
        var key = mi?.GetPathName() ?? "";
        if (matIdx.TryGetValue(key, out var found)) return found;
        var m = new Mat { Name = mi?.Name ?? "none" };
        if (mi != null)
        {
            try
            {
                var p = new CMaterialParams2(); mi.GetParams(p, EMaterialDepth.TopLayerOnly);
                string? k = Prefer.FirstOrDefault(x => p.Textures.ContainsKey(x)) ?? p.Textures.Keys.FirstOrDefault(x => { var l = x.ToLowerInvariant(); return (l.Contains("diffuse") || l.Contains("basecolor") || l.Contains("albedo") || l.EndsWith("_d") || l.EndsWith("_bc")) && !l.Contains("normal"); });
                if (k != null && p.Textures[k].Load<UTexture2D>() is UTexture2D t)
                {
                    var tk = t.GetPathName();
                    if (!texIdx.TryGetValue(tk, out var ti))
                    {
                        try
                        {
                            var ct = TextureDecoder.Decode(t, CUE4Parse.UE4.Assets.Exports.Texture.ETexturePlatform.DesktopMobile);
                            using var bmp = new SKBitmap(); var h = System.Runtime.InteropServices.GCHandle.Alloc(ct.Data, System.Runtime.InteropServices.GCHandleType.Pinned);
                            var pf = ct.PixelFormat == CUE4Parse.UE4.Assets.Exports.Texture.EPixelFormat.PF_B8G8R8A8 ? SKColorType.Bgra8888 : ct.PixelFormat == CUE4Parse.UE4.Assets.Exports.Texture.EPixelFormat.PF_R8G8B8A8 ? SKColorType.Rgba8888 : throw new Exception("pixfmt " + ct.PixelFormat);
                            bmp.InstallPixels(new SKImageInfo(ct.Width, ct.Height, pf, SKAlphaType.Unpremul), h.AddrOfPinnedObject());
                            SKBitmap use = bmp;
                            if (Math.Max(bmp.Width, bmp.Height) > MaxTex) { var s = (float)MaxTex / Math.Max(bmp.Width, bmp.Height); use = bmp.Resize(new SKImageInfo(Math.Max(1, (int)(bmp.Width * s)), Math.Max(1, (int)(bmp.Height * s))), SKFilterQuality.Medium)!; }
                            using var img = SKImage.FromBitmap(use);
                            using var data = img.Encode(SKEncodedImageFormat.Jpeg, 82);
                            ti = texIdx[tk] = texList.Count; texList.Add(new Tex(t.Name, data.ToArray(), "image/jpeg")); h.Free();
                        }
                        catch (Exception e) { Console.WriteLine("  tex fail " + t.Name + " " + e.Message); ti = texIdx[tk] = -1; }
                    }
                    m.Tex = ti;
                }
                foreach (var ck in new[] { "Color", "Tint", "BaseColor", "Base Color", "Diffuse Color" })
                    if (p.Colors.TryGetValue(ck, out var c) && m.Tex < 0) { m.Color = [c.R, c.G, c.B, 1]; break; }
            }
            catch (Exception e) { Console.WriteLine("  mat fail " + m.Name + " " + e.Message); }
        }
        return matIdx[key] = matList.Count == 0 && false ? 0 : AddMat(matList, m);
    }
    static int AddMat(List<Mat> l, Mat m) { l.Add(m); return l.Count - 1; }

    static void WriteGlb(string path, List<List<Prim>> meshes, List<float[]> nodes, List<int> nodeMesh, List<string> nodeName, List<Mat> mats, List<Tex> texs)
    {
        var bin = new MemoryStream(); var views = new List<object>(); var accs = new List<object>();
        int View(byte[] d, int? target = null)
        {
            while (bin.Length % 4 != 0) bin.WriteByte(0);
            views.Add(target == null ? new { buffer = 0, byteOffset = bin.Length, byteLength = d.Length } : (object)new { buffer = 0, byteOffset = bin.Length, byteLength = d.Length, target });
            bin.Write(d); return views.Count - 1;
        }
        int Acc(byte[] d, int target, int comp, int count, string type, float[]? min = null, float[]? max = null)
        {
            var v = View(d, target);
            accs.Add(min == null ? new { bufferView = v, componentType = comp, count, type } : (object)new { bufferView = v, componentType = comp, count, type, min, max });
            return accs.Count - 1;
        }
        var images = new List<object>(); var textures = new List<object>();
        foreach (var t in texs) { images.Add(new { bufferView = View(t.Data), mimeType = t.Mime, name = t.Name }); textures.Add(new { source = images.Count - 1 }); }
        var gm = mats.Select(m => m.Tex >= 0
            ? (object)new { name = m.Name, pbrMetallicRoughness = new { baseColorTexture = new { index = m.Tex }, metallicFactor = 0.0, roughnessFactor = 0.85 } }
            : new { name = m.Name, pbrMetallicRoughness = new { baseColorFactor = m.Color, metallicFactor = 0.0, roughnessFactor = 0.85 } }).ToList();
        var gmeshes = meshes.Select((ps, i) => new
        {
            name = "m" + i,
            primitives = ps.Select(p => new
            {
                attributes = new Dictionary<string, int> { ["POSITION"] = Acc(p.Pos, 34962, 5126, p.Verts, "VEC3", p.Min, p.Max), ["NORMAL"] = Acc(p.Nrm, 34962, 5126, p.Verts, "VEC3"), ["TEXCOORD_0"] = Acc(p.Uv, 34962, 5126, p.Verts, "VEC2") },
                indices = Acc(p.Idx, 34963, 5125, p.Indices, "SCALAR"), material = p.Mat
            }).ToList()
        }).ToList();
        var gnodes = nodes.Select((m, i) => new { name = nodeName[i], mesh = nodeMesh[i], matrix = m }).ToList();
        var root = new Dictionary<string, object>
        {
            ["asset"] = new { version = "2.0", generator = "ue-export scene" },
            ["scene"] = 0, ["scenes"] = new[] { new { nodes = Enumerable.Range(0, gnodes.Count).ToArray() } },
            ["nodes"] = gnodes, ["meshes"] = gmeshes, ["materials"] = gm, ["accessors"] = accs, ["bufferViews"] = views,
            ["buffers"] = new[] { new { byteLength = bin.Length } },
        };
        if (images.Count > 0) { root["images"] = images; root["textures"] = textures; }
        var json = System.Text.Encoding.UTF8.GetBytes(JsonSerializer.Serialize(root));
        var jpad = (4 - json.Length % 4) % 4; var bpad = (int)((4 - bin.Length % 4) % 4);
        using var f = File.Create(path); using var w = new BinaryWriter(f);
        w.Write(0x46546C67u); w.Write(2u); w.Write((uint)(12 + 8 + json.Length + jpad + 8 + bin.Length + bpad));
        w.Write((uint)(json.Length + jpad)); w.Write(0x4E4F534Au); w.Write(json); for (int i = 0; i < jpad; i++) w.Write((byte)' ');
        w.Write((uint)(bin.Length + bpad)); w.Write(0x004E4942u); w.Write(bin.ToArray()); for (int i = 0; i < bpad; i++) w.Write((byte)0);
        Console.WriteLine($"wrote {path} {f.Length / 1048576} MB");
    }
}
