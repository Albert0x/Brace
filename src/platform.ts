export type PlatformInfo = {
  os: string;
  arch: string;
};

export const isMacPlatform = (platform: PlatformInfo) => platform.os === "macos";

const quotePosix = (value: string) => `'${value.split("'").join("'\\''")}'`;
const quotePowerShell = (value: string) => `'${value.split("'").join("''")}'`;
const quoteCmd = (value: string) => `"${value.split('"').join('""')}"`;

export function changeDirectoryCommand(shellType: string, path: string): string {
  if (shellType === "cmd") return `cd /d ${quoteCmd(path)}`;
  if (shellType === "powershell") {
    return `Set-Location -LiteralPath ${quotePowerShell(path)}`;
  }
  return `cd -- ${quotePosix(path)}`;
}

export function showFileCommand(shellType: string, path: string): string {
  if (shellType === "cmd") return `type ${quoteCmd(path)}`;
  if (shellType === "powershell") {
    return `Get-Content -LiteralPath ${quotePowerShell(path)}`;
  }
  return `cat -- ${quotePosix(path)}`;
}

export function platformLabel(platform: PlatformInfo): string {
  const os = platform.os === "macos" ? "macOS" : platform.os === "windows" ? "Windows" : platform.os;
  const arch = platform.arch === "aarch64" ? "Apple Silicon" : platform.arch;
  return `${os} · ${arch}`;
}
