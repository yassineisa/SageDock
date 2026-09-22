# SageDock branding

The canonical logo is `src-tauri/icons/icon.svg`: a cream sigma on a burgundy notebook tile with a rose binding. It replaces the previous interlocking-ring mark. The palette is burgundy `#912338`, cream `#FFF7EE`, and rose `#E59BA8`.

The navigation and browser favicon use this SVG. The packaged application and Windows drag image use the generated icons in `src-tauri/icons`. Keep those files in sync when changing the vector source.

To regenerate icons using the installed CLI:

```powershell
npx.cmd tauri icon src-tauri/icons/icon.svg --output test-results/brand-icons
$iconFiles = @('32x32.png', '128x128.png', '128x128@2x.png', 'icon.png', 'icon.ico', 'icon.icns')
foreach ($iconFile in $iconFiles) {
    Copy-Item -LiteralPath (Join-Path 'test-results/brand-icons' $iconFile) -Destination (Join-Path 'src-tauri/icons' $iconFile)
}
```

Ordinary sentence punctuation is used without em dashes. The rule began with app-authored UI copy, including setup explanations and backend status/error messages, and now covers the whole repository: source comments, documentation, and commit messages. Use a comma, a colon, or a full stop, and prefer rewriting over a comma splice. Third-party diagnostic output is quoted as it arrives and is not rewritten.

These source assets take effect in newly built applications. An existing installed executable or installer must be rebuilt to contain the new icon; Windows may also cache shortcut icons.
