# Checks a txt output of pi_calc against known 50-digit windows of pi.
# Reference digits come from https://api.pi.delivery (start = digit after the decimal point).
# Usage: .\verify.ps1 pi_100M.txt
param([Parameter(Mandatory = $true)][string]$Path)

$windows = @(
    @(1, "14159265358979323846264338327950288419716939937510"),
    @(7500001, "13783825349005293901145685963732324648648439299939"),
    @(9999951, "65326553194060999469787333810631719481735348955897"),
    @(50000001, "56452116833617131843989639574271220779358831702756"),
    @(99999951, "75722031752074898161168313937514970581120187751592"),
    @(999999951, "71398682093196353628204612755715171395115275045519")
)

$file = [IO.File]::OpenRead((Resolve-Path -LiteralPath $Path))
try {
    # The file is "3." + digits + newline.
    $digits = $file.Length - 3
    Write-Output "$Path ($digits digits)"
    $failed = $false
    foreach ($window in $windows) {
        $start = $window[0]
        if ($start + 49 -gt $digits) { continue }

        $buffer = New-Object byte[] 50
        $null = $file.Seek($start + 1, [IO.SeekOrigin]::Begin)
        $null = $file.Read($buffer, 0, 50)
        $got = [Text.Encoding]::ASCII.GetString($buffer)
        if ($got -eq $window[1]) {
            Write-Output "  ok    digits $start..$($start + 49)"
        } else {
            Write-Output "  WRONG digits $start..$($start + 49): $got"
            $failed = $true
        }
    }
} finally {
    $file.Dispose()
}
if ($failed) { exit 1 }
