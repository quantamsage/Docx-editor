<#
.SYNOPSIS
  Regenerates tests/fixtures/word-sample.docx with Microsoft Word.

.DESCRIPTION
  Builds a representative document through Word's COM automation so the
  engine is tested against XML exactly as Word writes it: headings, mixed
  character formatting, bulleted and numbered lists, a table with merged
  cells and borders, an inline picture, a hyperlink, a header, a footer with
  a page-number field, a page break, a comment and a footnote.

  Requires Microsoft Word on Windows. Word runs invisibly and is closed at
  the end.
#>
[CmdletBinding()]
param(
    [string]$OutFile = (Join-Path $PSScriptRoot '..\tests\fixtures\word-sample.docx')
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

$OutFile = [System.IO.Path]::GetFullPath($OutFile)
New-Item -ItemType Directory -Force -Path (Split-Path $OutFile) | Out-Null

# A small, distinctive picture to embed.
$picture = Join-Path ([System.IO.Path]::GetTempPath()) 'docx-editor-fixture.png'
$bitmap = New-Object System.Drawing.Bitmap 120, 60
$graphics = [System.Drawing.Graphics]::FromImage($bitmap)
$graphics.Clear([System.Drawing.Color]::FromArgb(43, 87, 154))
$graphics.FillRectangle([System.Drawing.Brushes]::Gold, 10, 30, 20, 20)
$graphics.FillRectangle([System.Drawing.Brushes]::White, 40, 15, 20, 35)
$graphics.FillRectangle([System.Drawing.Brushes]::LightGreen, 70, 5, 20, 45)
$graphics.Dispose()
$bitmap.Save($picture, [System.Drawing.Imaging.ImageFormat]::Png)
$bitmap.Dispose()

$word = New-Object -ComObject Word.Application
$word.Visible = $false
$word.DisplayAlerts = 0
try {
    $doc = $word.Documents.Add()
    $sel = $word.Selection

    function Add-Paragraph([string]$text, [string]$style = 'Normal') {
        $sel.Style = $doc.Styles.Item($style)
        $sel.TypeText($text)
        $sel.TypeParagraph()
    }

    Add-Paragraph 'Quarterly Report' 'Title'
    Add-Paragraph 'Summary' 'Heading 1'

    $sel.Style = $doc.Styles.Item('Normal')
    $sel.TypeText('Revenue grew ')
    $sel.Font.Bold = 1; $sel.TypeText('twelve percent'); $sel.Font.Bold = 0
    $sel.TypeText(' while costs were ')
    $sel.Font.Italic = 1; $sel.Font.Underline = 1; $sel.TypeText('flat'); $sel.Font.Italic = 0; $sel.Font.Underline = 0
    $sel.TypeText('. ')
    $sel.Font.StrikeThrough = 1; $sel.TypeText('Draft'); $sel.Font.StrikeThrough = 0
    $sel.TypeText(' ')
    $sel.Font.Color = 255; $sel.Font.Name = 'Georgia'; $sel.Font.Size = 14
    $sel.TypeText('Red Georgia 14pt')
    $sel.Font.Color = -16777216; $sel.Font.Name = 'Calibri'; $sel.Font.Size = 11
    $sel.TypeText(' and ')
    $start = $sel.Start
    $sel.TypeText('highlighted')
    $doc.Range($start, $sel.Start).HighlightColorIndex = 7
    $sel.TypeText(' text.')
    $sel.TypeParagraph()

    $sel.ParagraphFormat.Alignment = 1
    $sel.TypeText('This paragraph is centred.')
    $sel.TypeParagraph()
    $sel.ParagraphFormat.Alignment = 3
    $sel.ParagraphFormat.LeftIndent = 36
    $sel.ParagraphFormat.SpaceBefore = 6
    $sel.ParagraphFormat.SpaceAfter = 12
    $sel.TypeText('This paragraph is justified and indented, with extra spacing before and after it so that the layout properties are exercised.')
    $sel.TypeParagraph()
    $sel.ParagraphFormat.Reset()

    Add-Paragraph 'Lists' 'Heading 2'
    $sel.Range.ListFormat.ApplyBulletDefault()
    $sel.TypeText('First bullet'); $sel.TypeParagraph()
    $sel.TypeText('Second bullet'); $sel.TypeParagraph()
    $sel.Range.ListFormat.RemoveNumbers()
    $sel.Range.ListFormat.ApplyNumberDefault()
    $sel.TypeText('Step one'); $sel.TypeParagraph()
    $sel.TypeText('Step two'); $sel.TypeParagraph()
    $sel.Range.ListFormat.RemoveNumbers()

    Add-Paragraph 'Data' 'Heading 2'
    $table = $doc.Tables.Add($sel.Range, 3, 3)
    $table.Borders.Enable = 1
    $table.Cell(1, 1).Range.Text = 'Region'
    $table.Cell(1, 2).Range.Text = 'Q1'
    $table.Cell(1, 3).Range.Text = 'Q2'
    $table.Cell(2, 1).Range.Text = 'North'
    $table.Cell(2, 2).Range.Text = '10'
    $table.Cell(2, 3).Range.Text = '12'
    $table.Cell(3, 1).Range.Text = 'Total across all regions'
    $table.Cell(3, 1).Merge($table.Cell(3, 2))
    $table.Cell(3, 2).Range.Text = '22'
    $table.Rows.Item(1).Range.Font.Bold = 1
    $table.Cell(2, 2).Range.ParagraphFormat.Alignment = 2
    $sel.EndKey(6) | Out-Null

    Add-Paragraph 'Chart' 'Heading 2'
    $shape = $sel.InlineShapes.AddPicture($picture)
    $shape.AlternativeText = 'Quarterly chart'
    $sel.TypeParagraph()

    $sel.TypeText('More information: ')
    $doc.Hyperlinks.Add($sel.Range, 'https://example.com/report', '', '', 'example.com report') | Out-Null
    $sel.TypeText('.')
    $sel.Footnotes.Add($sel.Range, [Type]::Missing, 'Source: internal figures.') | Out-Null
    $sel.TypeParagraph()

    $sel.TypeText('Reviewed paragraph.')
    $doc.Comments.Add($doc.Paragraphs.Last.Range, 'Please double-check these numbers.') | Out-Null
    $sel.EndKey(6) | Out-Null
    $sel.TypeParagraph()

    $sel.InsertBreak(7)
    Add-Paragraph 'Appendix' 'Heading 1'
    $sel.TypeText('Final page content.')

    $section = $doc.Sections.Item(1)
    $section.Headers.Item(1).Range.Text = 'Company Confidential'
    $section.Footers.Item(1).PageNumbers.Add(1, $true) | Out-Null

    $doc.SaveAs2([ref]$OutFile, [ref]16)
    $doc.Close([ref]0)
    $userName = $word.UserName
    $userInitials = $word.UserInitials
}
finally {
    $word.Quit([ref]0)
    [System.Runtime.InteropServices.Marshal]::ReleaseComObject($word) | Out-Null
    Remove-Item $picture -ErrorAction SilentlyContinue
}

# Word stamps the Office user's name and initials into the document
# properties, comments and the people list. Replace them with neutral values
# so the committed fixture carries no personal information.
Add-Type -AssemblyName System.IO.Compression
$escape = { param($s) [System.Security.SecurityElement]::Escape($s) }
$replacements = @(
    @{ From = "<dc:creator>$(& $escape $userName)</dc:creator>"; To = '<dc:creator>Fixture Author</dc:creator>' },
    @{ From = "<cp:lastModifiedBy>$(& $escape $userName)</cp:lastModifiedBy>"; To = '<cp:lastModifiedBy>Fixture Author</cp:lastModifiedBy>' },
    @{ From = "=`"$(& $escape $userName)`""; To = '="Fixture Author"' },
    @{ From = "w:initials=`"$(& $escape $userInitials)`""; To = 'w:initials="FA"' }
)
$archive = [System.IO.Compression.ZipFile]::Open($OutFile, [System.IO.Compression.ZipArchiveMode]::Update)
try {
    foreach ($name in 'docProps/core.xml', 'word/comments.xml', 'word/people.xml', 'word/commentsExtended.xml') {
        $entry = $archive.GetEntry($name)
        if (-not $entry) { continue }
        $reader = New-Object System.IO.StreamReader($entry.Open())
        $xml = $reader.ReadToEnd()
        $reader.Dispose()
        foreach ($r in $replacements) { $xml = $xml.Replace($r.From, $r.To) }
        $entry.Delete()
        $writer = New-Object System.IO.StreamWriter($archive.CreateEntry($name).Open(), (New-Object System.Text.UTF8Encoding($false)))
        $writer.Write($xml)
        $writer.Dispose()
    }
}
finally {
    $archive.Dispose()
}
Write-Output "Wrote $OutFile"
