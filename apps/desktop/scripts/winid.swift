import CoreGraphics
import Foundation
let owner = CommandLine.arguments.count > 1 ? CommandLine.arguments[1] : ""
let list = CGWindowListCopyWindowInfo([.optionAll], kCGNullWindowID) as? [[String: Any]] ?? []
for w in list {
  let o = w[kCGWindowOwnerName as String] as? String ?? ""
  let pid = w[kCGWindowOwnerPID as String] as? Int ?? 0
  let layer = w[kCGWindowLayer as String] as? Int ?? -1
  let b = w[kCGWindowBounds as String] as? [String: Any] ?? [:]
  let wd = b["Width"] as? Double ?? 0
  if (o == owner || String(pid) == owner) && layer == 0 && wd > 100 {
    print(w[kCGWindowNumber as String] as? Int ?? 0, pid, o, Int(wd), Int(b["Height"] as? Double ?? 0), w[kCGWindowName as String] as? String ?? "")
  }
}
