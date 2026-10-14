using System.Text.Json;
using System.Text.RegularExpressions;
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
using CUE4Parse.UE4.Assets.Objects;
using SkiaSharp;

// scene <umap>: static-mesh placements of a level and its always-loaded sublevels -> <out>/scene.glb (+ scene.json: player start).
// UE (cm, X fwd / Y right / Z up, left-handed) -> glTF/Bevy (m, Y up, -Z fwd): p' = (y, z, -x) * 0.01.
static class SceneExport
{
    // Texture size cap (px): SCENE_MAXTEX overrides. 1024 keeps the whole map ~1.5 GB of GPU memory; 512 was the original cap.
    static readonly int MaxTex = int.TryParse(Environment.GetEnvironmentVariable("SCENE_MAXTEX"), out var mt) ? mt : 1024;
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

    // ---- lights ----
    static readonly HashSet<string> LightTypes = ["PointLightComponent", "SpotLightComponent", "RectLightComponent", "DirectionalLightComponent", "SkyLightComponent"];
    static string? Str(UObject c, string n)
    {
        for (UObject? o = c; o != null; o = o.Template?.Object?.Value)
        { var p = o.Properties.FirstOrDefault(q => q.Name.Text == n); if (p != null) return p.Tag?.GenericValue?.ToString(); }
        return null;
    }
    static object? LightRec(UObject c, double[] w, string owner, string level)
    {
        var t = c.ExportType.Replace("LightComponent", "").ToLowerInvariant(); // point spot rect directional sky
        if (!c.GetOrDefault("bVisible", true) || !c.GetOrDefault("bAffectsWorld", true)) return null;
        var col = c.GetOrDefault("LightColor", new FColor(255, 255, 255, 255));
        var units = Str(c, "IntensityUnits") ?? "Unitless";
        if (!c.GetOrDefault("bAutoActivate", true) || owner.StartsWith("WEP_C")) return null; // weapon-prop lights (muzzle flash), off by default
        double dI = t switch { "directional" => 10, "sky" => 1, _ => units.Contains("Candelas") ? 8 : 5000 }; // UE 5.5: Candelas default is 8 cd, Unitless 5000
        double R(string n, double d) => c.GetOrDefault(n, (float)d);
        var fwd = new[] { w[0], w[4], w[8] }; double fl = Math.Sqrt(fwd[0] * fwd[0] + fwd[1] * fwd[1] + fwd[2] * fwd[2]); fwd = fwd.Select(v => v / fl).ToArray();
        return new
        {
            kind = t, owner, level,
            loc_cm = new[] { w[3], w[7], w[11] }, dir_ue = fwd, // dir = light's +X axis (UE space)
            intensity = R("Intensity", dI), units,
            color = new[] { (int)col.R, (int)col.G, (int)col.B },
            atten_cm = R("AttenuationRadius", 1000),
            src_radius = R("SourceRadius", 0), src_length = R("SourceLength", 0),
            rect_w = R("SourceWidth", 64), rect_h = R("SourceHeight", 64),
            inner_deg = R("InnerConeAngle", 0), outer_deg = R("OuterConeAngle", 44),
            temp = c.GetOrDefault("bUseTemperature", false) ? R("Temperature", 6500) : 0,
            shadows = c.GetOrDefault("CastShadows", true) && c.GetOrDefault("CastDynamicShadows", true),
            mobility = Str(c, "Mobility") ?? "",
            inv_sq = c.GetOrDefault("bUseInverseSquaredFalloff", true), falloff = R("LightFalloffExponent", 8),
            name = c.Name,
        };
    }
    static string? Str2(FStructFallback st, string n) => st.Properties.FirstOrDefault(q => q.Name.Text == n)?.Tag?.GenericValue?.ToString();
    static object? PostRec(UObject e, string lvl)
    {
        var st = e.GetOrDefault<FStructFallback?>("Settings", null); if (st == null) return null;
        double R(string n, double d) => st.GetOrDefault(n, (float)d);
        return new
        {
            name = e.Name, unbound = e.GetOrDefault("bUnbound", true), priority = e.GetOrDefault("Priority", 0f),
            ae_min = R("AutoExposureMinBrightness", 0), ae_max = R("AutoExposureMaxBrightness", 0), ae_bias = R("AutoExposureBias", 0),
            ae_method = Str2(st, "AutoExposureMethod"), ae_speed_up = R("AutoExposureSpeedUp", 3), ae_speed_down = R("AutoExposureSpeedDown", 1),
            bloom = R("BloomIntensity", 0.675), level = lvl,
        };
    }

