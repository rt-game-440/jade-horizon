# 把验收截图的顶部 HUD 区域裁出来，纵向拼成一张 hud-sheet.png，便于一次查看。
# 用法：pwsh tools/contact_sheet.ps1 [验收输出目录]（默认最新的 logs/verify-*）
param([string]$Dir)
Add-Type -AssemblyName System.Drawing
if (-not $Dir) { $Dir = (Get-ChildItem "$PSScriptRoot/../logs" -Directory -Filter "verify-*" | Sort-Object Name | Select-Object -Last 1).FullName }
$files = Get-ChildItem $Dir -Filter *.png | Where-Object Name -ne "hud-sheet.png" | Sort-Object Name
$cropW = 1000; $cropH = 260; $label = 30
$sheet = New-Object System.Drawing.Bitmap $cropW, (($cropH + $label) * $files.Count)
$g = [System.Drawing.Graphics]::FromImage($sheet)
$g.Clear([System.Drawing.Color]::Black)
$font = New-Object System.Drawing.Font "Consolas", 16
$y = 0
foreach ($f in $files) {
    $img = [System.Drawing.Image]::FromFile($f.FullName)
    $src = New-Object System.Drawing.Rectangle ([int](($img.Width - $cropW) / 2)), 0, $cropW, $cropH
    $g.DrawString($f.BaseName, $font, [System.Drawing.Brushes]::Yellow, 4, $y + 2)
    $g.DrawImage($img, (New-Object System.Drawing.Rectangle 0, ($y + $label), $cropW, $cropH), $src, [System.Drawing.GraphicsUnit]::Pixel)
    $img.Dispose()
    $y += $cropH + $label
}
$out = Join-Path $Dir "hud-sheet.png"
$sheet.Save($out, [System.Drawing.Imaging.ImageFormat]::Png)
$g.Dispose(); $sheet.Dispose()
$out
