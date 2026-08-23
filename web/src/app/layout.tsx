import type { Metadata } from "next";
import "./globals.css";

// The dashboard is an authenticated control plane, not a static document.
// Rendering it dynamically makes Next serve the HTML with private/no-store
// semantics, so a browser or intermediary cannot pin an old client bundle
// after a deployment. Hashed files under /_next/static remain cacheable.
export const dynamic = "force-dynamic";

export const metadata: Metadata = {
  title: "ShellFleet",
  description: "Manage systemd services and Docker workloads across your fleet from one terminal-flavored dashboard.",
  icons: { icon: "/favicon.svg" },
};

import { WebSocketProvider } from "@/components/providers/WebSocketProvider";
import { SessionProvider } from "@/components/providers/SessionProvider";
import { CoreFleetProvider } from "@/components/providers/CoreFleetProvider";
import { UiProvider } from "@/components/providers/UiProvider";
import ViewerBanner from "@/components/ViewerBanner";

export default function RootLayout({
  children,
}: Readonly<{
  children: React.ReactNode;
}>) {
  return (
    <html
      lang="en"
      data-theme="dark"
      data-accent="green"
      data-density="dense"
      className="antialiased"
    >
      <body>
        <UiProvider>
          <SessionProvider>
            <CoreFleetProvider>
              <ViewerBanner />
              <WebSocketProvider>
                {children}
              </WebSocketProvider>
            </CoreFleetProvider>
          </SessionProvider>
        </UiProvider>
      </body>
    </html>
  );
}
