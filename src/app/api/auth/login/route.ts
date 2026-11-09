import { NextRequest, NextResponse } from "next/server";
import { createSessionToken, verifyPassword, SESSION_COOKIE_NAME } from "@/lib/auth-session";

export async function POST(req: NextRequest) {
  try {
    const body = await req.json();
    const password = typeof body.password === "string" ? body.password : "";

    if (!verifyPassword(password)) {
      return NextResponse.json({ ok: false, error: "Geçersiz erişim parolası" }, { status: 401 });
    }

    const token = createSessionToken();
    const res = NextResponse.json({ ok: true, message: "Oturum açıldı" });
    res.cookies.set(SESSION_COOKIE_NAME, token, {
      httpOnly: true,
      secure: process.env.NODE_ENV === "production",
      sameSite: "strict",
      path: "/",
      maxAge: 7 * 24 * 60 * 60,
    });
    return res;
  } catch {
    return NextResponse.json({ ok: false, error: "İstek ayrıştırılamadı" }, { status: 400 });
  }
}
