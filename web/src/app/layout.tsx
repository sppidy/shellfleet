import type { Metadata } from "next";
import { JetBrains_Mono, Inter } from "next/font/google";
import "./globals.css";

const jetbrainsMono = JetBrains_Mono({
  variable: "--font-mono-google",
  subsets: ["latin"],
  weight: ["300", "400", "500", "600", "700"],
});

const inter = Inter({
  variable: "--font-sans-google",
  subsets: ["latin"],
  weight: ["400", "500", "600", "700"],
});

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
      className={`${jetbrainsMono.variable} ${inter.variable} antialiased`}
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
