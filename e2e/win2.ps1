# Reads or changes the state of ONE app window, chosen by process id.
# It never picks "any RushDownload window": the id is required, so a copy of
# the app that someone else has open is never touched.
param(
  [Parameter(Mandatory = $true)][int]$ProcessId,
  [string]$Do = "state"
)
Add-Type @"
using System;
using System.Text;
using System.Runtime.InteropServices;
public class W2 {
  public delegate bool EnumProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr l);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr h);
  [DllImport("user32.dll")] public static extern bool IsZoomed(IntPtr h);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
  [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int w, int ht, uint flags);
  public static IntPtr Find(uint pid, string title) {
    IntPtr found = IntPtr.Zero;
    EnumWindows((h, l) => {
      uint p; GetWindowThreadProcessId(h, out p);
      if (p != pid) return true;
      var sb = new StringBuilder(256); GetWindowText(h, sb, 256);
      if (sb.ToString() == title && IsWindowVisible(h)) { found = h; return false; }
      return true;
    }, IntPtr.Zero);
    return found;
  }
}
"@
$h = [W2]::Find([uint32]$ProcessId, "RushDownload")
if ($h -eq [IntPtr]::Zero) { "window of process $ProcessId not found"; exit 1 }
# hide = park off-screen so the test window is not in anyone's way
if ($Do -eq "hide") { [W2]::SetWindowPos($h, [IntPtr]::Zero, -3200, 100, 0, 0, 0x0015) | Out-Null }
if ($Do -eq "restore") { [W2]::ShowWindow($h, 9) | Out-Null; Start-Sleep -Milliseconds 500 }
"minimized=$([W2]::IsIconic($h)) maximized=$([W2]::IsZoomed($h))"
