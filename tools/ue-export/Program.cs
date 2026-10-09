using System.Text.Json;
using CUE4Parse.FileProvider;
using CUE4Parse.MappingsProvider;
using CUE4Parse.MappingsProvider.Usmap;
using CUE4Parse.UE4.Versions;
using CUE4Parse.UE4.Assets.Exports;
using CUE4Parse.UE4.Assets.Exports.SkeletalMesh;
using CUE4Parse.UE4.Assets.Exports.Animation;
using CUE4Parse.UE4.Assets.Exports.Texture;
using CUE4Parse.UE4.Assets.Exports.Material;
using CUE4Parse.UE4.Objects.UObject;
using CUE4Parse_Conversion;
using CUE4Parse_Conversion.Options;
using CUE4Parse.UE4.Assets.Exports.Texture;

// usage: ue-export <extractedBodycamDir> <usmap> <outDir> <cmd> [args]
//   find <substr>              list asset keys
//   mesh <asset> [name]        SKM -> <outDir>/<name>/<name>.psk + <name>.json (bones, sockets, materials) + textures
//   anim <asset>...            AnimSequence -> <outDir>/anims/<Name>.psa + <Name>.json (duration)
//   dump <asset>               print all export properties as JSON (debug)
var root = args[0]; var usmap = args[1]; var outDir = args[2]; var cmd = args[3];
Serilog.Log.Logger = new Serilog.LoggerConfiguration().MinimumLevel.Warning().WriteTo.Sink(new ConSink()).CreateLogger();
if (root.EndsWith("Paks", StringComparison.OrdinalIgnoreCase)) CUE4Parse.Compression.OodleHelper.Initialize();
var provider = new DefaultFileProvider(root, SearchOption.AllDirectories, new VersionContainer(EGame.GAME_UE5_5), StringComparer.OrdinalIgnoreCase);
provider.MappingsContainer = new FileUsmapTypeMappingsProvider(usmap);
provider.Initialize();
provider.SubmitKey(new CUE4Parse.UE4.Objects.Core.Misc.FGuid(), new CUE4Parse.Encryption.Aes.FAesKey(new byte[32]));
provider.LoadVirtualPaths();
var opts = new ExportOptions(EMeshFormat.ActorX, ENaniteMeshFormat.NoNanite, EMeshQuality.Highest, ETexturePlatform.DesktopMobile, ETextureFormat.Png, 100, false, false, EMaterialDepth.TopLayerOnly, false, false, ESocketFormat.None, default);
Directory.CreateDirectory(outDir);
var raw = Path.Combine(outDir, "_raw");
List<string> Run(UObject o) { var ses = new ExportSession((a, c) => { }); ses.Add(o); var r = ses.RunAsync(raw, opts, null!, default).GetAwaiter().GetResult(); foreach (var x in r) if (!x.Success) Console.WriteLine("export fail " + x.ObjectPath + " " + x.GetType().GetProperties().Select(p => p.GetValue(x)).OfType<Exception>().FirstOrDefault()); return r.Where(x => x.Success).SelectMany(x => x.DiskFilePaths).ToList(); }
var jopt = new JsonSerializerOptions { WriteIndented = true };

