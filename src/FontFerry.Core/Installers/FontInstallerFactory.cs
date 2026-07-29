namespace FontFerry.Core.Installers;

public sealed class FontInstallerFactory
{
    private readonly WindowsFontInstaller _windows;
    private readonly MacOsFontInstaller _macOs;

    public FontInstallerFactory(WindowsFontInstaller windows, MacOsFontInstaller macOs)
    {
        _windows = windows;
        _macOs = macOs;
    }

    public IFontInstaller GetCurrent()
    {
        if (OperatingSystem.IsWindows())
            return _windows;
        if (OperatingSystem.IsMacOS())
            return _macOs;
        throw new PlatformNotSupportedException(
            "FontFerry currently supports Windows and macOS.");
    }
}

