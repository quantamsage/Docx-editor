<#
.SYNOPSIS
  Opens .docx files in Microsoft Word and reports what Word sees.

.DESCRIPTION
  Used to check that documents written by the engine are accepted by Word
  (no corruption or repair prompt) and that their content survived: the
  paragraph texts and the counts of tables, pictures, hyperlinks, comments,
  footnotes, headers and footers.

  Word runs invisibly with alerts disabled and opens each file read-only,
  without repair, so a damaged file fails instead of being silently fixed.
  Outputs one JSON object per file.

.EXAMPLE
  ./scripts/verify-with-word.ps1 tests/fixtures/word-sample.docx target/edited.docx
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory, ValueFromRemainingArguments)]
    [string[]]$Path
)
$ErrorActionPreference = 'Stop'

$word = New-Object -ComObject Word.Application
$word.Visible = $false
$word.DisplayAlerts = 0
try {
    foreach ($file in $Path) {
        $full = [System.IO.Path]::GetFullPath($file)
        $result = [ordered]@{ file = $file; opened = $false }
        try {
            # Open(FileName, ConfirmConversions, ReadOnly, AddToRecentFiles). The remaining
            # arguments keep Word's defaults, which include OpenAndRepair = false.
            $doc = $word.Documents.Open($full, $false, $true, $false)
            $result.opened = $true
            $section = $doc.Sections.Item(1)
            $result.paragraphs = $doc.Paragraphs.Count
            $result.tables = $doc.Tables.Count
            $result.pictures = $doc.InlineShapes.Count
            $result.hyperlinks = @($doc.Hyperlinks | ForEach-Object { $_.Address })
            $result.comments = $doc.Comments.Count
            $result.footnotes = $doc.Footnotes.Count
            $result.header = $section.Headers.Item(1).Range.Text.Trim()
            $result.footerFields = $section.Footers.Item(1).Range.Fields.Count
            $result.pageWidthPt = $section.PageSetup.PageWidth
            $result.texts = @($doc.Paragraphs | ForEach-Object {
                $p = $_
                [ordered]@{ text = $p.Range.Text.Trim([char]13, [char]7, ' '); style = $p.Style.NameLocal; align = $p.Alignment }
            })
            $doc.Close([ref]0)
        }
        catch {
            $result.error = $_.Exception.Message
        }
        [pscustomobject]$result | ConvertTo-Json -Depth 5 -Compress
    }
}
finally {
    $word.Quit([ref]0)
    [System.Runtime.InteropServices.Marshal]::ReleaseComObject($word) | Out-Null
}