Console.Error.WriteLine("files: " + provider.Files.Count);
switch (cmd)
{
    case "find":
        foreach (var k in provider.Files.Keys.Where(k => k.Contains(args[4], StringComparison.OrdinalIgnoreCase) && k.EndsWith(".uasset")).Take(5000)) Console.WriteLine(k);
        break;
    case "dump": // full JSON of every export (properties resolved)
        foreach (var e in provider.LoadPackage(args[4]).GetExports()) Console.WriteLine(Newtonsoft.Json.JsonConvert.SerializeObject(e, Newtonsoft.Json.Formatting.Indented));
        break;
    case "animinfo": { var a = provider.LoadPackageObject<UAnimSequence>(args[4]); foreach (var p in a.Properties) Console.WriteLine("  prop " + p.Name + " = " + p.Tag?.GenericValue); Console.WriteLine($"len={a.SequenceLength} frames={a.NumFrames} cd={a.CompressedDataStructure?.GetType().Name}"); break; }
    case "scan": // scan <pathSubstring> -> every SkeletalMesh under it with its skeleton name
        foreach (var k in provider.Files.Keys.Where(k => k.Contains(args[4], StringComparison.OrdinalIgnoreCase) && k.EndsWith(".uasset") && !k.Contains("/Textures/") && !k.Contains("/Materials/")))
        {
            try
            {
                var pkg = provider.LoadPackage(k.Replace(".uasset", ""));
                if (pkg is CUE4Parse.UE4.Assets.Package pp && pp.ExportMap.Any(e => e.ClassName == "SkeletalMesh"))
                { var m = pkg.GetExports().OfType<USkeletalMesh>().FirstOrDefault(); Console.WriteLine($"{k.Replace(".uasset", "")}	{m?.Skeleton?.Name}	{m?.ReferenceSkeleton.FinalRefBoneInfo.Length}"); }
            }
            catch (Exception ex) { Console.Error.WriteLine("skip " + k + " " + ex.Message); }
        }
        break;
    case "animskel": // animskel <asset>... -> asset, skeleton, duration
        foreach (var a in args.Skip(4)) { try { var q = provider.LoadPackageObject<UAnimSequence>(a); Console.WriteLine($"{a}	{q.Skeleton?.Name}	{q.SequenceLength}"); } catch (Exception ex) { Console.WriteLine($"{a}	ERR {ex.Message}"); } }
        break;
    case "all": ExportAll(args.Length > 4 ? args[4].Split(',') : null); break;
    case "props": // props <asset> -> every export's property list
        foreach (var e in provider.LoadPackage(args[4]).GetExports()) { Console.WriteLine("## " + e.Name + " : " + e.ExportType); foreach (var q in e.Properties) Console.WriteLine("  " + q.Name + " = " + q.Tag?.GenericValue); }
        break;
    case "vbones": // vbones <skeletonAsset>
        { var sk = provider.LoadPackageObject<USkeleton>(args[4]); foreach (var v in sk.Get<CUE4Parse.UE4.Assets.Objects.FStructFallback[]>("VirtualBones")) Console.WriteLine(string.Join(" ", v.Properties.Select(q => q.Name + "=" + q.Tag?.GenericValue))); break; }
    case "refpose": // refpose <skm> -> bone ref pose (cm, quaternion)
        { var m = provider.LoadPackageObject<USkeletalMesh>(args[4]); for (int i = 0; i < m.ReferenceSkeleton.FinalRefBoneInfo.Length; i++) { var t = m.ReferenceSkeleton.FinalRefBonePose[i]; Console.WriteLine($"{m.ReferenceSkeleton.FinalRefBoneInfo[i].Name} parent={m.ReferenceSkeleton.FinalRefBoneInfo[i].ParentIndex} T=({t.Translation.X:F2},{t.Translation.Y:F2},{t.Translation.Z:F2}) Q=({t.Rotation.X:F3},{t.Rotation.Y:F3},{t.Rotation.Z:F3},{t.Rotation.W:F3})"); } break; }
    case "mesh": ExportMesh(args[4], args.Length > 5 ? args[5] : Path.GetFileNameWithoutExtension(args[4])); break;
    case "anim": foreach (var a in args.Skip(4)) ExportAnim(a); break;
}

