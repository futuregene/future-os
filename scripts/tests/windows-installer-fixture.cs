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
        // Simulate a cleanup failure without touching real sandbox state.
        return File.Exists(Path.Combine(AppDomain.CurrentDomain.BaseDirectory, "fail-cleanup")) ? 1 : 0;
    }
}
