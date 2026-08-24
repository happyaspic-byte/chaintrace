import type { Metadata } from "next";
import "./globals.css";

export const metadata: Metadata = {
  metadataBase: new URL("https://chaintrace.happyaspic.chatgpt.site"),
  title: "ChainTrace | 멀티체인 지갑 흐름 분석",
  description: "BTC, ETH, ETC, Solana, TRON, DOGE, LTC, XRP 지갑의 입출금 연결을 단계별로 추적하고 시각화하는 온체인 분석 도구입니다.",
  openGraph: {
    title: "ChainTrace | 멀티체인 지갑 흐름 분석",
    description: "14개 주요 네트워크의 지갑 입출금 연결을 단계별로 추적하고 시각화합니다.",
    type: "website",
    url: "https://chaintrace.happyaspic.chatgpt.site",
    images: [{ url: "https://chaintrace.happyaspic.chatgpt.site/og.png", width: 1200, height: 630, alt: "ChainTrace 멀티체인 지갑 흐름 분석" }],
  },
  twitter: {
    card: "summary_large_image",
    title: "ChainTrace | 멀티체인 지갑 흐름 분석",
    description: "14개 주요 네트워크의 지갑 입출금 연결을 단계별로 추적하고 시각화합니다.",
    images: ["https://chaintrace.happyaspic.chatgpt.site/og.png"],
  },
  icons: { icon: "/favicon.svg", shortcut: "/favicon.svg" },
};

export default function RootLayout({ children }: Readonly<{ children: React.ReactNode }>) {
  return <html lang="ko"><body>{children}</body></html>;
}
