import { describe, expect, it } from "vitest";
import {
  changeDirectoryCommand,
  platformLabel,
  showFileCommand,
} from "./platform";

describe("platform shell commands", () => {
  it("quotes apostrophes safely for POSIX shells", () => {
    expect(changeDirectoryCommand("zsh", "/tmp/Mary's files")).toBe(
      "cd -- '/tmp/Mary'\\''s files'",
    );
  });

  it("uses literal paths for PowerShell", () => {
    expect(showFileCommand("powershell", "C:\\O'Brien\\notes.txt")).toBe(
      "Get-Content -LiteralPath 'C:\\O''Brien\\notes.txt'",
    );
  });

  it("does not evaluate command substitutions in POSIX paths", () => {
    expect(showFileCommand("bash", "/tmp/$(touch owned)")).toBe(
      "cat -- '/tmp/$(touch owned)'",
    );
  });

  it("formats Apple Silicon for users", () => {
    expect(platformLabel({ os: "macos", arch: "aarch64" })).toBe(
      "macOS · Apple Silicon",
    );
  });
});
