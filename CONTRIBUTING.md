# Contributing

## Add or update a font

Built-in fonts are declarative JSON files under `catalog/`. Copy the closest
entry, choose a stable lowercase ID, and define either:

- `githubRelease`: an `owner/repository`, release channel, and asset presets;
- `staticUrl`: a direct URL and an explicit version.

Asset patterns are regular expressions matched against release asset names.
Keep each preset narrow enough to match exactly one asset in a release.

Before opening a pull request:

```powershell
dotnet test
dotnet build -c Release
```

Never commit font binaries unless their license explicitly permits
redistribution and the project has agreed to vendor them.
