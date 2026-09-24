// Harmless stand-in for both installed executables; never starts a real agent.
using System;
using System.IO;
using System.Diagnostics;
using System.Globalization;
using System.Threading;

internal static class InstallerFixture
{
    private static int Main(string[] args)
    {
#if INSTALLER_NEW
        // Keep the installer payload observably different from the old fixture
        // so rollback tests prove restoration rather than compare equal files.
        if (args.Length == 1 && args[0] == "--fixture-new")
        {
            return 0;
        }
#endif
        if (args.Length == 2 && args[0] == "--hold")
        {
            File.WriteAllText(args[1], "ready");
            Thread.Sleep(Timeout.Infinite);
        }
        if (args.Length == 3 && args[0] == "--hold-agent-lock")
        {
            string stateDirectory = args[2];
            Directory.CreateDirectory(stateDirectory);
            using (var lockFile = new FileStream(Path.Combine(stateDirectory, "agent-instance.lock"),
                FileMode.OpenOrCreate, FileAccess.ReadWrite, FileShare.ReadWrite))
            {
                lockFile.Lock(0, 1);
                using (var process = Process.GetCurrentProcess())
                {
                    string executable = process.MainModule.FileName.Replace("\\", "\\\\");
                    string futureHome = Directory.GetParent(stateDirectory).FullName.Replace("\\", "\\\\");
                    File.WriteAllText(Path.Combine(stateDirectory, "agent-instance.json"),
                        string.Format(CultureInfo.InvariantCulture,
                            "{{\"pid\":{0},\"executable\":\"{1}\",\"futureHome\":\"{2}\",\"startTimeFiletime\":{3}}}",
                            process.Id, executable, futureHome, process.StartTime.ToFileTimeUtc()));
                }
                File.WriteAllText(args[1], "ready");
                Thread.Sleep(Timeout.Infinite);
            }
        }
        // Simulate a mixed pre-sandbox CLI (clap uses exit 2 for an unknown
        // --reset-windows-sandbox flag) and an arbitrary cleanup failure
        // without touching real sandbox state.
        if (File.Exists(Path.Combine(AppDomain.CurrentDomain.BaseDirectory, "unsupported-cleanup")))
        {
            return 2;
        }
        if (File.Exists(Path.Combine(AppDomain.CurrentDomain.BaseDirectory, "hang-cleanup")))
        {
            Thread.Sleep(Timeout.Infinite);
        }
        return File.Exists(Path.Combine(AppDomain.CurrentDomain.BaseDirectory, "fail-cleanup")) ? 1 : 0;
    }
}
