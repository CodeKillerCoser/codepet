export interface ConnectionTraffic {
  channel: string;
  sentBytes: number;
  receivedBytes: number;
  sentBytesPerSecond: number;
  receivedBytesPerSecond: number;
}
export function channelLabel(channel: string): string {
  return ({ lan: "局域网", "ipv6-direct": "IPv6 直连", "ipv4-direct": "IPv4 直连", relay: "中继", direct: "点对点直连" })[channel] ?? "通道识别中";
}
export function formatBytes(bytes: number): string {
  const units = ["B", "KiB", "MiB", "GiB", "TiB"];
  let value = Math.max(0, bytes), unit = 0;
  while (value >= 1024 && unit < units.length - 1) { value /= 1024; unit++; }
  return `${value.toFixed(unit === 0 ? 0 : 1)} ${units[unit]}`;
}