void ExportMesh(string asset, string name, string sub = "")
{
    var m = provider.LoadPackageObject<USkeletalMesh>(asset);
    var dir = Path.Combine(outDir, sub, name); Directory.CreateDirectory(dir);
    foreach (var f in Run(m)) { File.Copy(f, Path.Combine(dir, Path.GetFileName(f)), true); Console.WriteLine("wrote " + f); }
    var skel = m.Skeleton?.Load<USkeleton>();
    var bones = m.ReferenceSkeleton.FinalRefBoneInfo.Select((b, i) => new { name = b.Name.Text, parent = b.ParentIndex }).ToList();
    var sockets = m.Sockets.Select(s => s.Load<USkeletalMeshSocket>()).Where(s => s != null)
        .Concat(skel?.Sockets.Select(s => s.Load<USkeletalMeshSocket>()).Where(s => s != null) ?? [])
        .Select(s => new { name = s!.SocketName.Text, bone = s.BoneName.Text, loc = new[] { s.RelativeLocation.X, s.RelativeLocation.Y, s.RelativeLocation.Z }, rot = new[] { s.RelativeRotation.Pitch, s.RelativeRotation.Yaw, s.RelativeRotation.Roll }, scale = new[] { s.RelativeScale.X, s.RelativeScale.Y, s.RelativeScale.Z } }).ToList();
    var mats = new List<object>();
    foreach (var sm in m.SkeletalMaterials)
    {
        var mi = sm?.MaterialInterface?.Load<UMaterialInterface>();
        if (mi == null) { mats.Add(new { slot = sm?.MaterialSlotName.Text }); continue; }
        var p = new CMaterialParams2(); mi.GetParams(p, EMaterialDepth.TopLayerOnly);
        var tex = new Dictionary<string, string>();
        foreach (var (k, v) in p.Textures)
        {
            if (v.Load<UTexture>() is UTexture2D t) { SaveTexture(t, dir); tex[k] = t.Name + ".png"; }
        }
        mats.Add(new { slot = sm!.MaterialSlotName.Text, material = mi.Name, textures = tex, scalars = p.Scalars, colors = p.Colors.ToDictionary(kv => kv.Key, kv => new[] { kv.Value.R, kv.Value.G, kv.Value.B, kv.Value.A }) });
    }
    File.WriteAllText(Path.Combine(dir, name + ".json"), JsonSerializer.Serialize(new { asset, skeleton = skel?.Name, bones, sockets, materials = mats }, jopt));
}

void SaveTexture(UTexture2D t, string dir)
{
    var path = Path.Combine(dir, t.Name + ".png");
    if (File.Exists(path)) return;
    var f = Run(t).FirstOrDefault(x => x.EndsWith(".png"));
    if (f == null) Console.WriteLine("tex export failed " + t.Name); else File.Copy(f, path, true);
}

void ExportAnim(string asset, string sub = "anims")
{
    var a = provider.LoadPackageObject<UAnimSequence>(asset);
    var dir = Path.Combine(outDir, sub); Directory.CreateDirectory(dir);
    var saved = string.Join(",", Run(a).Select(f => { var d = Path.Combine(dir, Path.GetFileName(f)); File.Copy(f, d, true); return d; }));
    File.WriteAllText(Path.Combine(dir, a.Name + ".json"), JsonSerializer.Serialize(new { asset, duration = a.SequenceLength, frames = a.NumFrames, skeleton = a.Skeleton?.Name, additive = a.AdditiveAnimType.ToString(), rateScale = a.RateScale }, jopt));
    Console.WriteLine("wrote " + saved);
}

