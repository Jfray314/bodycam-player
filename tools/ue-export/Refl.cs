// dev helper: reflect <TypeName>... prints public members of CUE4Parse types
static class Refl
{
    public static void Run(string[] names)
    {
        foreach (var asm in new[] { typeof(CUE4Parse.UE4.Assets.Exports.UObject).Assembly, typeof(CUE4Parse_Conversion.ExportSession).Assembly })
        {
            Type[] ts; try { ts = asm.GetTypes(); } catch (System.Reflection.ReflectionTypeLoadException e) { ts = e.Types.Where(t => t != null).ToArray()!; }
            foreach (var t in ts.Where(t => names.Contains(t.Name) || names.Any(n => n.StartsWith("+") && t.Name.Contains(n[1..]))))
            {
                Console.WriteLine("== " + t.FullName + " : " + t.BaseType?.Name);
                foreach (var m in t.GetMembers().Where(m => m.MemberType is System.Reflection.MemberTypes.Field or System.Reflection.MemberTypes.Property or System.Reflection.MemberTypes.Method && m.DeclaringType == t)) Console.WriteLine("  " + m);
            }
        }
    }
}
