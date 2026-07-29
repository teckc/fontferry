namespace FontFerry.Core;

public sealed class AppPaths
{
    public AppPaths(string? dataRoot = null)
    {
        DataRoot = dataRoot ?? GetDefaultDataRoot();
        CacheRoot = Path.Combine(DataRoot, "cache");
        BackupRoot = Path.Combine(DataRoot, "backups");
        UserCatalogRoot = Path.Combine(DataRoot, "catalog");
        LogRoot = Path.Combine(DataRoot, "logs");
        StateFile = Path.Combine(DataRoot, "state.json");

        Directory.CreateDirectory(DataRoot);
        Directory.CreateDirectory(CacheRoot);
        Directory.CreateDirectory(BackupRoot);
        Directory.CreateDirectory(UserCatalogRoot);
        Directory.CreateDirectory(LogRoot);
    }

    public string DataRoot { get; }
    public string CacheRoot { get; }
    public string BackupRoot { get; }
    public string UserCatalogRoot { get; }
    public string LogRoot { get; }
    public string StateFile { get; }

    private static string GetDefaultDataRoot()
    {
        if (OperatingSystem.IsMacOS())
        {
            return Path.Combine(
                Environment.GetFolderPath(Environment.SpecialFolder.UserProfile),
                "Library", "Application Support", "FontFerry");
        }

        return Path.Combine(
            Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),
            "FontFerry");
    }
}