// Discover every gun (anim folder with Character/A_FP_* and Weapon/A_W_*), match its mesh by skeleton name, export all.
// Gun naming/overrides in guns.json next to the exe sources: { "<AnimDir>": { "name": "...", "skm": "<asset>", "extraDirs": ["..."] } }
void ExportAll(string[]? only)
{
    const string AnimRoot = "Bodycam/Plugins/BodycamAnimationFramework/Content/Animations/";
    var cfg = JsonSerializer.Deserialize<Dictionary<string, GunCfg>>(File.ReadAllText(Path.Combine(AppContext.BaseDirectory, "guns.json")))!;
    var keys = provider.Files.Keys.Where(k => k.StartsWith(AnimRoot, StringComparison.OrdinalIgnoreCase) && k.EndsWith(".uasset") && !k.Contains("_Montage")).ToList();
    var skms = ScanSkms();
    // arms + shared
    if (only == null || only.Contains("arms"))
    {
        ExportMesh("Bodycam/Content/BodycamSkin/Arms/Basic/SKM_HumanArmsLongSleeves_Morphers", "arms");
        foreach (var k in keys.Where(k => k.Contains("/AdditiveMotions/") || k.Contains("/Gopro/Adjust/") || k.Contains("Additive")).Where(IsSeq)) ExportAnim(Strip(k), "common");
    }
    foreach (var dirName in keys.Select(k => k[AnimRoot.Length..].Split('/')[0]).Distinct())
    {
        var g = cfg.GetValueOrDefault(dirName) ?? new GunCfg();
        var name = g.name ?? dirName;
        if (g.skip == true || (only != null && !only.Contains(name, StringComparer.OrdinalIgnoreCase))) continue;
        var dirs = new[] { dirName }.Concat(g.extraDirs ?? []).ToList();
        string Under(string d, string sub) => AnimRoot + d + "/" + sub + "/";
        var fp = keys.Where(k => dirs.Any(d => k.StartsWith(Under(d, "Character"), StringComparison.OrdinalIgnoreCase))).Where(IsSeq).ToList();
        var wp = keys.Where(k => dirs.Any(d => k.StartsWith(Under(d, "Weapon"), StringComparison.OrdinalIgnoreCase))).Where(IsSeq).ToList();
        if (fp.Count == 0 || wp.Count == 0) { Console.WriteLine($"skip {dirName}: fp={fp.Count} weapon={wp.Count}"); continue; }
        var skel = provider.LoadPackageObject<UAnimSequence>(Strip(wp[0])).Skeleton?.Name;
        var skm = g.skm ?? skms.FirstOrDefault(m => m.skel == skel).path;
        if (skm == null) { Console.WriteLine($"NO MESH for {dirName} (skeleton {skel}); add to guns.json"); continue; }
        Console.WriteLine($"== {name}: mesh {skm}, {fp.Count} FP + {wp.Count} weapon clips");
        ExportMesh(skm, name, "guns");
        foreach (var k in wp) ExportAnim(Strip(k), $"guns/{name}/anims");
        foreach (var k in fp) ExportAnim(Strip(k), $"fp/{name}");
    }
}
string Strip(string k) => k[..^".uasset".Length];
bool IsSeq(string k) { try { return provider.LoadPackage(Strip(k)) is CUE4Parse.UE4.Assets.Package p && p.ExportMap.Any(e => e.ClassName == "AnimSequence"); } catch { return false; } }
List<(string path, string? skel)> ScanSkms()
{
    var cache = Path.Combine(outDir, "_skms.tsv");
    if (File.Exists(cache)) return File.ReadAllLines(cache).Select(l => l.Split('	')).Select(a => (a[0], (string?)a[1])).ToList();
    var res = new List<(string, string?)>();
    foreach (var k in provider.Files.Keys.Where(k => k.StartsWith("Bodycam/Content/BodycamWeapons/Guns/") && k.EndsWith(".uasset") && !k.Contains("/Textures/") && !k.Contains("/Materials/")))
    {
        try { var pk = provider.LoadPackage(Strip(k)); if (pk is CUE4Parse.UE4.Assets.Package p && p.ExportMap.Any(e => e.ClassName == "SkeletalMesh")) res.Add((Strip(k), pk.GetExports().OfType<USkeletalMesh>().FirstOrDefault()?.Skeleton?.Name)); } catch { }
    }
    File.WriteAllLines(cache, res.Select(r => r.Item1 + "	" + r.Item2));
    return res;
}
class GunCfg { public string? name { get; set; } public string? skm { get; set; } public string[]? extraDirs { get; set; } public bool? skip { get; set; } }

class ConSink : Serilog.Core.ILogEventSink { public void Emit(Serilog.Events.LogEvent e) => Console.Error.WriteLine($"[{e.Level}] {e.RenderMessage()} {e.Exception}"); }
