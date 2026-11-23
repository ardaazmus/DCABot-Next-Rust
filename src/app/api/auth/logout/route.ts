import { NextResponse } from "next/server";
import { isSameOriginJsonRequest, SESSION_COOKIE_NAME } from "@/lib/auth-session";

export async function POST(request: Request) {
  if (!isSameOriginJsonRequest(request.headers, request.url)) {
    return NextResponse.json({ ok: false, error: "Same-origin JSON request required" }, { status: 403 });
  }

  const res = NextResponse.json({ ok: true, message: "Oturum kapatıldı" });
  res.cookies.set(SESSION_COOKIE_NAME, "", {
    httpOnly: true,
    secure: process.env.NODE_ENV === "production",
    sameSite: "strict",
    path: "/",
    maxAge: 0,
  });
  return res;
}