    record Tex(string Name, byte[] Data, string Mime);
    // Emis = emissive colour (linear, UE emissive units), EmisTex = emissive texture, Surface = EPhysicalSurface index of the PhysMat (0 = default / none).
    class Mat { public string Name = ""; public int Tex = -1; public float[] Color = [0.6f, 0.6f, 0.6f, 1]; public string Alpha = "OPAQUE"; public float[]? Emis; public int EmisTex = -1; public int Surface; }
    static readonly Regex EmisKey = new("emiss", RegexOptions.IgnoreCase);
    // PhysMat -> SurfaceType: MaterialInstance chain (PhysMaterial override or Parent) up to the Material.
    static int PhysSurface(UMaterialInterface? mi)
    {
        UObject? o = mi;
        for (int d = 0; o != null && d < 12; d++)
        {
            if (o.GetOrDefault<FPackageIndex?>("PhysMaterial", null) is { IsNull: false } pm && pm.Load() is UObject phys)
            {
                var s = Str(phys, "SurfaceType") ?? ""; var m = Regex.Match(s, @"SurfaceType(\d+)\s*$");
                return m.Success ? int.Parse(m.Groups[1].Value) : 0;
            }
            o = o.GetOrDefault<FPackageIndex?>("Parent", null) is { IsNull: false } par ? par.Load() : null;
        }
        return 0;
    }
    class Prim { public byte[] Pos = [], Nrm = [], Uv = [], Idx = []; public int Verts, Indices, Mat; public float[] Min = [], Max = []; }

    public static void Run(DefaultFileProvider provider, string outDir, string umap, double radiusCm)
    {
        var texList = new List<Tex>(); var texIdx = new Dictionary<string, int>();
        var matList = new List<Mat>(); var matIdx = new Dictionary<string, int>();
        var meshList = new List<List<Prim>>(); var meshIdx = new Dictionary<string, int>();
        var nodes = new List<float[]>(); var nodeMesh = new List<int>(); var nodeName = new List<string>();
        object? playerStart = null;
        var lights = new List<object>(); var posts = new List<object>(); var lightKeys = new HashSet<string>(); var bpInst = new List<(UObject e, string lv)>();
        bool lightsOnly = Environment.GetEnvironmentVariable("SCENE_LIGHTS_ONLY") == "1"; // write scene.json only, skip meshes/glb
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
                        todo.Enqueue("Bodycam/Content" + wa["/Game".Length..wa.LastIndexOf('.')]);
                    }
                    else if (e.ExportType == "PlayerStart" && playerStart == null)
                    {
                        var rc = e.GetOrDefault<FPackageIndex>("RootComponent").Load()!; var w = World(rc);
                        var rot = rc.GetOrDefault("RelativeRotation", FRotator.ZeroRotator);
                        playerStart = new { map = lv, loc_cm = new[] { w[3], w[7], w[11] }, yaw_deg = rot.Yaw };
                        Console.WriteLine("PlayerStart " + JsonSerializer.Serialize(playerStart));
                    }
                    else if (LightTypes.Contains(e.ExportType))
                    {
                        var own = e.Outer?.Name.Text ?? ""; var r = LightRec(e, World(e), own, lv);
                        if (r != null && lightKeys.Add(e.Name + "@" + own)) lights.Add(r);
                    }
                    else if (e.ExportType == "PostProcessVolume") { var r = PostRec(e, lv); if (r != null) posts.Add(r); }
                    else if (e.ExportType.EndsWith("_C") && e.GetOrDefault<FPackageIndex?>("RootComponent", null) is { IsNull: false }) bpInst.Add((e, lv));
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
        foreach (var (a, lv) in bpInst)
        {
            try
            {
                var rootW = World(a.GetOrDefault<FPackageIndex>("RootComponent").Load()!);
                var seenN = new HashSet<string>();
                for (var cls = a.Class; cls != null; cls = cls.Super)
                {
                    var pkgPath = cls.GetPathName(); pkgPath = pkgPath[..pkgPath.IndexOf('.')];
                    if (pkgPath.StartsWith("/Script")) break;
                    IEnumerable<UObject> bx; try { bx = provider.LoadPackage(pkgPath.TrimStart('/').Replace("Game/", "Bodycam/Content/")).GetExports().ToList(); } catch { break; }
                    foreach (var c in bx.Where(x => LightTypes.Contains(x.ExportType)))
                    {
                        var nm = c.Name.Replace("_GEN_VARIABLE", ""); if (!seenN.Add(nm)) continue;
                        if (lightKeys.Contains(c.Name + "@" + a.Name)) continue; // level already holds this instance's override
                        // relative transform inside the BP: chain up to (excluding) the BP root, which the instance's world matrix replaces
                        var rel = Local(c); var par = c.GetOrDefault<FPackageIndex?>("AttachParent", null); int d = 0;
                        while (d++ < 16 && par != null && !par.IsNull && par.Load() is UObject pc && pc.GetOrDefault<FPackageIndex?>("AttachParent", null) is { IsNull: false }) { rel = Mul(Local(pc), rel); par = pc.GetOrDefault<FPackageIndex?>("AttachParent", null); }
                        var r = LightRec(c, Mul(rootW, rel), a.Name + ":" + cls.Name, lv);
                        if (r != null && lightKeys.Add(c.Name + "@" + a.Name)) lights.Add(r);
                    }
                }
            }
            catch (Exception e2) { Console.WriteLine($"  bp light fail {a.Name}: {e2.Message}"); }
        }
        if (playerStart != null) { var p0 = (double[])((dynamic)playerStart).loc_cm; lights.RemoveAll(l => { var q = (double[])((dynamic)l).loc_cm; return Math.Abs(q[0] - p0[0]) > 40000 || Math.Abs(q[1] - p0[1]) > 40000; }); } // drops the off-map loadout room (x = -6760 m)
        Console.WriteLine("lights " + lights.Count + " " + string.Join(",", lights.GroupBy(l => ((dynamic)l).kind).Select(g => g.Key + "=" + g.Count())) + " post " + posts.Count);
        var jo = new JsonSerializerOptions { WriteIndented = true };
        if (lightsOnly) { File.WriteAllText(Path.Combine(outDir, "scene.json"), JsonSerializer.Serialize(new { umap, player_start = playerStart, lights, post = posts }, jo)); return; }
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
        // Unique glTF material names (Bevy's GltfMaterialName on each mesh entity is the key into scene.json "surfaces": name -> EPhysicalSurface index).
        var surfaces = new SortedDictionary<string, int>(); var used = new HashSet<string>();
        for (int i = 0; i < matList.Count; i++) { var n = matList[i].Name; if (!used.Add(n)) { n += "~" + i; used.Add(n); } matList[i].Name = n; if (matList[i].Surface != 0) surfaces[n] = matList[i].Surface; }
        var sname = Environment.GetEnvironmentVariable("SCENE_NAME") ?? "scene";
        WriteGlb(Path.Combine(outDir, sname + ".glb"), meshList, nodes, nodeMesh, nodeName, matList, texList);
        File.WriteAllText(Path.Combine(outDir, sname + ".json"), JsonSerializer.Serialize(new { umap, player_start = playerStart, lights, post = posts, surfaces }, jo));
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
            if (mat < 0) continue;
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

