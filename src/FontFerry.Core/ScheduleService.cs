using System.Diagnostics;
using System.Security;

namespace FontFerry.Core;

public sealed class ScheduleService
{
    private const string WindowsTaskName = "FontFerry Weekly Update";
    private const string MacLabel = "io.github.fontferry.update";

    public async Task ConfigureAsync(
        bool enabled,
        CancellationToken cancellationToken = default)
    {
        if (OperatingSystem.IsWindows())
        {
            await ConfigureWindowsAsync(enabled, cancellationToken);
            return;
        }

        if (OperatingSystem.IsMacOS())
        {
            await ConfigureMacAsync(enabled, cancellationToken);
            return;
        }

        throw new PlatformNotSupportedException();
    }

    public async Task<bool> IsEnabledAsync(CancellationToken cancellationToken = default)
    {
        if (OperatingSystem.IsWindows())
        {
            var result = await RunProcessAsync(
                "schtasks.exe", ["/Query", "/TN", WindowsTaskName],
                cancellationToken, ignoreFailure: true);
            return result.ExitCode == 0;
        }

        if (OperatingSystem.IsMacOS())
        {
            var path = Path.Combine(
                Environment.GetFolderPath(Environment.SpecialFolder.UserProfile),
                "Library", "LaunchAgents", $"{MacLabel}.plist");
            return File.Exists(path);
        }

        return false;
    }

    private static async Task ConfigureWindowsAsync(
        bool enabled,
        CancellationToken cancellationToken)
    {
        var arguments = new List<string>();
        if (!enabled)
        {
            arguments.AddRange(["/Delete", "/TN", WindowsTaskName, "/F"]);
        }
        else
        {
            arguments.AddRange([
                "/Create",
                "/TN", WindowsTaskName,
                "/SC", "WEEKLY",
                "/D", "SUN",
                "/ST", "10:00",
                "/TR", BuildCommandLine(),
                "/F"
            ]);
        }

        var result = await RunProcessAsync("schtasks.exe", arguments, cancellationToken);
        if (result.ExitCode != 0 && !(result.ExitCode == 1 && !enabled))
            throw new InvalidOperationException(result.Error);
    }

    private static async Task ConfigureMacAsync(
        bool enabled,
        CancellationToken cancellationToken)
    {
        var directory = Path.Combine(
            Environment.GetFolderPath(Environment.SpecialFolder.UserProfile),
            "Library", "LaunchAgents");
        Directory.CreateDirectory(directory);
        var path = Path.Combine(directory, $"{MacLabel}.plist");

        if (!enabled)
        {
            var domain = await GetMacDomainAsync(cancellationToken);
            await RunProcessAsync(
                "launchctl", ["bootout", domain, path], cancellationToken,
                ignoreFailure: true);
            if (File.Exists(path))
                File.Delete(path);
            return;
        }

        var (executable, commandArguments) = GetCommandParts();
        var argumentXml = string.Join(
            Environment.NewLine,
            new[] { executable }.Concat(commandArguments)
                .Select(value => $"      <string>{SecurityElement.Escape(value)}</string>"));
        var plist = $"""
                     <?xml version="1.0" encoding="UTF-8"?>
                     <!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
                       "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
                     <plist version="1.0">
                     <dict>
                       <key>Label</key>
                       <string>{MacLabel}</string>
                       <key>ProgramArguments</key>
                       <array>
                     {argumentXml}
                       </array>
                       <key>StartInterval</key>
                       <integer>604800</integer>
                       <key>RunAtLoad</key>
                       <true/>
                     </dict>
                     </plist>
                     """;
        await File.WriteAllTextAsync(path, plist, cancellationToken);
        var launchDomain = await GetMacDomainAsync(cancellationToken);
        await RunProcessAsync(
            "launchctl", ["bootout", launchDomain, path], cancellationToken,
            ignoreFailure: true);
        await RunProcessAsync(
            "launchctl", ["bootstrap", launchDomain, path], cancellationToken,
            ignoreFailure: false);
    }

    private static async Task<string> GetMacDomainAsync(CancellationToken cancellationToken)
    {
        var result = await RunProcessAsync(
            "id", ["-u"], cancellationToken, ignoreFailure: false);
        if (result.ExitCode != 0 || !int.TryParse(result.Output.Trim(), out var uid))
            throw new InvalidOperationException("Cannot determine the macOS user ID.");
        return $"gui/{uid}";
    }

    private static string BuildCommandLine()
    {
        var (executable, arguments) = GetCommandParts();
        return string.Join(' ',
            new[] { Quote(executable) }.Concat(arguments.Select(Quote)));
    }

    private static (string Executable, string[] Arguments) GetCommandParts()
    {
        var executable = Environment.ProcessPath
            ?? throw new InvalidOperationException("Cannot determine the FontFerry executable path.");
        if (Path.GetFileName(executable).Equals("dotnet.exe", StringComparison.OrdinalIgnoreCase) ||
            Path.GetFileName(executable).Equals("dotnet", StringComparison.OrdinalIgnoreCase))
        {
            var assembly = Environment.GetCommandLineArgs()[0];
            return (executable, [assembly, "update-all", "--no-browser"]);
        }

        return (executable, ["update-all", "--no-browser"]);
    }

    private static string Quote(string value) =>
        value.Contains(' ') ? $"\"{value.Replace("\"", "\\\"")}\"" : value;

    private static async Task<ProcessResult> RunProcessAsync(
        string fileName,
        IEnumerable<string> arguments,
        CancellationToken cancellationToken,
        bool ignoreFailure = false)
    {
        using var process = new Process
        {
            StartInfo = new ProcessStartInfo
            {
                FileName = fileName,
                RedirectStandardOutput = true,
                RedirectStandardError = true,
                UseShellExecute = false,
                CreateNoWindow = true
            }
        };
        foreach (var argument in arguments)
            process.StartInfo.ArgumentList.Add(argument);

        process.Start();
        var outputTask = process.StandardOutput.ReadToEndAsync(cancellationToken);
        var errorTask = process.StandardError.ReadToEndAsync(cancellationToken);
        await process.WaitForExitAsync(cancellationToken);
        var result = new ProcessResult(
            process.ExitCode, await outputTask, await errorTask);
        if (!ignoreFailure && result.ExitCode != 0)
            return result;
        return result;
    }

    private sealed record ProcessResult(int ExitCode, string Output, string Error);
}
