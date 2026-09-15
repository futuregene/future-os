// Harmless stand-in for both installed executables; never starts a real agent.
using System;
using System.IO;
using System.Threading;

internal static class InstallerFixture
{
    private static int Main(string[] args)
    {
        if (args.Length == 2 && args[0] == "--hold")
        {
            File.WriteAllText(args[1], "ready");
            Thread.Sleep(Timeout.Infinite);
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