    // Albedo texture parameter names, best first. CUE4Parse's "PM_Diffuse" guess is only a last resort: it often lands on a
    // tint/ID mask (TX_Trims_01a_M, IDMap...) whose R/G/B channels render as saturated red/green/purple.
    static readonly string[] AlbedoKeys = ["Albedo", "albedo", "Base Color", "BaseColor", "BaseColorTexture", "BaseColour", "Diffuse", "Main_Albedo", "Albedo Map", "ColorMap", "AlbedoT", "BC", "CustomAlbedo",
        "02 - Diffuse Texture", "Albedo_L1", "Albedo_1", "Top_LayerAlbedo", "Top Layer | BaseColorTexture", "Middle Layer | BaseColorTexture", "Tile A", "Texture", "RustAlbedoT", "AlbedoDamageT"];
    // Colour parameters that multiply the albedo (or are the colour when there is no albedo texture). Tint masks (R/G/B/Alpha variants) are ignored.
    static readonly string[] TintKeys = ["Albedo Color Tint (Base)", "Albedo Tint", "Albedo Color Tint", "Main_AlbedoTint", "BaseColor Tint", "BaseColour Tint", "Base Color Tint", "Tint Color", "Tint", "TintColor", "AlbedoColor",
        "BaseColorFactor", "00 - Flat Colour", "Glass Color", "Base Color", "BaseColor", "Color"];
    static readonly StreamWriter? MatDump = Environment.GetEnvironmentVariable("SCENE_MATDUMP") is { Length: > 0 } f ? new StreamWriter(f) { AutoFlush = true } : null; // debug: every material's cooked params as JSON lines
    static bool MaskLike(string path) { var n = path.Split('/').Last().Split('.')[0].ToLowerInvariant(); return n.EndsWith("_m") || n.Contains("mask") || n.EndsWith("_id") || n.Contains("idmap"); }
    // Decode + resize to MaxTex + encode (JPEG, or PNG when the alpha channel really carries cut-outs) once per texture; returns the texList index or -1.
    static int LoadTex(UTexture2D t, bool wantAlpha, List<Tex> texList, Dictionary<string, int> texIdx)
    {
        var ck = t.GetPathName() + (wantAlpha ? "|a" : "");
        if (texIdx.TryGetValue(ck, out var ti)) return ti;
        try
        {
            var ct = TextureDecoder.Decode(t, CUE4Parse.UE4.Assets.Exports.Texture.ETexturePlatform.DesktopMobile);
            using var bmp = new SKBitmap(); var h = System.Runtime.InteropServices.GCHandle.Alloc(ct.Data, System.Runtime.InteropServices.GCHandleType.Pinned);
            var pf = ct.PixelFormat == CUE4Parse.UE4.Assets.Exports.Texture.EPixelFormat.PF_B8G8R8A8 ? SKColorType.Bgra8888 : ct.PixelFormat == CUE4Parse.UE4.Assets.Exports.Texture.EPixelFormat.PF_R8G8B8A8 ? SKColorType.Rgba8888 : throw new Exception("pixfmt " + ct.PixelFormat);
            bmp.InstallPixels(new SKImageInfo(ct.Width, ct.Height, pf, SKAlphaType.Unpremul), h.AddrOfPinnedObject());
            SKBitmap use = bmp;
            if (Math.Max(bmp.Width, bmp.Height) > MaxTex) { var s = (float)MaxTex / Math.Max(bmp.Width, bmp.Height); use = bmp.Resize(new SKImageInfo(Math.Max(1, (int)(bmp.Width * s)), Math.Max(1, (int)(bmp.Height * s))), SKFilterQuality.Medium)!; }
            // keep alpha (PNG) only when the albedo really carries a cut-out / opacity (some are all-0 or all-255 and would vanish)
            bool png = false;
            if (wantAlpha) { int lo = 255, hi = 0; foreach (var px in use.Pixels) { lo = Math.Min(lo, px.Alpha); hi = Math.Max(hi, px.Alpha); } png = lo < 200 && hi > 200; }
            using var img = SKImage.FromBitmap(use);
            using var data = png ? img.Encode(SKEncodedImageFormat.Png, 100) : img.Encode(SKEncodedImageFormat.Jpeg, 85);
            ti = texIdx[ck] = texList.Count; texList.Add(new Tex(t.Name, data.ToArray(), png ? "image/png" : "image/jpeg")); h.Free();
            if (!ReferenceEquals(use, bmp)) use.Dispose();
        }
        catch (Exception e) { Console.WriteLine("  tex fail " + t.Name + " " + e.Message); ti = texIdx[ck] = -1; }
        return ti;
    }
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
                if (MatDump != null) MatDump.WriteLine(JsonSerializer.Serialize(new { m.Name, path = key, blend = p.BlendMode.ToString(), shading = p.ShadingModel.ToString(), tex = p.Textures.ToDictionary(x => x.Key, x => x.Value.ToString()), col = p.Colors.ToDictionary(x => x.Key, x => new[] { x.Value.R, x.Value.G, x.Value.B, x.Value.A }), sc = p.Scalars }));
                bool trans = p.BlendMode is EBlendMode.BLEND_Translucent or EBlendMode.BLEND_Additive or EBlendMode.BLEND_Modulate, masked = p.BlendMode == EBlendMode.BLEND_Masked;
                if (trans && m.Name.Contains("Decal", StringComparison.OrdinalIgnoreCase)) return matIdx[key] = -1; // projected decals were static meshes in the map; opaque they cover the wall
                string? k = AlbedoKeys.FirstOrDefault(x => p.Textures.ContainsKey(x))
                    ?? p.Textures.Keys.FirstOrDefault(x => { var l = x.ToLowerInvariant(); return !x.StartsWith("PM_") && (l.Contains("albedo") || l.Contains("basecolor") || l.Contains("diffuse") || l.EndsWith("_d") || l.EndsWith("_bc") || l.EndsWith("_alb")) && !l.Contains("mask") && !l.Contains("normal") && !l.Contains("dust") && !l.Contains("damage"); })
                    ?? (p.Textures.TryGetValue("PM_Diffuse", out var pm) && !MaskLike(pm.ToString() ?? "") ? "PM_Diffuse" : null);
                if (k != null && p.Textures[k].Load<UTexture2D>() is UTexture2D t)
                {
                    var ti = LoadTex(t, masked || trans, texList, texIdx);
                    if (ti >= 0 && texList[ti].Mime == "image/png") m.Alpha = masked ? "MASK" : "BLEND";
                    m.Tex = ti;
                }
                // Emissive: colour parameter(s) x scalar intensity (light fixtures, screens, signs), optional emissive texture.
                {
                    var ek = p.Colors.Keys.FirstOrDefault(x => EmisKey.IsMatch(x) && !x.Contains("mask", StringComparison.OrdinalIgnoreCase));
                    var es = p.Scalars.Where(x => EmisKey.IsMatch(x.Key) && (x.Key.Contains("intens", StringComparison.OrdinalIgnoreCase) || x.Key.Contains("power", StringComparison.OrdinalIgnoreCase) || x.Key.Contains("mult", StringComparison.OrdinalIgnoreCase) || x.Key.Contains("bright", StringComparison.OrdinalIgnoreCase))).Select(x => x.Value).DefaultIfEmpty(p.Scalars.Where(x => EmisKey.IsMatch(x.Key)).Select(x => x.Value).DefaultIfEmpty(1f).Max()).Max();
                    var ekt = p.Textures.Keys.FirstOrDefault(x => EmisKey.IsMatch(x) && !x.Contains("mask", StringComparison.OrdinalIgnoreCase)) ?? p.Textures.Keys.FirstOrDefault(x => EmisKey.IsMatch(x));
                    if (ekt != null && p.Textures[ekt].Load<UTexture2D>() is UTexture2D et) m.EmisTex = LoadTex(et, false, texList, texIdx);
                    if (ek != null && p.Colors[ek] is { } ec && (ec.R + ec.G + ec.B) * es > 0.0001f) m.Emis = [ec.R * es, ec.G * es, ec.B * es];
                    else if (m.EmisTex >= 0 && p.Scalars.Keys.Any(x => EmisKey.IsMatch(x)) && es > 0.0001f) m.Emis = [es, es, es]; // texture-only needs an explicit strength scalar (PM_Emissive defaults would light everything)
                    if (m.Emis == null) m.EmisTex = -1;
                }
                m.Surface = PhysSurface(mi);
                var tintKey = k == "Tile A" ? "Tile A tint" : TintKeys.FirstOrDefault(x => p.Colors.ContainsKey(x));
                if (tintKey != null && p.Colors.TryGetValue(tintKey, out var c)) m.Color = [c.R, c.G, c.B, 1];
                else if (m.Tex < 0) m.Color = [0.5f, 0.5f, 0.5f, 1];
                else m.Color = [1, 1, 1, 1];
                if (trans) { m.Alpha = "BLEND"; m.Color[3] = p.Scalars.TryGetValue("Opacity", out var op) ? Math.Clamp(op, 0.15f, 1f) : 0.4f; }
            }
            catch (Exception e) { Console.WriteLine("  mat fail " + m.Name + " " + e.Message); }
        }
        return matIdx[key] = AddMat(matList, m);
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
        var gm = mats.Select(m =>
        {
            var pbr = new Dictionary<string, object> { ["baseColorFactor"] = m.Color, ["metallicFactor"] = 0.0, ["roughnessFactor"] = 0.85 };
            if (m.Tex >= 0) pbr["baseColorTexture"] = new { index = m.Tex };
            var g = new Dictionary<string, object> { ["name"] = m.Name, ["pbrMetallicRoughness"] = pbr };
            if (m.Emis != null)
            {
                // glTF emissiveFactor is clamped to 0..1; the overshoot goes into KHR_materials_emissive_strength.
                var mx = Math.Max(1f, m.Emis.Max()); g["emissiveFactor"] = m.Emis.Select(v => v / mx).ToArray();
                if (mx > 1f) g["extensions"] = new Dictionary<string, object> { ["KHR_materials_emissive_strength"] = new { emissiveStrength = mx } };
                if (m.EmisTex >= 0) g["emissiveTexture"] = new { index = m.EmisTex };
            }
            if (m.Alpha != "OPAQUE") { g["alphaMode"] = m.Alpha; if (m.Alpha == "MASK") g["alphaCutoff"] = 0.5; g["doubleSided"] = true; }
            return g;
        }).ToList();
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
            ["extensionsUsed"] = new[] { "KHR_materials_emissive_strength" }, ["nodes"] = gnodes, ["meshes"] = gmeshes, ["materials"] = gm, ["accessors"] = accs, ["bufferViews"] = views,
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
