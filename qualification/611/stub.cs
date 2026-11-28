using System;
using System.IO;
using System.Threading;
class ClaudeStub {
  static void Main() {
    Console.ReadLine();
    var home = Environment.GetEnvironmentVariable("HOME");
    File.WriteAllText(Path.Combine(home, "active"), "");
    for (int i = 0; i < 600 && !File.Exists(Path.Combine(home, "finish")); i++) Thread.Sleep(100);
    Console.WriteLine("{\"type\":\"system\",\"subtype\":\"init\",\"session_id\":\"fixture-611\"}");
    Console.WriteLine("{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"fixture-done-611\"}]}}");
    Console.WriteLine("{\"type\":\"result\",\"subtype\":\"success\",\"result\":\"fixture-done-611\"}");
  }
}
