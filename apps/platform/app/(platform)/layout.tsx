import { PlatformLayoutClient } from "./platform-layout-client";

export default function PlatformLayout({ children }: { children: React.ReactNode }) {
  return <PlatformLayoutClient>{children}</PlatformLayoutClient>;
}
